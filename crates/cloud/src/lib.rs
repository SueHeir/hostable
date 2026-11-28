//! The Hostable cloud host: the same host core as the desktop app, headless,
//! in an `axum` server with SQLite.
//!
//! - Games are created over HTTP and get a share link (`/g/<id>`); players
//!   claim seats from the link or join a live room by code.
//! - Every game runs in its own worker thread with the sandbox's fuel and
//!   memory limits plus a CPU quota ([`hostable_host_runtime::Metered`]).
//! - State, seats and the action log are written to the database after
//!   every action, and games come back after a restart by reclaiming their
//!   relay rooms with their host tokens.
//! - Async games send Web Push when a seat is waited on.
//! - A desktop game can be handed over mid-game ([`handoff`]).
//! - An admin can delist a pack, which pauses its games.
//!
//! The HTTP API is documented in `docs/api.md`.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::{Path as UrlPath, State};
use axum::http::{header, HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use hostable_host_core::{check_table, Effect, Pace, Room};
use hostable_host_runtime::{
    run, Catalog, Command, Metered, Options, Quota, SaveTo, Saved, Update,
};
use hostable_rules_api::DynRules;
use hostable_sandbox::Limits;
use rand::Rng as _;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot, watch};

pub mod cli;
pub mod push;

use push::PushSender;

#[derive(Clone)]
pub struct Config {
    /// The relay as this server reaches it, e.g. `ws://127.0.0.1:8787`.
    pub relay: String,
    /// The phone socket URL browsers use (`wss://.../play/ws/phone`).
    pub public_relay: String,
    /// The path the site is served under, ending in `/`.
    pub base: String,
    pub db: PathBuf,
    pub client_dir: Option<PathBuf>,
    pub admin_token: Option<String>,
    pub limits: Limits,
    pub quota: Quota,
}

struct LiveGame {
    cmds: mpsc::UnboundedSender<Command>,
    pack: String,
    name: String,
    pace: Pace,
    code: watch::Receiver<String>,
}

#[derive(Clone)]
pub struct Cloud {
    db: Arc<Mutex<Connection>>,
    catalog: Arc<Catalog>,
    games: Arc<Mutex<HashMap<String, LiveGame>>>,
    config: Arc<Config>,
    push: Arc<dyn PushSender>,
}

type ApiResult = Result<Response, (StatusCode, String)>;

fn err(status: StatusCode, message: impl Into<String>) -> (StatusCode, String) {
    (status, message.into())
}

fn ok(v: Value) -> ApiResult {
    Ok(Json(v).into_response())
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS games (
    id TEXT PRIMARY KEY,
    pack TEXT NOT NULL,
    name TEXT NOT NULL,
    pace TEXT NOT NULL,
    turn_timer_ms INTEGER,
    seed INTEGER NOT NULL,
    code TEXT,
    host_token TEXT,
    snapshot TEXT,
    finished INTEGER NOT NULL DEFAULT 0,
    created INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS seats (
    game TEXT NOT NULL,
    seat INTEGER NOT NULL,
    name TEXT NOT NULL,
    player INTEGER,
    token TEXT,
    claimed INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (game, seat)
);
CREATE TABLE IF NOT EXISTS push_subscriptions (
    game TEXT NOT NULL,
    player INTEGER NOT NULL,
    subscription TEXT NOT NULL,
    PRIMARY KEY (game, player)
);
CREATE TABLE IF NOT EXISTS delisted (
    pack TEXT PRIMARY KEY,
    reason TEXT NOT NULL,
    at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS reports (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    pack TEXT NOT NULL,
    reason TEXT NOT NULL,
    at INTEGER NOT NULL
);
";

/// id, pack, name, pace, code, host token, snapshot, turn timer.
type SavedRow = (
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    Option<i64>,
);

fn now_ms() -> i64 {
    hostable_host_runtime::now_ms() as i64
}

fn new_id() -> String {
    rand::thread_rng()
        .sample_iter(&rand::distributions::Alphanumeric)
        .take(12)
        .map(char::from)
        .collect()
}

impl Cloud {
    /// Open the database and bring back every game that was running.
    pub fn new(
        config: Config,
        catalog: Catalog,
        push: Arc<dyn PushSender>,
    ) -> Result<Self, String> {
        let conn = Connection::open(&config.db).map_err(|e| e.to_string())?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")
            .map_err(|e| e.to_string())?;
        conn.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
        let cloud = Self {
            db: Arc::new(Mutex::new(conn)),
            catalog: Arc::new(catalog),
            games: Arc::new(Mutex::new(HashMap::new())),
            config: Arc::new(config),
            push,
        };
        Ok(cloud)
    }

    /// Restart every unfinished game from its last save. Call once inside
    /// the tokio runtime.
    pub fn resume_games(&self) -> Vec<String> {
        let rows: Vec<SavedRow> = {
            let db = self.db.lock().unwrap();
            let mut stmt = db
                .prepare(
                    "SELECT id, pack, name, pace, code, host_token, snapshot, turn_timer_ms FROM games
                     WHERE code IS NOT NULL AND snapshot IS NOT NULL",
                )
                .expect("query");
            stmt.query_map([], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                ))
            })
            .expect("rows")
            .filter_map(Result::ok)
            .collect()
        };
        let mut resumed = Vec::new();
        for (id, pack, name, pace, code, host_token, snapshot, timer) in rows {
            let Ok(snapshot) = serde_json::from_str::<Value>(&snapshot) else {
                continue;
            };
            let saved = Saved {
                pack: pack.clone(),
                code,
                host_token,
                snapshot,
            };
            let Ok(rules) = self.rules_for(&pack) else {
                continue;
            };
            let Ok(room) = saved.restore(rules) else {
                continue;
            };
            let room = room
                .with_name(name.clone())
                .with_turn_timer(timer.map(|t| t as u64));
            let pace = parse_pace(&pace);
            self.spawn(
                id.clone(),
                pack,
                name,
                pace,
                room,
                Some((saved.code, saved.host_token)),
                Vec::new(),
            );
            if let Some(reason) = self.delisted(&saved.pack) {
                self.command(&id, Command::Suspend(reason));
            }
            resumed.push(id);
        }
        resumed
    }

    fn rules_for(&self, pack: &str) -> Result<Box<DynRules>, String> {
        let entry = self
            .catalog
            .get(pack)
            .ok_or_else(|| format!("no pack {pack}"))?;
        let rules = entry.rules(self.config.limits)?;
        Ok(Box::new(Metered::new(rules, self.config.quota)))
    }

    fn delisted(&self, pack: &str) -> Option<String> {
        self.db
            .lock()
            .unwrap()
            .query_row("SELECT reason FROM delisted WHERE pack = ?1", [pack], |r| {
                r.get(0)
            })
            .optional()
            .ok()
            .flatten()
    }

    fn command(&self, id: &str, cmd: Command) -> bool {
        self.games
            .lock()
            .unwrap()
            .get(id)
            .is_some_and(|g| g.cmds.send(cmd).is_ok())
    }

    /// Run a game in its own worker thread, and follow its updates.
    #[allow(clippy::too_many_arguments)]
    fn spawn(
        &self,
        id: String,
        pack: String,
        name: String,
        pace: Pace,
        room: hostable_host_runtime::GameRoom,
        reclaim: Option<(String, String)>,
        new_seats: Vec<String>,
    ) {
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let (upd_tx, mut upd_rx) = mpsc::unbounded_channel();
        let (code_tx, code_rx) =
            watch::channel(reclaim.as_ref().map(|r| r.0.clone()).unwrap_or_default());
        for seat in new_seats {
            let _ = cmd_tx.send(Command::AddPlayer(seat));
        }
        let db = self.db.clone();
        let game_id = id.clone();
        let options = Options {
            relay: self.config.relay.clone(),
            pack: pack.clone(),
            save_to: SaveTo::Callback(Arc::new(move |saved: &Saved| {
                let finished = saved.snapshot.get("finished").is_some_and(|f| !f.is_null());
                let _ = db.lock().unwrap().execute(
                    "UPDATE games SET code = ?2, host_token = ?3, snapshot = ?4, finished = ?5 WHERE id = ?1",
                    params![
                        game_id,
                        saved.code,
                        saved.host_token,
                        saved.snapshot.to_string(),
                        finished as i64
                    ],
                );
            })),
            max_reconnects: None,
        };
        std::thread::Builder::new()
            .name(format!("game-{id}"))
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("game runtime");
                if let Err(e) = rt.block_on(run(room, reclaim, options, cmd_rx, upd_tx)) {
                    eprintln!("hostable-cloud: game stopped: {e}");
                }
            })
            .expect("spawn game thread");
        self.games.lock().unwrap().insert(
            id.clone(),
            LiveGame {
                cmds: cmd_tx,
                pack,
                name: name.clone(),
                pace,
                code: code_rx,
            },
        );
        let cloud = self.clone();
        tokio::spawn(async move {
            let mut seats_added = cloud.seat_count(&id);
            while let Some(update) = upd_rx.recv().await {
                match update {
                    Update::Opened { code, .. } => {
                        let _ = code_tx.send(code);
                    }
                    Update::PlayerAdded {
                        player,
                        name,
                        player_token,
                    } => {
                        let _ = cloud.db.lock().unwrap().execute(
                            "UPDATE seats SET player = ?3, token = ?4 WHERE game = ?1 AND seat = ?2 AND name = ?5",
                            params![id, seats_added, player, player_token, name],
                        );
                        seats_added += 1;
                    }
                    Update::Effects(effects) => {
                        cloud.handle_effects(&id, &name, pace, effects).await
                    }
                    Update::Closed => break,
                    _ => {}
                }
            }
            cloud.games.lock().unwrap().remove(&id);
        });
    }

    fn seat_count(&self, id: &str) -> i64 {
        self.db
            .lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM seats WHERE game = ?1 AND token IS NOT NULL",
                [id],
                |r| r.get(0),
            )
            .unwrap_or(0)
    }

    async fn handle_effects(&self, id: &str, name: &str, pace: Pace, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::PushSubscribed {
                    player,
                    subscription,
                } => {
                    let _ = self.db.lock().unwrap().execute(
                        "INSERT OR REPLACE INTO push_subscriptions (game, player, subscription) VALUES (?1, ?2, ?3)",
                        params![id, player, subscription.to_string()],
                    );
                }
                Effect::Notify { player, prompt, .. } if pace == Pace::Async => {
                    let sub: Option<String> = self
                        .db
                        .lock()
                        .unwrap()
                        .query_row(
                            "SELECT subscription FROM push_subscriptions WHERE game = ?1 AND player = ?2",
                            params![id, player],
                            |r| r.get(0),
                        )
                        .optional()
                        .ok()
                        .flatten();
                    let Some(sub) = sub.and_then(|s| serde_json::from_str::<Value>(&s).ok()) else {
                        continue;
                    };
                    let body = if prompt.is_empty() {
                        "It's your turn.".to_string()
                    } else {
                        prompt
                    };
                    let payload = json!({
                        "title": name,
                        "body": body,
                        "url": format!("{}g/{id}", self.config.base),
                    });
                    if let Err(e) = self.push.send(&sub, payload).await {
                        eprintln!("hostable-cloud: push failed: {e}");
                    }
                }
                _ => {}
            }
        }
    }

    async fn inspect<T: Send + 'static>(
        &self,
        id: &str,
        f: impl FnOnce(&hostable_host_runtime::GameRoom) -> T + Send + 'static,
    ) -> Option<T> {
        let (tx, rx) = oneshot::channel();
        let sent = self.command(
            id,
            Command::Inspect(Box::new(move |room| {
                let _ = tx.send(f(room));
            })),
        );
        if !sent {
            return None;
        }
        tokio::time::timeout(Duration::from_secs(5), rx)
            .await
            .ok()?
            .ok()
    }

    async fn code_of(&self, id: &str) -> Option<String> {
        let mut rx = self.games.lock().unwrap().get(id)?.code.clone();
        let wait = async {
            loop {
                let code = rx.borrow().clone();
                if !code.is_empty() {
                    return Some(code);
                }
                rx.changed().await.ok()?;
            }
        };
        tokio::time::timeout(Duration::from_secs(10), wait)
            .await
            .ok()
            .flatten()
    }

    pub fn router(&self) -> Router {
        Router::new()
            .route("/config.json", get(config_json))
            .route("/healthz", get(|| async { "ok" }))
            .route("/api/packs", get(list_packs))
            .route("/api/games", post(create_game))
            .route("/api/games/{id}", get(get_game))
            .route("/api/games/{id}/claim", post(claim_seat))
            .route("/api/games/{id}/start", post(start_game))
            .route("/api/handoff", post(handoff))
            .route("/api/reports", post(report_pack))
            .route("/api/admin/reports", get(list_reports))
            .route("/api/admin/delist", post(delist_pack))
            .fallback(static_file)
            .with_state(self.clone())
    }
}

fn parse_pace(s: &str) -> Pace {
    if s == "async" {
        Pace::Async
    } else {
        Pace::Live
    }
}

fn pace_name(p: Pace) -> &'static str {
    match p {
        Pace::Live => "live",
        Pace::Async => "async",
    }
}

async fn config_json(State(c): State<Cloud>) -> Json<Value> {
    Json(json!({
        "relay": c.config.public_relay,
        "api": format!("{}api", c.config.base),
        "vapid_public_key": c.push.public_key(),
    }))
}

async fn list_packs(State(c): State<Cloud>) -> Json<Value> {
    let delisted: Vec<String> = {
        let db = c.db.lock().unwrap();
        let mut stmt = db.prepare("SELECT pack FROM delisted").expect("query");
        stmt.query_map([], |r| r.get(0))
            .expect("rows")
            .filter_map(Result::ok)
            .collect()
    };
    Json(Value::Array(
        c.catalog
            .packs
            .values()
            .filter(|p| !delisted.contains(&p.manifest.id))
            .map(|p| {
                json!({
                    "id": p.manifest.id,
                    "name": p.manifest.name,
                    "min_seats": p.manifest.players.min,
                    "max_seats": p.manifest.players.max,
                    "description": p.manifest.description,
                })
            })
            .collect(),
    ))
}

#[derive(Deserialize)]
struct NewGame {
    pack: String,
    #[serde(default)]
    seats: Option<Vec<String>>,
    #[serde(default)]
    pace: Option<String>,
    #[serde(default)]
    turn_timer_ms: Option<u64>,
}

async fn create_game(State(c): State<Cloud>, Json(req): Json<NewGame>) -> ApiResult {
    let entry = c
        .catalog
        .get(&req.pack)
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "no such pack"))?
        .clone();
    if let Some(reason) = c.delisted(&req.pack) {
        return Err(err(
            StatusCode::GONE,
            format!("this pack was taken down: {reason}"),
        ));
    }
    let seats = req.seats.unwrap_or_default();
    if seats.len() as u32 > entry.manifest.players.max {
        return Err(err(StatusCode::BAD_REQUEST, "too many seats for this game"));
    }
    if seats
        .iter()
        .any(|s| s.trim().is_empty() || s.chars().count() > 24)
    {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "seat names must be 1-24 characters",
        ));
    }
    if req.turn_timer_ms.is_some_and(|t| t < 500) {
        return Err(err(StatusCode::BAD_REQUEST, "turn timer too short"));
    }
    let pace = parse_pace(req.pace.as_deref().unwrap_or("live"));
    let id = new_id();
    let seed: u64 = rand::thread_rng().gen();
    let name = entry.manifest.name.clone();
    {
        let db = c.db.lock().unwrap();
        db.execute(
            "INSERT INTO games (id, pack, name, pace, turn_timer_ms, seed, created) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![id, req.pack, name, pace_name(pace), req.turn_timer_ms.map(|t| t as i64), seed as i64, now_ms()],
        )
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        for (i, seat) in seats.iter().enumerate() {
            db.execute(
                "INSERT INTO seats (game, seat, name) VALUES (?1, ?2, ?3)",
                params![id, i as i64, seat.trim()],
            )
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        }
    }
    let rules = c
        .rules_for(&req.pack)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let room = Room::new(rules, seed)
        .with_name(name.clone())
        .with_pace(pace)
        .with_turn_timer(req.turn_timer_ms);
    let seat_names = seats.iter().map(|s| s.trim().to_string()).collect();
    c.spawn(id.clone(), req.pack, name, pace, room, None, seat_names);
    let code = c
        .code_of(&id)
        .await
        .ok_or_else(|| err(StatusCode::BAD_GATEWAY, "the relay didn't open a room"))?;
    ok(json!({ "id": id, "code": code, "url": format!("{}g/{id}", c.config.base) }))
}

async fn get_game(State(c): State<Cloud>, UrlPath(id): UrlPath<String>) -> ApiResult {
    let (pack, name, pace) = {
        let games = c.games.lock().unwrap();
        let g = games
            .get(&id)
            .ok_or_else(|| err(StatusCode::NOT_FOUND, "no such game"))?;
        (g.pack.clone(), g.name.clone(), g.pace)
    };
    let code = c.code_of(&id).await.unwrap_or_default();
    let claimed: BTreeMap<i64, bool> = {
        let db = c.db.lock().unwrap();
        let mut stmt = db
            .prepare("SELECT seat, claimed FROM seats WHERE game = ?1")
            .expect("query");
        stmt.query_map([&id], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)? != 0))
        })
        .expect("rows")
        .filter_map(Result::ok)
        .collect()
    };
    let view = c
        .inspect(&id, move |room| {
            let mut seats: Vec<Value> = room
                .players()
                .into_iter()
                .filter_map(|(_, seat, name, connected)| {
                    seat.map(|s| json!({"seat": s, "name": name, "connected": connected}))
                })
                .collect();
            seats.sort_by_key(|s| s["seat"].as_u64());
            let table = room
                .table_view()
                .and_then(Result::ok)
                .and_then(|t| check_table(&t).ok())
                .map(|t| serde_json::to_value(t).unwrap_or(Value::Null))
                .unwrap_or(Value::Null);
            json!({
                "started": room.is_started(),
                "finished": room.finished(),
                "paused": room.paused(),
                "seats": seats,
                "table": table,
            })
        })
        .await
        .ok_or_else(|| err(StatusCode::SERVICE_UNAVAILABLE, "the game isn't responding"))?;
    let mut seats = view["seats"].as_array().cloned().unwrap_or_default();
    for s in &mut seats {
        let n = s["seat"].as_i64().unwrap_or(-1);
        s["claimed"] = json!(claimed.get(&n).copied().unwrap_or(true));
    }
    ok(json!({
        "id": id, "pack": pack, "name": name, "code": code, "pace": pace_name(pace),
        "started": view["started"], "finished": view["finished"], "paused": view["paused"],
        "seats": seats, "table": view["table"],
    }))
}

#[derive(Deserialize)]
struct Claim {
    seat: i64,
}

async fn claim_seat(
    State(c): State<Cloud>,
    UrlPath(id): UrlPath<String>,
    Json(req): Json<Claim>,
) -> ApiResult {
    let code = c
        .code_of(&id)
        .await
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "no such game"))?;
    // The seat's token arrives from the relay shortly after the game opens.
    for _ in 0..60 {
        let row: Option<(Option<i64>, Option<String>, i64)> =
            c.db.lock()
                .unwrap()
                .query_row(
                    "SELECT player, token, claimed FROM seats WHERE game = ?1 AND seat = ?2",
                    params![id, req.seat],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()
                .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        match row {
            None => return Err(err(StatusCode::NOT_FOUND, "no such seat")),
            Some((_, _, 1)) => return Err(err(StatusCode::CONFLICT, "that seat is taken")),
            Some((Some(player), Some(token), _)) => {
                let changed = c
                    .db
                    .lock()
                    .unwrap()
                    .execute(
                        "UPDATE seats SET claimed = 1 WHERE game = ?1 AND seat = ?2 AND claimed = 0",
                        params![id, req.seat],
                    )
                    .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
                if changed == 0 {
                    return Err(err(StatusCode::CONFLICT, "that seat is taken"));
                }
                return ok(json!({ "code": code, "player": player, "player_token": token }));
            }
            Some(_) => tokio::time::sleep(Duration::from_millis(50)).await,
        }
    }
    Err(err(
        StatusCode::SERVICE_UNAVAILABLE,
        "the seat isn't ready yet",
    ))
}

async fn start_game(State(c): State<Cloud>, UrlPath(id): UrlPath<String>) -> ApiResult {
    let started = c
        .inspect(&id, |room| room.is_started())
        .await
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "no such game"))?;
    if started {
        return Err(err(StatusCode::CONFLICT, "already started"));
    }
    c.command(&id, Command::Start);
    for _ in 0..40 {
        if c.inspect(&id, |room| room.is_started()).await == Some(true) {
            return ok(json!({ "started": true }));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Err(err(
        StatusCode::CONFLICT,
        "the game couldn't start (not enough players?)",
    ))
}

#[derive(Deserialize)]
struct Handoff {
    pack: String,
    code: String,
    host_token: String,
    snapshot: Value,
    #[serde(default)]
    pace: Option<String>,
}

/// Take over a live desktop game: same room code, same seat tokens.
async fn handoff(State(c): State<Cloud>, Json(req): Json<Handoff>) -> ApiResult {
    let entry = c
        .catalog
        .get(&req.pack)
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "no such pack"))?;
    let name = entry.manifest.name.clone();
    let saved = Saved {
        pack: req.pack.clone(),
        code: req.code.clone(),
        host_token: req.host_token.clone(),
        snapshot: req.snapshot,
    };
    let rules = c
        .rules_for(&req.pack)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let pace = parse_pace(req.pace.as_deref().unwrap_or("async"));
    let room = saved
        .restore(rules)
        .map_err(|e| err(StatusCode::BAD_REQUEST, format!("bad snapshot: {e}")))?;
    let timer = room.turn_timer();
    let room = room.with_name(name.clone()).with_pace(pace);
    let id = new_id();
    c.db.lock()
        .unwrap()
        .execute(
            "INSERT INTO games (id, pack, name, pace, turn_timer_ms, seed, code, host_token, snapshot, created)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                id, req.pack, name, pace_name(pace), timer.map(|t| t as i64), room.seed() as i64,
                saved.code, saved.host_token, saved.snapshot.to_string(), now_ms()
            ],
        )
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    c.spawn(
        id.clone(),
        req.pack,
        name,
        pace,
        room,
        Some((saved.code, saved.host_token)),
        Vec::new(),
    );
    ok(json!({ "id": id, "url": format!("{}g/{id}", c.config.base) }))
}

#[derive(Deserialize)]
struct Report {
    pack: String,
    reason: String,
}

async fn report_pack(State(c): State<Cloud>, Json(req): Json<Report>) -> ApiResult {
    let reason: String = req.reason.chars().take(2000).collect();
    c.db.lock()
        .unwrap()
        .execute(
            "INSERT INTO reports (pack, reason, at) VALUES (?1, ?2, ?3)",
            params![req.pack, reason, now_ms()],
        )
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok((StatusCode::ACCEPTED, Json(json!({"received": true}))).into_response())
}

fn require_admin(c: &Cloud, headers: &HeaderMap) -> Result<(), (StatusCode, String)> {
    let given = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    match (&c.config.admin_token, given) {
        (Some(want), Some(got)) if !want.is_empty() && constant_time_eq(want, got) => Ok(()),
        _ => Err(err(StatusCode::UNAUTHORIZED, "admin token required")),
    }
}

fn constant_time_eq(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

async fn list_reports(State(c): State<Cloud>, headers: HeaderMap) -> ApiResult {
    require_admin(&c, &headers)?;
    let rows: Vec<Value> = {
        let db = c.db.lock().unwrap();
        let mut stmt = db
            .prepare("SELECT pack, reason, at FROM reports ORDER BY id")
            .expect("query");
        stmt.query_map([], |r| {
            Ok(json!({"pack": r.get::<_, String>(0)?, "reason": r.get::<_, String>(1)?, "at": r.get::<_, i64>(2)?}))
        })
        .expect("rows")
        .filter_map(Result::ok)
        .collect()
    };
    ok(Value::Array(rows))
}

async fn delist_pack(
    State(c): State<Cloud>,
    headers: HeaderMap,
    Json(req): Json<Report>,
) -> ApiResult {
    require_admin(&c, &headers)?;
    let reason: String = req.reason.chars().take(500).collect();
    c.db.lock()
        .unwrap()
        .execute(
            "INSERT OR REPLACE INTO delisted (pack, reason, at) VALUES (?1, ?2, ?3)",
            params![req.pack, reason, now_ms()],
        )
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let ids: Vec<String> = c
        .games
        .lock()
        .unwrap()
        .iter()
        .filter(|(_, g)| g.pack == req.pack)
        .map(|(id, _)| id.clone())
        .collect();
    for id in &ids {
        c.command(
            id,
            Command::Suspend(format!("This game was taken down: {reason}")),
        );
    }
    ok(json!({ "delisted": req.pack, "paused_games": ids.len() }))
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "webmanifest" => "application/manifest+json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// The web client, with `index.html` for client-side routes.
async fn static_file(State(c): State<Cloud>, uri: Uri) -> Response {
    let Some(dir) = &c.config.client_dir else {
        return (StatusCode::NOT_FOUND, "no client").into_response();
    };
    let path = uri.path().trim_start_matches('/');
    let safe = !path
        .split('/')
        .any(|seg| seg == ".." || seg.starts_with('.'));
    let candidate = dir.join(path);
    let file = if safe && !path.is_empty() && candidate.is_file() {
        candidate
    } else if path.starts_with("api/") || path.starts_with("assets/") {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    } else {
        dir.join("index.html")
    };
    match tokio::fs::read(&file).await {
        Ok(bytes) => {
            let cache = if file.ends_with("index.html") || file.ends_with("sw.js") {
                "no-cache"
            } else {
                "public, max-age=3600"
            };
            (
                [
                    (header::CONTENT_TYPE, content_type(&file)),
                    (header::CACHE_CONTROL, cache),
                    (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
                ],
                bytes,
            )
                .into_response()
        }
        Err(_) => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

#[cfg(test)]
mod tests;
