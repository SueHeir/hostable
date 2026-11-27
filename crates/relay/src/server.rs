//! WebSocket endpoints for the relay: `/ws/host` and `/ws/phone`, plus
//! `/healthz`.

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, State};
use axum::http::HeaderMap;
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use hostable_protocol::{HostToRelay, PhoneToRelay, RelayToHost, RelayToPhone};
use tokio::net::TcpListener;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};

use crate::limits::{JoinGuard, RateLimiter, PHONE_MESSAGES_PER_SECOND};
use crate::{to_json, Outbox, Registry, RelayError, Saved};

/// Largest message a phone may send, in bytes.
pub const MAX_MESSAGE_BYTES: usize = 16 * 1024;
/// Largest message a host may send (whole screens and table views).
pub const MAX_HOST_MESSAGE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Default)]
pub struct Config {
    /// Take the client IP from `X-Forwarded-For` (set this behind Caddy).
    pub trust_proxy: bool,
    /// Save rooms and tokens here so they survive a restart.
    pub state_file: Option<PathBuf>,
}

#[derive(Clone)]
pub struct Relay {
    pub registry: Arc<Mutex<Registry>>,
    guard: Arc<Mutex<JoinGuard>>,
    config: Arc<Config>,
}

impl Relay {
    /// A relay, with rooms loaded from the state file if there is one.
    pub fn new(config: Config) -> Self {
        let registry = config
            .state_file
            .as_ref()
            .and_then(|path| std::fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice::<Saved>(&bytes).ok())
            .map(|saved| Registry::restore(saved, Instant::now()))
            .unwrap_or_default();
        Self {
            registry: Arc::new(Mutex::new(registry)),
            guard: Arc::new(Mutex::new(JoinGuard::default())),
            config: Arc::new(config),
        }
    }

    pub fn router(&self) -> Router {
        Router::new()
            .route("/healthz", get(|| async { "ok" }))
            .route("/ws/host", get(host_socket))
            .route("/ws/phone", get(phone_socket))
            .with_state(self.clone())
    }

    /// Close abandoned rooms once a minute and save state once a second.
    pub fn spawn_background(&self) {
        let relay = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(1));
            let mut n: u64 = 0;
            loop {
                tick.tick().await;
                n += 1;
                if n.is_multiple_of(60) {
                    let now = Instant::now();
                    relay.registry.lock().unwrap().sweep(now);
                    relay.guard.lock().unwrap().sweep(now);
                }
                relay.save_if_dirty();
            }
        });
    }

    /// Write the state file if rooms or tokens changed.
    pub fn save_if_dirty(&self) {
        let Some(path) = &self.config.state_file else {
            return;
        };
        let saved = {
            let mut reg = self.registry.lock().unwrap();
            if !reg.is_dirty() {
                return;
            }
            reg.mark_saved();
            reg.save()
        };
        let tmp = path.with_extension("tmp");
        let text = serde_json::to_vec(&saved).expect("saved state serializes");
        if std::fs::write(&tmp, text)
            .and_then(|()| std::fs::rename(&tmp, path))
            .is_err()
        {
            eprintln!("hostable-relay: could not write {}", path.display());
            self.registry.lock().unwrap().dirty = true;
        }
    }

    /// Serve on an already bound listener until the process ends.
    pub async fn serve(self, listener: TcpListener) -> std::io::Result<()> {
        self.spawn_background();
        axum::serve(
            listener,
            self.router()
                .into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
    }
}

fn client_ip(relay: &Relay, addr: SocketAddr, headers: &HeaderMap) -> IpAddr {
    if relay.config.trust_proxy {
        let forwarded = headers
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(',').next())
            .and_then(|v| v.trim().parse().ok());
        if let Some(ip) = forwarded {
            return ip;
        }
    }
    addr.ip()
}

async fn host_socket(ws: WebSocketUpgrade, State(relay): State<Relay>) -> Response {
    ws.max_message_size(MAX_HOST_MESSAGE_BYTES)
        .max_frame_size(MAX_HOST_MESSAGE_BYTES)
        .on_upgrade(move |socket| run_host(socket, relay))
}

async fn phone_socket(
    ws: WebSocketUpgrade,
    State(relay): State<Relay>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    let ip = client_ip(&relay, addr, &headers);
    ws.max_message_size(MAX_MESSAGE_BYTES)
        .max_frame_size(MAX_MESSAGE_BYTES)
        .on_upgrade(move |socket| run_phone(socket, relay, ip))
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
        let _ = sink.close().await;
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

async fn run_host(socket: WebSocket, relay: Relay) {
    let (sink, mut stream) = socket.split();
    let (tx, rx) = unbounded_channel();
    spawn_writer(sink, rx);
    let reg = &relay.registry;

    let Some(first) = next_text(&mut stream).await else {
        return;
    };
    let (code, conn) = match serde_json::from_str::<HostToRelay>(&first) {
        Ok(HostToRelay::CreateRoom) => reg.lock().unwrap().create_room(tx.clone()),
        Ok(HostToRelay::ReclaimRoom { code, host_token }) => {
            let res = reg
                .lock()
                .unwrap()
                .reclaim_room(&code, &host_token, tx.clone());
            match res {
                Ok(conn) => (code, conn),
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
        if !reg.lock().unwrap().from_host(&code, conn, msg) {
            return;
        }
    }
    reg.lock().unwrap().host_left(&code, conn, Instant::now());
}

async fn run_phone(socket: WebSocket, relay: Relay, ip: IpAddr) {
    let (sink, mut stream) = socket.split();
    let (tx, rx) = unbounded_channel();
    spawn_writer(sink, rx);
    let reg = &relay.registry;

    let Some(first) = next_text(&mut stream).await else {
        return;
    };
    if !relay.guard.lock().unwrap().may_try(ip, Instant::now()) {
        return phone_error(&tx, RelayError::TooManyAttempts.message());
    }
    let joined = match serde_json::from_str::<PhoneToRelay>(&first) {
        Ok(PhoneToRelay::Join { code, name, color }) => {
            let res = reg.lock().unwrap().join(&code, &name, &color, tx.clone());
            res.map(|p| (code.trim().to_ascii_uppercase(), p))
        }
        Ok(PhoneToRelay::Rejoin { code, player_token }) => {
            let res = reg.lock().unwrap().rejoin(&code, &player_token, tx.clone());
            res.map(|p| (code.trim().to_ascii_uppercase(), p))
        }
        _ => Err(RelayError::NoSuchRoom),
    };
    let (code, (player, conn)) = match joined {
        Ok(joined) => joined,
        Err(e) => {
            if matches!(e, RelayError::NoSuchRoom | RelayError::BadToken) {
                relay.guard.lock().unwrap().failed(ip, Instant::now());
            }
            return phone_error(&tx, e.message());
        }
    };

    let mut limiter = RateLimiter::new(PHONE_MESSAGES_PER_SECOND, Instant::now());
    let mut last_warning: Option<Instant> = None;
    while let Some(text) = next_text(&mut stream).await {
        let now = Instant::now();
        if !limiter.allow(now) {
            if last_warning.is_none_or(|t| now.duration_since(t) >= Duration::from_secs(1)) {
                last_warning = Some(now);
                phone_error(&tx, "Too many messages; slow down.");
            }
            continue;
        }
        if let Ok(PhoneToRelay::Input(input)) = serde_json::from_str(&text) {
            reg.lock().unwrap().from_player(&code, player, conn, input);
        }
    }
    reg.lock().unwrap().player_left(&code, player, conn);
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
