//! Run the relay: `hostable-relay [ADDR]` (default 127.0.0.1:8787).
//! In production it sits behind Caddy, which handles TLS.

use std::sync::{Arc, Mutex};

use hostable_relay::{server, Registry};

#[tokio::main]
async fn main() {
    let addr = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:8787".into());
    let registry = Arc::new(Mutex::new(Registry::default()));
    server::spawn_sweeper(registry.clone());
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("bind relay address");
    println!("hostable relay listening on {addr}");
    axum::serve(listener, server::router(registry))
        .await
        .expect("relay server");
}
