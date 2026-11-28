//! Helpers for the end-to-end tests: real relay and cloud processes,
//! simulated phones over real sockets, and HTTP calls.
//!
//! The tests themselves are in `tests/e2e.rs`. They need the packs built by
//! `scripts/packs.sh` in `target/packs`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use hostable_protocol::{HostToPhone, PhoneInput, PhoneToRelay, RelayToPhone, Widget};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio_tungstenite::tungstenite::Message;

pub type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// The repository root.
pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("repo root")
        .to_path_buf()
}

/// `target/packs`, which must hold the built packs.
pub fn packs_dir() -> PathBuf {
    let dir = root().join("target/packs");
    for id in [
        "buzzer",
        "wizards-duel",
        "tideholm",
        "riposte",
        "ts-tictactoe",
    ] {
        assert!(
            dir.join(format!("{id}.gamepack")).exists(),
            "{id}.gamepack is missing from {}: run scripts/packs.sh first",
            dir.display()
        );
    }
    dir
}

/// A binary of this package. Cargo builds `e2e-relay` and `e2e-cloud` next
/// to the test binary's `deps/` folder before integration tests run.
pub fn bin(name: &str) -> String {
    let exe = std::env::current_exe().expect("test binary path");
    let dir = exe.parent().and_then(Path::parent).expect("target dir");
    let path = dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    assert!(path.exists(), "{} wasn't built", path.display());
    path.display().to_string()
}

/// A fresh scratch directory for one test.
pub fn scratch(name: &str) -> PathBuf {
    let dir = root()
        .join("target/e2e-scratch")
        .join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A server process that is killed when dropped.
pub struct Server {
    pub child: Child,
    /// `host:port` it listens on.
    pub addr: String,
}

impl Server {
    /// Start `exe args...` and wait for it to print `listening on ADDR`.
    pub async fn start(exe: &str, args: &[String], envs: &[(&str, String)]) -> Server {
        let mut cmd = Command::new(exe);
        cmd.args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);
        for (k, v) in envs {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().unwrap_or_else(|e| panic!("start {exe}: {e}"));
        let stdout = child.stdout.take().unwrap();
        let mut lines = BufReader::new(stdout).lines();
        let addr = tokio::time::timeout(Duration::from_secs(30), async {
            while let Ok(Some(line)) = lines.next_line().await {
                if let Some(a) = line.split("listening on ").nth(1) {
                    return a.trim().to_string();
                }
            }
            panic!("{exe} exited before listening");
        })
        .await
        .expect("server didn't start");
        // Keep draining stdout so the child never blocks on a full pipe.
        tokio::spawn(async move { while let Ok(Some(_)) = lines.next_line().await {} });
        Server { child, addr }
    }

    /// Kill the process hard (like a crash or power loss).
    pub async fn kill(mut self) {
        let _ = self.child.kill().await;
        let _ = self.child.wait().await;
    }
}

pub async fn relay(state_file: Option<&Path>) -> Server {
    let mut args = vec!["--addr".to_string(), "127.0.0.1:0".to_string()];
    if let Some(f) = state_file {
        args.push("--state".into());
        args.push(f.display().to_string());
    }
    Server::start(&bin("e2e-relay"), &args, &[]).await
}

pub async fn relay_at(addr: &str, state_file: &Path) -> Server {
    let args = vec![
        "--addr".to_string(),
        addr.to_string(),
        "--state".to_string(),
        state_file.display().to_string(),
    ];
    Server::start(&bin("e2e-relay"), &args, &[]).await
}

pub async fn cloud(relay: &Server, db: &Path, envs: &[(&str, String)]) -> Server {
    cloud_at("127.0.0.1:0", relay, db, envs).await
}

pub async fn cloud_at(addr: &str, relay: &Server, db: &Path, envs: &[(&str, String)]) -> Server {
    let args = vec![
        "--addr".to_string(),
        addr.to_string(),
        "--relay".to_string(),
        format!("ws://{}", relay.addr),
        "--db".to_string(),
        db.display().to_string(),
        "--packs".to_string(),
        packs_dir().display().to_string(),
    ];
    let mut envs = envs.to_vec();
    envs.push(("HOSTABLE_ADMIN_TOKEN", "e2e-admin".to_string()));
    Server::start(&bin("e2e-cloud"), &args, &envs).await
}

// ---------------------------------------------------------------------------
// HTTP
// ---------------------------------------------------------------------------

pub async fn post(base: &str, path: &str, body: Value) -> (u16, Value) {
    let res = reqwest::Client::new()
        .post(format!("http://{base}{path}"))
        .json(&body)
        .send()
        .await
        .unwrap();
    let status = res.status().as_u16();
    (status, res.json().await.unwrap_or(Value::Null))
}

pub async fn get(base: &str, path: &str) -> Value {
    reqwest::get(format!("http://{base}{path}"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

// ---------------------------------------------------------------------------
// Phones
// ---------------------------------------------------------------------------

/// A simulated phone: a real socket to the relay, the latest screen, and a
/// record of everything the host sent it.
pub struct Phone {
    pub name: String,
    relay: String,
    pub code: String,
    pub ws: Option<Ws>,
    pub player: u32,
    pub token: String,
    pub seat: Option<u32>,
    pub ui: Option<Widget>,
    pub ui_version: u64,
    /// The screen hasn't changed since we last pressed something.
    pub stale: bool,
    /// Every screen this phone was sent, in order.
    pub screens: Vec<Widget>,
    pub tables: Vec<Value>,
    pub errors: Vec<String>,
}

impl Phone {
    async fn connect(relay: &str, first: &PhoneToRelay) -> Ws {
        let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{relay}/ws/phone"))
            .await
            .expect("phone connect");
        ws.send(Message::Text(serde_json::to_string(first).unwrap().into()))
            .await
            .unwrap();
        ws
    }

    pub async fn join(relay: &str, code: &str, name: &str) -> Phone {
        let ws = Self::connect(
            relay,
            &PhoneToRelay::Join {
                code: code.into(),
                name: name.into(),
                color: String::new(),
            },
        )
        .await;
        let mut p = Phone::new(relay, code, name, ws);
        while p.token.is_empty() {
            p.recv(Duration::from_secs(10)).await.expect("no joined");
        }
        p
    }

    /// Come in with a token (from an earlier join or a share link).
    pub async fn with_token(
        relay: &str,
        code: &str,
        name: &str,
        player: u32,
        token: &str,
    ) -> Phone {
        let ws = Self::connect(
            relay,
            &PhoneToRelay::Rejoin {
                code: code.into(),
                player_token: token.into(),
            },
        )
        .await;
        let mut p = Phone::new(relay, code, name, ws);
        p.token = token.into();
        p.player = player;
        p
    }

    fn new(relay: &str, code: &str, name: &str, ws: Ws) -> Phone {
        Phone {
            name: name.into(),
            relay: relay.into(),
            code: code.into(),
            ws: Some(ws),
            player: 0,
            token: String::new(),
            seat: None,
            ui: None,
            ui_version: 0,
            stale: false,
            screens: Vec::new(),
            tables: Vec::new(),
            errors: Vec::new(),
        }
    }

    /// Drop the connection (the phone loses signal).
    pub fn drop_connection(&mut self) {
        self.ws = None;
    }

    /// Reconnect with the stored token, like the web client does.
    pub async fn reconnect(&mut self) {
        let ws = Self::connect(
            &self.relay.clone(),
            &PhoneToRelay::Rejoin {
                code: self.code.clone(),
                player_token: self.token.clone(),
            },
        )
        .await;
        self.ws = Some(ws);
        self.ui = None;
    }

    /// Reconnect to a relay at a new address (after a relay restart).
    pub async fn reconnect_to(&mut self, relay: &str) {
        self.relay = relay.into();
        self.reconnect().await;
    }

    /// Wait up to `timeout` for one message. `None` if nothing came.
    pub async fn recv(&mut self, timeout: Duration) -> Option<RelayToPhone> {
        let ws = self.ws.as_mut()?;
        loop {
            let msg = match tokio::time::timeout(timeout, ws.next()).await {
                Ok(Some(Ok(m))) => m,
                Ok(_) => {
                    self.ws = None;
                    return None;
                }
                Err(_) => return None,
            };
            let Message::Text(text) = msg else { continue };
            let m: RelayToPhone = serde_json::from_str(&text).expect("relay sent bad json");
            match &m {
                RelayToPhone::Joined {
                    player,
                    player_token,
                } => {
                    self.player = *player;
                    self.token = player_token.clone();
                }
                RelayToPhone::Rejoined { player } => self.player = *player,
                RelayToPhone::FromHost { message } => match message {
                    HostToPhone::Ui { version, ui } => {
                        self.ui_version = *version;
                        self.ui = Some(ui.clone());
                        self.stale = false;
                        self.screens.push(ui.clone());
                    }
                    HostToPhone::Table { table, .. } => {
                        self.tables.push(serde_json::to_value(table).unwrap())
                    }
                    HostToPhone::Seat { seat, .. } => self.seat = *seat,
                    HostToPhone::Feedback { .. } => {}
                },
                RelayToPhone::Error { message } => self.errors.push(message.clone()),
                _ => {}
            }
            return Some(m);
        }
    }

    /// Read everything already waiting.
    pub async fn drain(&mut self) {
        while self.recv(Duration::from_millis(5)).await.is_some() {}
    }

    /// Wait until a screen satisfying `f` arrives.
    pub async fn wait_until(&mut self, timeout: Duration, f: impl Fn(&Widget) -> bool) -> bool {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if self.ui.as_ref().is_some_and(&f) {
                return true;
            }
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            if left.is_zero() {
                return false;
            }
            self.recv(left.min(Duration::from_millis(200))).await;
        }
    }

    pub async fn send(&mut self, input: PhoneInput) {
        self.stale = true;
        let msg = PhoneToRelay::Input(input);
        if let Some(ws) = self.ws.as_mut() {
            let _ = ws
                .send(Message::Text(serde_json::to_string(&msg).unwrap().into()))
                .await;
        }
    }
}

/// All the text in a screen, for simple assertions.
pub fn text_of(ui: &Widget) -> String {
    serde_json::to_string(ui).unwrap()
}

/// Strip the countdown the host puts above a waiting seat's screen.
pub fn without_timer(ui: &Widget) -> &Widget {
    if let Widget::Column { children } = ui {
        if children.len() == 2 && matches!(children[0], Widget::Timer { .. }) {
            return &children[1];
        }
    }
    ui
}

/// A policy: pick an input for a screen.
pub type Bot = fn(&Widget, &mut hostable_rules_api::Rng) -> Option<PhoneInput>;

/// Drive phones with a bot until the cloud says the game finished.
/// Returns the number of inputs sent.
pub async fn play_until_finished(
    cloud: &str,
    id: &str,
    phones: &mut [Phone],
    bot: Bot,
    seed: u64,
    limit: Duration,
) -> u64 {
    let mut rng = hostable_rules_api::Rng::new(seed);
    let started = tokio::time::Instant::now();
    let mut moves = 0u64;
    let mut idle_rounds = 0u32;
    loop {
        let mut acted = false;
        for p in phones.iter_mut() {
            p.drain().await;
            if p.stale {
                // Give the host a moment to answer the last press.
                p.recv(Duration::from_millis(20)).await;
            }
            let Some(ui) = p.ui.clone() else { continue };
            if let Some(input) = bot(&ui, &mut rng) {
                assert!(
                    ui.accepts(&input),
                    "bot sent an input the screen can't send: {input:?}"
                );
                p.send(input).await;
                moves += 1;
                acted = true;
            }
        }
        idle_rounds = if acted { 0 } else { idle_rounds + 1 };
        if moves.is_multiple_of(25) || idle_rounds > 0 {
            let game = get(cloud, &format!("/api/games/{id}")).await;
            if !game["finished"].is_null() {
                for p in phones.iter_mut() {
                    p.recv(Duration::from_millis(100)).await;
                    p.drain().await;
                }
                return moves;
            }
            assert!(game["paused"].is_null(), "game paused: {}", game["paused"]);
            if idle_rounds > 0 {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
        assert!(
            started.elapsed() < limit,
            "game {id} didn't finish within {limit:?} ({moves} moves)"
        );
    }
}

/// Phones join a cloud game by code, in order, and the game starts.
pub async fn join_and_start(
    relay: &str,
    cloud: &str,
    id: &str,
    code: &str,
    names: &[&str],
) -> Vec<Phone> {
    let mut phones = Vec::new();
    for name in names {
        let mut p = Phone::join(relay, code, name).await;
        assert!(
            p.wait_until(Duration::from_secs(10), |_| true).await,
            "{name}: no lobby"
        );
        phones.push(p);
    }
    let (status, body) = post(cloud, &format!("/api/games/{id}/start"), Value::Null).await;
    assert_eq!(status, 200, "start: {body}");
    phones
}

/// Count of each widget id across a phone's screens (debugging aid).
pub fn widget_ids(screens: &[Widget]) -> BTreeMap<String, usize> {
    let mut out = BTreeMap::new();
    for s in screens {
        for w in s.inputs() {
            if let Some(id) = w.input_id() {
                *out.entry(id.to_string()).or_default() += 1;
            }
        }
    }
    out
}
