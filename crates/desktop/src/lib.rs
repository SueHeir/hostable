//! The Hostable desktop host: a Bevy app that shows the table on a shared
//! screen while players join from their phones by scanning a QR code.
//!
//! The game itself runs in [`hostable_host_runtime::run`] on a background
//! tokio runtime ([`host`]); Bevy only shows it. `--smoke-test` runs the same
//! host without a window ([`smoke`]).

pub mod cli;
pub mod color;
pub mod config;
pub mod duel;
pub mod game;
pub mod geometry;
pub mod gui;
pub mod host;
pub mod qr;
pub mod scene;
pub mod smoke;
