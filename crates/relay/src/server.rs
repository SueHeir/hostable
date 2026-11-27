//! WebSocket endpoints for the relay: `/ws/host` and `/ws/phone`.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use hostable_protocol::{HostToRelay, PhoneToRelay, RelayToHost, RelayToPhone};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};

use crate::{to_json, Outbox, Registry, RelayError};

/// Largest message the relay accepts, in bytes.
pub const MAX_MESSAGE_BYTES: usize = 16 * 1024;

pub type Shared = Arc<Mutex<Registry>>;

pub fn router(registry: Shared) -> Router {
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/ws/host", get(host_socket))
        .route("/ws/phone", get(phone_socket))
        .with_state(registry)
}

/// Close rooms whose host never came back, once a minute.
pub fn spawn_sweeper(registry: Shared) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(60));
        loop {
            tick.tick().await;
            registry.lock().unwrap().sweep(Instant::now());
        }
    });
}

async fn host_socket(ws: WebSocketUpgrade, State(reg): State<Shared>) -> Response {
    ws.max_message_size(MAX_MESSAGE_BYTES)
        .on_upgrade(move |socket| run_host(socket, reg))
}

async fn phone_socket(ws: WebSocketUpgrade, State(reg): State<Shared>) -> Response {
    ws.max_message_size(MAX_MESSAGE_BYTES)
        .on_upgrade(move |socket| run_phone(socket, reg))
}

/// Pump an outbox into the socket's sending half.
fn spawn_writer(
    mut sink: futures_util::stream::SplitSink<WebSocket, Message>,
    mut rx: UnboundedReceiver<String>,
) {
    tokio::spawn(async move {
        while let Some(text) = rx.recv().await {
            if sink.send(Message::Text(text.into())).await.is_err() {
                break;
            }
        }
    });
}

async fn next_text(stream: &mut futures_util::stream::SplitStream<WebSocket>) -> Option<String> {
    while let Some(Ok(msg)) = stream.next().await {
        match msg {
            Message::Text(text) => return Some(text.to_string()),
            Message::Close(_) => return None,
            _ => continue,
        }
    }
    None
}

async fn run_host(socket: WebSocket, reg: Shared) {
    let (sink, mut stream) = socket.split();
    let (tx, rx) = unbounded_channel();
    spawn_writer(sink, rx);

    let Some(first) = next_text(&mut stream).await else {
        return;
    };
    let code = match serde_json::from_str::<HostToRelay>(&first) {
        Ok(HostToRelay::CreateRoom) => reg.lock().unwrap().create_room(tx.clone()),
        Ok(HostToRelay::ReclaimRoom { code, host_token }) => {
            match reg
                .lock()
                .unwrap()
                .reclaim_room(&code, &host_token, tx.clone())
            {
                Ok(()) => code,
                Err(e) => return host_error(&tx, e.message()),
            }
        }
        _ => return host_error(&tx, "Start with create_room or reclaim_room."),
    };

    while let Some(text) = next_text(&mut stream).await {
        let Ok(msg) = serde_json::from_str::<HostToRelay>(&text) else {
            host_error(&tx, "Unreadable message.");
            continue;
        };
        if !reg.lock().unwrap().from_host(&code, msg) {
            return;
        }
    }
    reg.lock().unwrap().host_left(&code, Instant::now());
}

async fn run_phone(socket: WebSocket, reg: Shared) {
    let (sink, mut stream) = socket.split();
    let (tx, rx) = unbounded_channel();
    spawn_writer(sink, rx);

    let Some(first) = next_text(&mut stream).await else {
        return;
    };
    let joined = match serde_json::from_str::<PhoneToRelay>(&first) {
        Ok(PhoneToRelay::Join { code, name }) => {
            let res = reg.lock().unwrap().join(&code, &name, tx.clone());
            res.map(|p| (code.trim().to_ascii_uppercase(), p))
        }
        Ok(PhoneToRelay::Rejoin { code, player_token }) => {
            let res = reg.lock().unwrap().rejoin(&code, &player_token, tx.clone());
            res.map(|p| (code.trim().to_ascii_uppercase(), p))
        }
        _ => Err(RelayError::NoSuchRoom),
    };
    let (code, player) = match joined {
        Ok(joined) => joined,
        Err(e) => return phone_error(&tx, e.message()),
    };

    // TODO(phase 1): per-phone rate limit (60 messages a second) and a
    // per-IP limit on failed joins.
    while let Some(text) = next_text(&mut stream).await {
        if let Ok(PhoneToRelay::Input(input)) = serde_json::from_str(&text) {
            reg.lock().unwrap().from_player(&code, player, input);
        }
    }
    reg.lock().unwrap().player_left(&code, player);
}

fn host_error(tx: &Outbox, message: &str) {
    let _ = tx.send(to_json(&RelayToHost::Error {
        message: message.into(),
    }));
}

fn phone_error(tx: &Outbox, message: &str) {
    let _ = tx.send(to_json(&RelayToPhone::Error {
        message: message.into(),
    }));
}
