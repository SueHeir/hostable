//! The `hostable-relay` command line.

use std::path::PathBuf;

use crate::server::{Config, Relay};

/// Run the relay with command-line arguments (without the program name).
pub async fn main_with_args(args: Vec<String>) {
    let mut addr = "127.0.0.1:8787".to_string();
    let mut config = Config::default();
    let mut args = args.into_iter();
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
