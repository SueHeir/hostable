use super::*;
use hostable_pack::{generate_key, Pack};
use hostable_relay::server::{Config as RelayConfig, Relay};
use std::future::IntoFuture;

const JS_GAME: &str = r#"
function handle(req) {
  switch (req.call) {
    case "info": return { min_seats: 2, max_seats: 3, api: 1 };
    case "setup": return { state: { n: 0, turn: 0, seats: req.seats.length },
      waiting_on: [{ seat: 0, prompt: "", live_timeout_ms: 60000, async_timeout_ms: 86400000 }] };
    case "on_action": case "on_timeout": {
      const s = { n: req.state.n + 1, turn: (req.state.turn + 1) % req.state.seats, seats: req.state.seats };
      const out = { state: s, waiting_on: [{ seat: s.turn, prompt: "Your move", live_timeout_ms: 60000, async_timeout_ms: 86400000 }] };
      if (s.n >= 4) { out.finished = { ranking: [[0, 1]], summary: "done" }; out.waiting_on = []; }
      return out;
    }
    case "table_view": return { title: "Pass", status: "n=" + req.state.n, players: [], log: [] };
    case "phone_view": return req.state.turn === req.seat
      ? { type: "button", id: "go", label: "Go" } : { type: "text", value: "wait" };
    case "can_respond": return true;
  }
}
globalThis.__hostable_dispatch = (t) => JSON.stringify({ ok: handle(JSON.parse(t)) });
"#;

fn catalog() -> Catalog {
    let mut pack = Pack::default();
    pack.files.insert(
        "manifest.json".into(),
        serde_json::to_vec(&json!({"id": "pass", "name": "Pass", "version": "1.0.0",
            "players": {"min": 2, "max": 3}, "api": 1, "rules": "rules.js"}))
        .unwrap(),
    );
    pack.files
        .insert("rules.js".into(), JS_GAME.as_bytes().to_vec());
    pack.sign(&generate_key()).unwrap();
    let mut c = Catalog::default();
    c.add(&pack.write().unwrap()).unwrap();
    c
}

async fn start(db: &Path, recorder: push::Recorder) -> (String, Cloud) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let relay = format!("ws://{}", listener.local_addr().unwrap());
    tokio::spawn(Relay::new(RelayConfig::default()).serve(listener));
    let cloud = Cloud::new(
        Config {
            relay: relay.clone(),
            public_relay: format!("{relay}/ws/phone"),
            base: "/".into(),
            db: db.to_path_buf(),
            client_dir: None,
            admin_token: Some("secret".into()),
            limits: Limits::DEFAULT,
            quota: Quota::default(),
        },
        catalog(),
        Arc::new(recorder),
    )
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let http = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(axum::serve(listener, cloud.router()).into_future());
    (http, cloud)
}

fn tmp_db(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hostable-cloud-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("cloud.db")
}

async fn post(url: &str, body: Value) -> (u16, Value) {
    let res = reqwest::Client::new()
        .post(url)
        .body(body.to_string())
        .header("content-type", "application/json")
        .send()
        .await
        .unwrap();
    let status = res.status().as_u16();
    (status, res.json().await.unwrap_or(Value::Null))
}

#[tokio::test]
async fn games_are_created_seats_claimed_once_and_viewed() {
    let (http, _cloud) = start(&tmp_db("create"), push::Recorder::default()).await;
    let packs: Value = reqwest::get(format!("{http}/api/packs"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(packs[0]["id"], "pass");
    let (status, game) = post(
        &format!("{http}/api/games"),
        json!({"pack": "pass", "seats": ["Ana", "Bo"], "pace": "async"}),
    )
    .await;
    assert_eq!(status, 200, "{game}");
    assert_eq!(game["code"].as_str().unwrap().len(), 4);
    let id = game["id"].as_str().unwrap();
    let (status, claim) = post(&format!("{http}/api/games/{id}/claim"), json!({"seat": 0})).await;
    assert_eq!(status, 200, "{claim}");
    assert_eq!(claim["code"], game["code"]);
    assert!(claim["player_token"].as_str().unwrap().len() >= 32);
    let (status, _) = post(&format!("{http}/api/games/{id}/claim"), json!({"seat": 0})).await;
    assert_eq!(status, 409);
    let (status, _) = post(&format!("{http}/api/games/{id}/start"), json!({})).await;
    assert_eq!(status, 200);
    let view: Value = reqwest::get(format!("{http}/api/games/{id}"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(view["started"], true);
    assert_eq!(view["seats"][0]["name"], "Ana");
    assert_eq!(view["seats"][0]["claimed"], true);
    assert_eq!(view["seats"][1]["claimed"], false);
    assert_eq!(view["table"]["status"], "n=0");
    let config: Value = reqwest::get(format!("{http}/config.json"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(config["relay"].as_str().unwrap().ends_with("/ws/phone"));
    assert_eq!(config["vapid_public_key"], "test-key");
}

#[tokio::test]
async fn admins_can_take_a_pack_down() {
    let (http, _cloud) = start(&tmp_db("delist"), push::Recorder::default()).await;
    let (_, game) = post(
        &format!("{http}/api/games"),
        json!({"pack": "pass", "seats": ["A", "B"]}),
    )
    .await;
    let id = game["id"].as_str().unwrap().to_string();
    post(&format!("{http}/api/games/{id}/start"), json!({})).await;
    let (status, _) = post(
        &format!("{http}/api/reports"),
        json!({"pack": "pass", "reason": "rude"}),
    )
    .await;
    assert_eq!(status, 202);
    // Without the token: refused.
    let (status, _) = post(
        &format!("{http}/api/admin/delist"),
        json!({"pack": "pass", "reason": "x"}),
    )
    .await;
    assert_eq!(status, 401);
    let res = reqwest::Client::new()
        .post(format!("{http}/api/admin/delist"))
        .header("authorization", "Bearer secret")
        .header("content-type", "application/json")
        .body(json!({"pack": "pass", "reason": "reported"}).to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(res.status().as_u16(), 200);
    let packs: Value = reqwest::get(format!("{http}/api/packs"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(packs, json!([]));
    let (status, _) = post(&format!("{http}/api/games"), json!({"pack": "pass"})).await;
    assert_eq!(status, 410);
    let mut paused = Value::Null;
    for _ in 0..50 {
        let view: Value = reqwest::get(format!("{http}/api/games/{id}"))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        paused = view["paused"].clone();
        if !paused.is_null() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(paused.as_str().unwrap().contains("taken down"));
    let reports = reqwest::Client::new()
        .get(format!("{http}/api/admin/reports"))
        .header("authorization", "Bearer secret")
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(reports[0]["reason"], "rude");
}

#[tokio::test]
async fn async_turns_send_web_push_to_subscribed_players() {
    let recorder = push::Recorder::default();
    let (http, cloud) = start(&tmp_db("push"), recorder.clone()).await;
    let (_, game) = post(
        &format!("{http}/api/games"),
        json!({"pack": "pass", "seats": ["A", "B"], "pace": "async"}),
    )
    .await;
    let id = game["id"].as_str().unwrap().to_string();
    let sub = json!({"endpoint": "https://push.example/1", "keys": {"p256dh": "k", "auth": "a"}});
    cloud
        .handle_effects(
            &id,
            "Pass",
            Pace::Async,
            vec![Effect::PushSubscribed {
                player: 7,
                subscription: sub,
            }],
        )
        .await;
    // A wait on a player without a subscription sends nothing.
    cloud
        .handle_effects(
            &id,
            "Pass",
            Pace::Async,
            vec![Effect::Notify {
                player: 8,
                seat: 0,
                prompt: String::new(),
            }],
        )
        .await;
    // Live games don't push.
    cloud
        .handle_effects(
            &id,
            "Pass",
            Pace::Live,
            vec![Effect::Notify {
                player: 7,
                seat: 1,
                prompt: "x".into(),
            }],
        )
        .await;
    cloud
        .handle_effects(
            &id,
            "Pass",
            Pace::Async,
            vec![Effect::Notify {
                player: 7,
                seat: 1,
                prompt: "Your move".into(),
            }],
        )
        .await;
    let sent = recorder.sent.lock().unwrap().clone();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0, "https://push.example/1");
    assert_eq!(sent[0].1["body"], "Your move");
    assert_eq!(sent[0].1["title"], "Pass");
    assert_eq!(sent[0].1["url"], format!("/g/{id}"));
}

#[tokio::test]
async fn every_change_is_saved_to_the_database() {
    let db = tmp_db("saves");
    let (http, cloud) = start(&db, push::Recorder::default()).await;
    let (_, game) = post(
        &format!("{http}/api/games"),
        json!({"pack": "pass", "seats": ["A", "B"]}),
    )
    .await;
    let id = game["id"].as_str().unwrap().to_string();
    post(&format!("{http}/api/games/{id}/start"), json!({})).await;
    let snap: String = Connection::open(&db)
        .unwrap()
        .query_row("SELECT snapshot FROM games WHERE id = ?1", [&id], |r| {
            r.get(0)
        })
        .unwrap();
    let snap: Value = serde_json::from_str(&snap).unwrap();
    assert_eq!(snap["state"]["n"], 0);
    assert_eq!(snap["waits"][0]["wait"]["seat"], 0);
    let _ = cloud;
}
