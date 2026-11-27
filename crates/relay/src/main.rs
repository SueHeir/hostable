//! Run the relay:
//!
//! ```text
//! hostable-relay [--addr 127.0.0.1:8787] [--state relay-state.json] [--trust-proxy]
//! ```
//!
//! In production it sits behind Caddy, which handles TLS; pass
//! `--trust-proxy` there so per-IP limits see real client addresses. With
//! `--addr 127.0.0.1:0` it picks a free port and prints it.

use std::path::PathBuf;

use hostable_relay::server::{Config, Relay};

#[tokio::main]
async fn main() {
    let mut addr = "127.0.0.1:8787".to_string();
    let mut config = Config::default();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--addr" => addr = args.next().expect("--addr needs a value"),
            "--state" => {
                config.state_file = Some(PathBuf::from(args.next().expect("--state needs a path")))
            }
            "--trust-proxy" => config.trust_proxy = true,
            "--help" | "-h" => {
                println!("hostable-relay [--addr ADDR] [--state FILE] [--trust-proxy]");
                return;
            }
            other if !other.starts_with('-') => addr = other.to_string(),
            other => {
                eprintln!("unknown option {other}");
                std::process::exit(2);
            }
        }
    }
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("bind relay address");
    println!(
        "hostable relay listening on {}",
        listener.local_addr().unwrap()
    );
    Relay::new(config)
        .serve(listener)
        .await
        .expect("relay server");
}
