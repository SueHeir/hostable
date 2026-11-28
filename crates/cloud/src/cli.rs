//! The `hostable-cloud` command line.

use std::path::PathBuf;
use std::sync::Arc;

use crate::push::{Disabled, PushSender, WebPush};
use crate::{Cloud, Config};
use hostable_host_runtime::{Catalog, Quota};
use hostable_sandbox::Limits;

const USAGE: &str = "hostable-cloud --relay URL --db FILE --packs DIR [--addr ADDR] [--client-dir DIR] [--base /] [--public-relay URL]";

/// Run the cloud host with command-line arguments (without the program name).
pub async fn main_with_args(args: Vec<String>) {
    let mut addr = "127.0.0.1:8790".to_string();
    let mut relay = None;
    let mut public_relay = None;
    let mut db = None;
    let mut packs = None;
    let mut client_dir = None;
    let mut base = "/".to_string();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let mut value = || {
            args.next()
                .unwrap_or_else(|| fail(&format!("{arg} needs a value")))
        };
        match arg.as_str() {
            "--addr" => addr = value(),
            "--relay" => relay = Some(value()),
            "--public-relay" => public_relay = Some(value()),
            "--db" => db = Some(PathBuf::from(value())),
            "--packs" => packs = Some(PathBuf::from(value())),
            "--client-dir" => client_dir = Some(PathBuf::from(value())),
            "--base" => base = value(),
            "--help" | "-h" => {
                println!("{USAGE}");
                return;
            }
            other => fail(&format!("unknown option {other}")),
        }
    }
    let relay = relay.unwrap_or_else(|| fail("--relay is required"));
    if !base.ends_with('/') {
        base.push('/');
    }
    let catalog = Catalog::from_dir(&packs.unwrap_or_else(|| fail("--packs is required")));
    for (file, why) in &catalog.rejected {
        eprintln!("hostable-cloud: skipped {file}: {why}");
    }
    let push: Arc<dyn PushSender> = match WebPush::from_env() {
        Some(Ok(p)) => Arc::new(p),
        Some(Err(e)) => fail(&e),
        None => Arc::new(Disabled),
    };
    let config = Config {
        public_relay: public_relay
            .unwrap_or_else(|| format!("{}/ws/phone", relay.trim_end_matches('/'))),
        relay,
        base,
        db: db.unwrap_or_else(|| fail("--db is required")),
        client_dir,
        admin_token: std::env::var("HOSTABLE_ADMIN_TOKEN")
            .ok()
            .filter(|t| !t.is_empty()),
        limits: Limits::DEFAULT,
        quota: Quota::default(),
    };
    let cloud = Cloud::new(config, catalog, push).unwrap_or_else(|e| fail(&e));
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| fail(&format!("bind {addr}: {e}")));
    let resumed = cloud.resume_games();
    if !resumed.is_empty() {
        eprintln!("hostable-cloud: resumed {} games", resumed.len());
    }
    println!(
        "hostable cloud listening on {}",
        listener.local_addr().unwrap()
    );
    axum::serve(listener, cloud.router())
        .await
        .expect("cloud server");
}

fn fail(message: &str) -> ! {
    eprintln!("hostable-cloud: {message}\n{USAGE}");
    std::process::exit(2);
}
