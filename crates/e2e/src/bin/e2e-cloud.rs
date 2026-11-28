//! The cloud host, built into the e2e package so tests can start it as a
//! real process (`CARGO_BIN_EXE_e2e-cloud`). Same code as `hostable-cloud`.
#[tokio::main]
async fn main() {
    hostable_cloud::cli::main_with_args(std::env::args().skip(1).collect()).await;
}
