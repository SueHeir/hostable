//! Run the relay:
//!
//! ```text
//! hostable-relay [--addr 127.0.0.1:8787] [--state relay-state.json] [--trust-proxy]
//! ```
//!
//! In production it sits behind Caddy, which handles TLS; pass
//! `--trust-proxy` there so per-IP limits see real client addresses. With
//! `--addr 127.0.0.1:0` it picks a free port and prints it.

#[tokio::main]
async fn main() {
    hostable_relay::cli::main_with_args(std::env::args().skip(1).collect()).await;
}
