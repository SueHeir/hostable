//! The relay, built into the e2e package so tests can start it as a real
//! process (`CARGO_BIN_EXE_e2e-relay`). Same code as `hostable-relay`.
#[tokio::main]
async fn main() {
    hostable_relay::cli::main_with_args(std::env::args().skip(1).collect()).await;
}
