//! Run the cloud host:
//!
//! ```text
//! hostable-cloud --relay ws://127.0.0.1:8787 --db cloud.db --packs packs/ \
//!     [--addr 127.0.0.1:8790] [--client-dir client/dist] [--base /play/] \
//!     [--public-relay wss://example.com/play/ws/phone]
//! ```
//!
//! Environment: `HOSTABLE_ADMIN_TOKEN` enables the admin endpoints;
//! `HOSTABLE_VAPID_PRIVATE_KEY` and `HOSTABLE_VAPID_SUBJECT` enable Web Push.
//! With `--addr 127.0.0.1:0` it picks a free port and prints it.

#[tokio::main]
async fn main() {
    hostable_cloud::cli::main_with_args(std::env::args().skip(1).collect()).await;
}
