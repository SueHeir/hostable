//! End-to-end: real `e2e-relay` and `e2e-cloud` processes (the same code as
//! `hostable-relay` and `hostable-cloud`), real WebSockets and HTTP, the real
//! wasm packs from `target/packs`, and simulated phones driven by each
//! game's bot.
//!
//! These are `#[ignore]`d so `cargo test --workspace` stays quick; run them
//! with `cargo test -p hostable-e2e -- --include-ignored` after
//! `scripts/packs.sh`.

use std::future::IntoFuture;
use std::time::Duration;

use base64::Engine as _;
use hostable_e2e::*;
use hostable_host_core::bot::random_input;
use hostable_host_core::Pace;
use hostable_host_runtime::{hand_off, Catalog, Command, Options, SaveTo, Update};
use hostable_protocol::{PhoneInput, Widget, INPUT_PUSH};
use hostable_rules_api::{DynRules, Json, Rng, Rules, Seat};
use serde_json::{json, Value};
use tokio::sync::mpsc;

const GAME_LIMIT: Duration = Duration::from_secs(240);

fn buzzer_bot(ui: &Widget, rng: &mut Rng) -> Option<PhoneInput> {
    random_input(ui, rng, &[("buzz", 100), ("next_round", 100)])
}

async fn create(cloud: &str, body: Value) -> (String, String) {
    let (status, game) = post(cloud, "/api/games", body).await;
    assert_eq!(status, 200, "create: {game}");
    (
        game["id"].as_str().unwrap().to_string(),
        game["code"].as_str().unwrap().to_string(),
    )
}

fn db_snapshot(db: &std::path::Path, id: &str) -> Value {
    let conn = rusqlite::Connection::open(db).unwrap();
    let text: String = conn
        .query_row("SELECT snapshot FROM games WHERE id = ?1", [id], |r| {
            r.get(0)
        })
        .unwrap();
    serde_json::from_str(&text).unwrap()
}

#[tokio::test]
#[ignore = "end-to-end: run with --include-ignored after scripts/packs.sh"]
async fn buzzer_plays_to_a_winner_with_a_reconnect() {
    let dir = scratch("buzzer");
    let relay = relay(None).await;
    let cloud = cloud(&relay, &dir.join("cloud.db"), &[]).await;
    let (id, code) = create(&cloud.addr, json!({"pack": "buzzer", "pace": "live"})).await;
    let mut phones =
        join_and_start(&relay.addr, &cloud.addr, &id, &code, &["Ana", "Bo", "Cy"]).await;
    let seat_before = phones[1].seat;
    // A few presses, then Bo's phone drops and comes back with its token.
    for _ in 0..3 {
        for p in phones.iter_mut() {
            p.drain().await;
            let ui = p.ui.clone().unwrap();
            if let Some(i) = buzzer_bot(&ui, &mut Rng::new(1)) {
                p.send(i).await;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    phones[1].drop_connection();
    tokio::time::sleep(Duration::from_millis(300)).await;
    phones[1].reconnect().await;
    assert!(
        phones[1]
            .wait_until(Duration::from_secs(10), |u| text_of(u).contains("Round"))
            .await
    );
    assert_eq!(phones[1].seat, seat_before, "the seat changed on reconnect");
    play_until_finished(&cloud.addr, &id, &mut phones, buzzer_bot, 7, GAME_LIMIT).await;
    let game = get(&cloud.addr, &format!("/api/games/{id}")).await;
    let winner = game["finished"]["ranking"][0][0].as_u64().unwrap() as usize;
    for (i, p) in phones.iter_mut().enumerate() {
        assert!(
            p.wait_until(Duration::from_secs(5), |u| text_of(u).contains("win"))
                .await,
            "{} never saw the result",
            p.name
        );
        if i == winner {
            assert!(text_of(p.ui.as_ref().unwrap()).contains("You win!"));
        }
    }
    assert!(game["table"]["status"].as_str().unwrap().contains("wins"));
}

#[tokio::test]
#[ignore = "end-to-end: run with --include-ignored after scripts/packs.sh"]
async fn wizards_duel_plays_to_the_end() {
    let dir = scratch("wizards");
    let relay = relay(None).await;
    let cloud = cloud(&relay, &dir.join("cloud.db"), &[]).await;
    let (id, code) = create(&cloud.addr, json!({"pack": "wizards-duel", "pace": "live"})).await;
    let mut phones = join_and_start(&relay.addr, &cloud.addr, &id, &code, &["Ana", "Bo"]).await;
    let moves = play_until_finished(
        &cloud.addr,
        &id,
        &mut phones,
        hostable_wizards_duel::bot::choose,
        3,
        GAME_LIMIT,
    )
    .await;
    let game = get(&cloud.addr, &format!("/api/games/{id}")).await;
    assert!(!game["finished"].is_null());
    assert!(
        game["table"]["extra"]["board"]["hexes"].is_array(),
        "native renderer data missing"
    );
    eprintln!("wizards duel finished in {moves} inputs");
}

#[tokio::test]
#[ignore = "end-to-end: run with --include-ignored after scripts/packs.sh"]
async fn tideholm_plays_to_the_end() {
    let dir = scratch("tideholm");
    let relay = relay(None).await;
    let cloud = cloud(&relay, &dir.join("cloud.db"), &[]).await;
    let (id, code) = create(&cloud.addr, json!({"pack": "tideholm", "pace": "live"})).await;
    let mut phones =
        join_and_start(&relay.addr, &cloud.addr, &id, &code, &["Ana", "Bo", "Cy"]).await;
    let moves = play_until_finished(
        &cloud.addr,
        &id,
        &mut phones,
        hostable_tideholm::bot::choose,
        5,
        GAME_LIMIT,
    )
    .await;
    let game = get(&cloud.addr, &format!("/api/games/{id}")).await;
    let summary = game["finished"]["summary"].as_str().unwrap();
    assert!(!summary.is_empty());
    eprintln!("tideholm finished in {moves} inputs: {summary}");
}

#[tokio::test]
#[ignore = "end-to-end: run with --include-ignored after scripts/packs.sh"]
async fn riposte_plays_to_the_end_with_a_nested_chain() {
    let dir = scratch("riposte");
    let relay = relay(None).await;
    let cloud = cloud(&relay, &dir.join("cloud.db"), &[]).await;
    let mut deepest = 0;
    for game_no in 0..6u64 {
        let (id, code) = create(&cloud.addr, json!({"pack": "riposte", "pace": "live"})).await;
        let mut phones = join_and_start(&relay.addr, &cloud.addr, &id, &code, &["Ana", "Bo"]).await;
        play_until_finished(
            &cloud.addr,
            &id,
            &mut phones,
            hostable_riposte::bot::choose,
            game_no,
            GAME_LIMIT,
        )
        .await;
        let game = get(&cloud.addr, &format!("/api/games/{id}")).await;
        assert!(!game["finished"].is_null());
        deepest = deepest.max(
            game["table"]["extra"]["deepest_chain"]
                .as_u64()
                .unwrap_or(0),
        );
        if deepest >= 2 {
            break;
        }
    }
    assert!(
        deepest >= 2,
        "no response to a response happened in six games"
    );
}

#[tokio::test]
#[ignore = "end-to-end: run with --include-ignored after scripts/packs.sh"]
async fn host_reclaims_its_room_after_a_drop() {
    // The relay restarts (taking every connection down with it); the cloud
    // host reconnects and reclaims its room with its host token, and the
    // phones come back with their own tokens.
    let dir = scratch("reclaim");
    let state = dir.join("relay.json");
    let relay1 = relay(Some(&state)).await;
    let relay_addr = relay1.addr.clone();
    let cloud = cloud(&relay1, &dir.join("cloud.db"), &[]).await;
    let (id, code) = create(&cloud.addr, json!({"pack": "buzzer", "pace": "live"})).await;
    let mut phones = join_and_start(&relay_addr, &cloud.addr, &id, &code, &["Ana", "Bo"]).await;
    for p in phones.iter_mut() {
        assert!(
            p.wait_until(Duration::from_secs(10), |u| u.has_input("buzz"))
                .await
        );
    }
    phones[1].send(PhoneInput::press("buzz")).await;
    assert!(
        phones[0]
            .wait_until(Duration::from_secs(10), |u| u.has_input("next_round"))
            .await
    );
    // Let the relay write its state file, then kill it.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    relay1.kill().await;
    let _relay2 = relay_at(&relay_addr, &state).await;
    for p in phones.iter_mut() {
        p.reconnect_to(&relay_addr).await;
    }
    // The host's screens come back once it has reclaimed the room.
    assert!(
        phones[0]
            .wait_until(Duration::from_secs(20), |u| u.has_input("next_round"))
            .await
    );
    let game = get(&cloud.addr, &format!("/api/games/{id}")).await;
    assert_eq!(game["code"], json!(code), "the room code changed");
    play_until_finished(&cloud.addr, &id, &mut phones, buzzer_bot, 2, GAME_LIMIT).await;
}

#[tokio::test]
#[ignore = "end-to-end: run with --include-ignored after scripts/packs.sh"]
async fn cloud_restart_mid_game_loses_nothing() {
    let dir = scratch("restart");
    let db = dir.join("cloud.db");
    let relay = relay(None).await;
    let cloud1 = cloud(&relay, &db, &[]).await;
    let (id, code) = create(&cloud1.addr, json!({"pack": "tideholm", "pace": "live"})).await;
    let mut phones =
        join_and_start(&relay.addr, &cloud1.addr, &id, &code, &["Ana", "Bo", "Cy"]).await;
    let mut rng = Rng::new(11);
    let mut moves = 0;
    while moves < 80 {
        for p in phones.iter_mut() {
            p.drain().await;
            let Some(ui) = p.ui.clone() else { continue };
            if let Some(i) = hostable_tideholm::bot::choose(&ui, &mut rng) {
                p.send(i).await;
                moves += 1;
            }
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // Quiet down, then look at what's saved.
    tokio::time::sleep(Duration::from_millis(500)).await;
    for p in phones.iter_mut() {
        p.drain().await;
    }
    let before = db_snapshot(&db, &id);
    let table_before = get(&cloud1.addr, &format!("/api/games/{id}")).await["table"].clone();
    assert!(before["log"].as_array().unwrap().len() > 20);
    // The cloud process dies without warning and comes back on the same database.
    cloud1.kill().await;
    let cloud2 = cloud(&relay, &db, &[]).await;
    let game = get(&cloud2.addr, &format!("/api/games/{id}")).await;
    assert_eq!(game["code"], json!(code));
    assert_eq!(
        game["table"], table_before,
        "the table changed across the restart"
    );
    let after = db_snapshot(&db, &id);
    assert_eq!(after["state"], before["state"]);
    assert_eq!(after["log"], before["log"]);
    // Phones never reconnected; the restarted host resends their screens.
    for p in phones.iter_mut() {
        p.ui = None;
        assert!(
            p.wait_until(Duration::from_secs(20), |_| true).await,
            "{} got no screen",
            p.name
        );
    }
    play_until_finished(
        &cloud2.addr,
        &id,
        &mut phones,
        hostable_tideholm::bot::choose,
        12,
        GAME_LIMIT,
    )
    .await;
}

// ---- Web Push, received and decrypted by a fake push service ----

struct PushKeys {
    secret: p256::SecretKey,
    auth: [u8; 16],
}

fn b64url(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn decrypt_push(keys: &PushKeys, body: &[u8]) -> Vec<u8> {
    use aes_gcm::aead::{Aead, KeyInit};
    use hmac::Mac;
    use p256::elliptic_curve::sec1::ToEncodedPoint;
    let salt = &body[..16];
    let idlen = body[20] as usize;
    let as_public = &body[21..21 + idlen];
    let ciphertext = &body[21 + idlen..];
    let ua_public = keys.secret.public_key().to_encoded_point(false);
    let peer = p256::PublicKey::from_sec1_bytes(as_public).unwrap();
    let shared = p256::ecdh::diffie_hellman(keys.secret.to_nonzero_scalar(), peer.as_affine());
    let mut mac = <hmac::Hmac<sha2_push::Sha256> as Mac>::new_from_slice(&keys.auth).unwrap();
    mac.update(shared.raw_secret_bytes());
    let prk_key = mac.finalize().into_bytes();
    let mut info = b"WebPush: info\0".to_vec();
    info.extend_from_slice(ua_public.as_bytes());
    info.extend_from_slice(as_public);
    info.push(1);
    let mut mac = <hmac::Hmac<sha2_push::Sha256> as Mac>::new_from_slice(&prk_key).unwrap();
    mac.update(&info);
    let ikm = mac.finalize().into_bytes();
    let hk = hkdf::Hkdf::<sha2_push::Sha256>::new(Some(salt), &ikm);
    let mut cek = [0u8; 16];
    let mut nonce = [0u8; 12];
    hk.expand(b"Content-Encoding: aes128gcm\0", &mut cek)
        .unwrap();
    hk.expand(b"Content-Encoding: nonce\0", &mut nonce).unwrap();
    let cipher = aes_gcm::Aes128Gcm::new_from_slice(&cek).unwrap();
    let mut plain = cipher
        .decrypt(aes_gcm::Nonce::from_slice(&nonce), ciphertext)
        .expect("push message didn't decrypt");
    assert_eq!(plain.pop(), Some(2), "missing last-record delimiter");
    plain
}

#[tokio::test]
#[ignore = "end-to-end: run with --include-ignored after scripts/packs.sh"]
async fn async_game_turn_timeout() {
    use p256::elliptic_curve::sec1::ToEncodedPoint;
    let dir = scratch("async");
    // A fake push service that records what it receives.
    let (pushed_tx, mut pushed_rx) = mpsc::unbounded_channel::<(String, Vec<u8>)>();
    let app = axum::Router::new().route(
        "/push/{who}",
        axum::routing::post(
            move |axum::extract::Path(who): axum::extract::Path<String>,
                  headers: axum::http::HeaderMap,
                  body: axum::body::Bytes| {
                let tx = pushed_tx.clone();
                async move {
                    assert_eq!(headers["content-encoding"], "aes128gcm");
                    assert!(headers["authorization"]
                        .to_str()
                        .unwrap()
                        .starts_with("vapid t="));
                    let _ = tx.send((who, body.to_vec()));
                    axum::http::StatusCode::CREATED
                }
            },
        ),
    );
    let push_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let push_addr = push_listener.local_addr().unwrap();
    tokio::spawn(axum::serve(push_listener, app).into_future());
    // VAPID keys generated for this run only.
    let vapid = p256::SecretKey::random(&mut p256::elliptic_curve::rand_core::OsRng);
    let envs = [
        ("HOSTABLE_VAPID_PRIVATE_KEY", b64url(&vapid.to_bytes())),
        ("HOSTABLE_VAPID_SUBJECT", "mailto:e2e@localhost".to_string()),
    ];
    let relay = relay(None).await;
    let cloud = cloud(&relay, &dir.join("cloud.db"), &envs).await;
    let config = get(&cloud.addr, "/config.json").await;
    assert!(!config["vapid_public_key"].as_str().unwrap().is_empty());
    // An async game of noughts and crosses with a 1.5 s turn timer.
    let (id, code) = create(
        &cloud.addr,
        json!({"pack": "ts-tictactoe", "seats": ["Ana", "Bo"], "pace": "async", "turn_timer_ms": 1500}),
    )
    .await;
    let mut phones = Vec::new();
    for seat in 0..2 {
        let (status, claim) = post(
            &cloud.addr,
            &format!("/api/games/{id}/claim"),
            json!({"seat": seat}),
        )
        .await;
        assert_eq!(status, 200, "{claim}");
        assert_eq!(claim["code"], json!(code));
        let p = Phone::with_token(
            &relay.addr,
            &code,
            ["Ana", "Bo"][seat],
            claim["player"].as_u64().unwrap() as u32,
            claim["player_token"].as_str().unwrap(),
        )
        .await;
        phones.push(p);
    }
    // Bo turns on notifications.
    let keys = PushKeys {
        secret: p256::SecretKey::random(&mut p256::elliptic_curve::rand_core::OsRng),
        auth: *b"0123456789abcdef",
    };
    let sub = json!({
        "endpoint": format!("http://{push_addr}/push/bo"),
        "keys": {
            "p256dh": b64url(keys.secret.public_key().to_encoded_point(false).as_bytes()),
            "auth": b64url(&keys.auth),
        }
    });
    phones[1].send(PhoneInput::with(INPUT_PUSH, sub)).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let (status, _) = post(&cloud.addr, &format!("/api/games/{id}/start"), Value::Null).await;
    assert_eq!(status, 200);
    // Nobody plays. Ana's turn times out (she gets the first free square),
    // then it's Bo's turn, so Bo's phone is pushed.
    let (who, body) = tokio::time::timeout(Duration::from_secs(15), pushed_rx.recv())
        .await
        .expect("no push arrived")
        .unwrap();
    assert_eq!(who, "bo");
    let payload: Value = serde_json::from_slice(&decrypt_push(&keys, &body)).unwrap();
    assert_eq!(payload["title"], "Noughts and crosses");
    assert_eq!(payload["url"], format!("/g/{id}"));
    let table = get(&cloud.addr, &format!("/api/games/{id}")).await["table"].clone();
    assert!(text_of_value(&table).contains("\"label\":\"X\""), "{table}");
    // Timeouts alone carry the game to the end.
    let mut finished = Value::Null;
    for _ in 0..80 {
        finished = get(&cloud.addr, &format!("/api/games/{id}")).await["finished"].clone();
        if !finished.is_null() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert!(!finished.is_null(), "the game never finished on timeouts");
    let snap = db_snapshot(&dir.join("cloud.db"), &id);
    let timeouts = snap["log"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|a| a["kind"] == "timeout")
        .count();
    assert!(timeouts >= 5, "only {timeouts} timeouts logged");
}

fn text_of_value(v: &Value) -> String {
    v.to_string()
}

#[tokio::test]
#[ignore = "end-to-end: run with --include-ignored after scripts/packs.sh"]
async fn desktop_to_cloud_handoff_keeps_seats() {
    let dir = scratch("handoff");
    let relay = relay(None).await;
    let cloud = cloud(&relay, &dir.join("cloud.db"), &[]).await;
    // The desktop host: the same host loop the desktop app runs, with the
    // buzzer pack from disk.
    let catalog = Catalog::from_dir(&packs_dir());
    let rules = catalog
        .get("buzzer")
        .unwrap()
        .rules(Default::default())
        .unwrap();
    let room = hostable_host_core::Room::new(rules, 9).with_name("Buzzer");
    let (cmds, cmd_rx) = mpsc::unbounded_channel();
    let (upd_tx, mut updates) = mpsc::unbounded_channel();
    let options = Options {
        relay: format!("ws://{}", relay.addr),
        pack: "buzzer".into(),
        save_to: SaveTo::Nowhere,
        max_reconnects: Some(3),
    };
    let desktop = tokio::spawn(hostable_host_runtime::run(
        room, None, options, cmd_rx, upd_tx,
    ));
    let code = loop {
        if let Some(Update::Opened { code, .. }) = updates.recv().await {
            break code;
        }
    };
    let mut phones = Vec::new();
    for name in ["Ana", "Bo", "Cy"] {
        let mut p = Phone::join(&relay.addr, &code, name).await;
        assert!(p.wait_until(Duration::from_secs(10), |_| true).await);
        phones.push(p);
    }
    cmds.send(Command::Start).unwrap();
    // A round on the desktop.
    assert!(
        phones[2]
            .wait_until(Duration::from_secs(10), |u| u.has_input("buzz"))
            .await
    );
    phones[2].send(PhoneInput::press("buzz")).await;
    assert!(
        phones[0]
            .wait_until(Duration::from_secs(10), |u| u.has_input("next_round"))
            .await
    );
    let tokens: Vec<String> = phones.iter().map(|p| p.token.clone()).collect();
    // Move it online.
    let (id, url) = hand_off(&cmds, &format!("http://{}", cloud.addr), Pace::Live)
        .await
        .unwrap();
    assert_eq!(url, format!("/g/{id}"));
    assert!(desktop.await.unwrap().is_ok());
    let game = get(&cloud.addr, &format!("/api/games/{id}")).await;
    assert_eq!(game["code"], json!(code));
    assert!(
        game["table"]["players"][2]["lines"][0]
            .as_str()
            .unwrap()
            .starts_with('1'),
        "{}",
        game["table"]
    );
    // Same sockets, same seats, same tokens: play on to the end on the cloud.
    for p in phones.iter_mut() {
        p.ui = None;
        assert!(
            p.wait_until(Duration::from_secs(10), |_| true).await,
            "{} got no screen",
            p.name
        );
    }
    play_until_finished(&cloud.addr, &id, &mut phones, buzzer_bot, 4, GAME_LIMIT).await;
    let after: Vec<String> = phones.iter().map(|p| p.token.clone()).collect();
    assert_eq!(tokens, after);
    assert!(
        phones.iter().all(|p| p.errors.is_empty()),
        "phones saw errors"
    );
}

/// Replay a finished game natively and collect every screen each seat could
/// have been shown.
fn screens_by_seat(rules: &DynRules, snap: &Value) -> Vec<Vec<Widget>> {
    let mut seats: Vec<Seat> = snap["players"]
        .as_object()
        .unwrap()
        .values()
        .filter_map(|p| {
            p["seat"].as_u64().map(|s| Seat {
                id: s as u32,
                name: p["name"].as_str().unwrap().to_string(),
                color: p["color"].as_str().unwrap_or("").to_string(),
            })
        })
        .collect();
    seats.sort_by_key(|s| s.id);
    let seed = snap["seed"].as_u64().unwrap();
    let mut state = rules.setup(&seats, seed).unwrap().state;
    let mut out: Vec<Vec<Widget>> = vec![Vec::new(); seats.len()];
    let record = |state: &Value, out: &mut Vec<Vec<Widget>>| {
        for s in &seats {
            out[s.id as usize].push(rules.phone_view(state, s.id).unwrap());
        }
    };
    record(&state, &mut out);
    for entry in snap["log"].as_array().unwrap() {
        let seat = entry["seat"].as_u64().unwrap() as u32;
        state = match entry["kind"].as_str().unwrap() {
            "timeout" => {
                rules
                    .on_timeout(state, seat, entry["now_ms"].as_u64().unwrap())
                    .unwrap()
                    .state
            }
            _ => {
                let input: PhoneInput = serde_json::from_value(entry["input"].clone()).unwrap();
                rules.on_action(state, seat, &input).unwrap().state
            }
        };
        record(&state, &mut out);
    }
    out
}

#[tokio::test]
#[ignore = "end-to-end: run with --include-ignored after scripts/packs.sh"]
async fn no_phone_ever_sees_another_seats_private_view() {
    let dir = scratch("privacy");
    let db = dir.join("cloud.db");
    let relay = relay(None).await;
    let cloud = cloud(&relay, &db, &[]).await;
    let games: [(&str, &[&str], Bot, Box<DynRules>); 2] = [
        (
            "riposte",
            &["Ana", "Bo"],
            hostable_riposte::bot::choose,
            Box::new(Json(hostable_riposte::Riposte)),
        ),
        (
            "tideholm",
            &["Ana", "Bo", "Cy"],
            hostable_tideholm::bot::choose,
            Box::new(Json(hostable_tideholm::Tideholm)),
        ),
    ];
    for (pack, names, bot, native) in games {
        let (id, code) = create(&cloud.addr, json!({"pack": pack, "pace": "live"})).await;
        let mut phones = join_and_start(&relay.addr, &cloud.addr, &id, &code, names).await;
        play_until_finished(&cloud.addr, &id, &mut phones, bot, 21, GAME_LIMIT).await;
        let snap = db_snapshot(&db, &id);
        let possible = screens_by_seat(&*native, &snap);
        let mut checked = 0;
        for p in &phones {
            let mine = p.seat.expect("seated") as usize;
            for screen in &p.screens {
                let shown = without_timer(screen);
                let lobby = text_of(shown).contains("Waiting for the game to start");
                if lobby {
                    continue;
                }
                let is_mine = possible[mine].contains(shown);
                assert!(
                    is_mine,
                    "{pack}: {} was sent a screen that was never seat {mine}'s: {}",
                    p.name,
                    text_of(shown)
                );
                for (other, views) in possible.iter().enumerate() {
                    if other != mine && views.contains(shown) && !possible[mine].contains(shown) {
                        panic!("{pack}: {} saw seat {other}'s private view", p.name);
                    }
                }
                checked += 1;
            }
        }
        assert!(checked > 20, "{pack}: only {checked} screens checked");
        eprintln!("{pack}: {checked} screens checked, all private to their seat");
    }
}

#[test]
#[ignore = "end-to-end: run with --include-ignored after scripts/packs.sh"]
fn the_packs_on_disk_load_and_match_their_native_rules() {
    let catalog = Catalog::from_dir(&packs_dir());
    for (id, native) in [
        (
            "buzzer",
            Box::new(Json(hostable_buzzer::Buzzer)) as Box<DynRules>,
        ),
        ("tideholm", Box::new(Json(hostable_tideholm::Tideholm))),
        ("riposte", Box::new(Json(hostable_riposte::Riposte))),
        (
            "wizards-duel",
            Box::new(Json(hostable_wizards_duel::WizardsDuel)),
        ),
    ] {
        let wasm = catalog.get(id).unwrap().rules(Default::default()).unwrap();
        assert_eq!(wasm.seats(), native.seats(), "{id}");
        let (min, _) = native.seats();
        let seats: Vec<Seat> = (0..min).map(|i| Seat::new(i, format!("P{i}"))).collect();
        let a = wasm.setup(&seats, 42).unwrap();
        let b = native.setup(&seats, 42).unwrap();
        assert_eq!(a.state, b.state, "{id}: wasm and native setup differ");
        let (ta, tb) = (
            wasm.table_view(&a.state).unwrap(),
            native.table_view(&b.state).unwrap(),
        );
        // States must match exactly; drawing coordinates may differ in the
        // last float digit (wasm and native trig differ slightly).
        assert!(
            close(&ta, &tb),
            "{id}: table views differ at {}",
            first_difference(&ta, &tb, String::new())
        );
    }
}

/// The JSON path where two values first differ, with both sides.
fn first_difference(a: &Value, b: &Value, path: String) -> String {
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            for (k, v) in x {
                match y.get(k) {
                    Some(w) if w != v => return first_difference(v, w, format!("{path}.{k}")),
                    None => return format!("{path}.{k} missing on the right"),
                    _ => {}
                }
            }
            format!("{path}: keys differ")
        }
        (Value::Array(x), Value::Array(y)) if x.len() == y.len() => {
            for (i, (v, w)) in x.iter().zip(y).enumerate() {
                if v != w {
                    return first_difference(v, w, format!("{path}[{i}]"));
                }
            }
            path
        }
        _ => format!("{path}: {a} vs {b}"),
    }
}

fn close(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            let (x, y) = (x.as_f64().unwrap(), y.as_f64().unwrap());
            (x - y).abs() <= 1e-6 * x.abs().max(y.abs()).max(1.0)
        }
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(v, w)| close(v, w))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| close(v, w)))
        }
        _ => a == b,
    }
}
