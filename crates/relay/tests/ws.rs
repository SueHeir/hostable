//! The relay over real WebSockets: message caps and abuse limits.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use hostable_protocol::{HostToRelay, PhoneInput, PhoneToRelay, RelayToHost, RelayToPhone};
use hostable_relay::limits::MAX_FAILED_JOINS;
use hostable_relay::server::{Config, Relay, MAX_MESSAGE_BYTES};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

async fn start(config: Config) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(Relay::new(config).serve(listener));
    format!("ws://{addr}")
}

async fn send<T: serde::Serialize>(ws: &mut Ws, msg: &T) {
    ws.send(Message::Text(serde_json::to_string(msg).unwrap().into()))
        .await
        .unwrap();
}

async fn recv<T: serde::de::DeserializeOwned>(ws: &mut Ws) -> Option<T> {
    loop {
        match tokio::time::timeout(Duration::from_secs(5), ws.next()).await {
            Ok(Some(Ok(Message::Text(t)))) => return Some(serde_json::from_str(&t).unwrap()),
            Ok(Some(Ok(Message::Close(_)))) | Ok(None) | Ok(Some(Err(_))) | Err(_) => return None,
            Ok(Some(Ok(_))) => continue,
        }
    }
}

async fn host(base: &str) -> (Ws, String) {
    let (mut ws, _) = connect_async(format!("{base}/ws/host")).await.unwrap();
    send(&mut ws, &HostToRelay::CreateRoom).await;
    let Some(RelayToHost::RoomCreated { code, .. }) = recv(&mut ws).await else {
        panic!("no room")
    };
    (ws, code)
}

async fn phone(base: &str, code: &str, name: &str) -> Ws {
    let (mut ws, _) = connect_async(format!("{base}/ws/phone")).await.unwrap();
    send(
        &mut ws,
        &PhoneToRelay::Join {
            code: code.into(),
            name: name.into(),
            color: String::new(),
        },
    )
    .await;
    assert!(matches!(
        recv(&mut ws).await,
        Some(RelayToPhone::Joined { .. })
    ));
    ws
}

#[tokio::test]
async fn a_phone_is_limited_to_sixty_messages_a_second() {
    let base = start(Config::default()).await;
    let (mut host_ws, code) = host(&base).await;
    let mut ph = phone(&base, &code, "Ana").await;
    assert!(matches!(
        recv::<RelayToHost>(&mut host_ws).await,
        Some(RelayToHost::PlayerJoined { .. })
    ));
    for i in 0..200 {
        send(
            &mut ph,
            &PhoneToRelay::Input(PhoneInput::press(format!("b{i}"))),
        )
        .await;
    }
    // The phone hears that it's going too fast.
    assert!(
        matches!(recv::<RelayToPhone>(&mut ph).await, Some(RelayToPhone::Error { message }) if message.contains("slow"))
    );
    let mut forwarded = 0;
    while let Ok(Some(_)) = tokio::time::timeout(
        Duration::from_millis(300),
        recv::<RelayToHost>(&mut host_ws),
    )
    .await
    {
        forwarded += 1;
    }
    assert!((60..=75).contains(&forwarded), "forwarded {forwarded}");
}

#[tokio::test]
async fn oversized_phone_messages_close_the_socket() {
    let base = start(Config::default()).await;
    let (mut host_ws, code) = host(&base).await;
    let mut ph = phone(&base, &code, "Ana").await;
    recv::<RelayToHost>(&mut host_ws).await;
    let big = "x".repeat(MAX_MESSAGE_BYTES + 1);
    let _ = ph
        .send(Message::Text(
            serde_json::to_string(&PhoneToRelay::Input(PhoneInput::press(big)))
                .unwrap()
                .into(),
        ))
        .await;
    assert!(recv::<RelayToPhone>(&mut ph).await.is_none());
    // The host never sees it, only that the phone went away.
    assert_eq!(
        recv::<RelayToHost>(&mut host_ws).await,
        Some(RelayToHost::PlayerAway { player: 1 })
    );
}

#[tokio::test]
async fn too_many_wrong_codes_from_one_ip_are_refused() {
    let base = start(Config::default()).await;
    let (_host_ws, code) = host(&base).await;
    for _ in 0..MAX_FAILED_JOINS {
        let (mut ws, _) = connect_async(format!("{base}/ws/phone")).await.unwrap();
        send(
            &mut ws,
            &PhoneToRelay::Join {
                code: "ZZZZ".into(),
                name: "x".into(),
                color: String::new(),
            },
        )
        .await;
        assert!(
            matches!(recv::<RelayToPhone>(&mut ws).await, Some(RelayToPhone::Error { message }) if message.contains("No game"))
        );
    }
    // Even the right code is refused now.
    let (mut ws, _) = connect_async(format!("{base}/ws/phone")).await.unwrap();
    send(
        &mut ws,
        &PhoneToRelay::Join {
            code,
            name: "x".into(),
            color: String::new(),
        },
    )
    .await;
    assert!(
        matches!(recv::<RelayToPhone>(&mut ws).await, Some(RelayToPhone::Error { message }) if message.contains("Too many"))
    );
}

#[tokio::test]
async fn behind_a_proxy_the_forwarded_ip_is_limited() {
    let base = start(Config {
        trust_proxy: true,
        state_file: None,
    })
    .await;
    let (_host_ws, code) = host(&base).await;
    let attempt = |ip: &'static str, code: String| {
        let base = base.clone();
        async move {
            let mut req =
                tokio_tungstenite::tungstenite::client::IntoClientRequest::into_client_request(
                    format!("{base}/ws/phone"),
                )
                .unwrap();
            req.headers_mut()
                .insert("x-forwarded-for", ip.parse().unwrap());
            let (mut ws, _) = connect_async(req).await.unwrap();
            send(
                &mut ws,
                &PhoneToRelay::Join {
                    code,
                    name: "x".into(),
                    color: String::new(),
                },
            )
            .await;
            recv::<RelayToPhone>(&mut ws).await
        }
    };
    for _ in 0..MAX_FAILED_JOINS {
        attempt("198.51.100.1", "ZZZZ".into()).await;
    }
    assert!(matches!(
        attempt("198.51.100.1", code.clone()).await,
        Some(RelayToPhone::Error { .. })
    ));
    // A different client behind the same proxy is fine.
    assert!(matches!(
        attempt("198.51.100.2", code).await,
        Some(RelayToPhone::Joined { .. })
    ));
}

#[tokio::test]
async fn the_state_file_brings_rooms_back() {
    let dir = std::env::temp_dir().join(format!("hostable-relay-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("state.json");
    let _ = std::fs::remove_file(&path);
    let config = Config {
        trust_proxy: false,
        state_file: Some(path.clone()),
    };
    let relay = Relay::new(config.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("ws://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(relay.clone().serve(listener));
    let (_h, code) = host(&base).await;
    relay.save_if_dirty();
    server.abort();
    let saved: hostable_relay::Saved =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(saved.rooms.len(), 1);
    assert_eq!(saved.rooms[0].code, code);
    let again = Relay::new(config);
    assert_eq!(again.registry.lock().unwrap().room_count(), 1);
    std::fs::remove_dir_all(&dir).unwrap();
}
