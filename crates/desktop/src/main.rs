//! `hostable-desktop`: see [`hostable_desktop::cli::USAGE`].

use std::time::Duration;

use bevy::app::AppExit;
use hostable_desktop::cli::{Cli, USAGE};
use hostable_desktop::{game, gui, smoke};

fn main() {
    let cli = match Cli::parse(std::env::args().skip(1)) {
        Ok(cli) => cli,
        Err(e) => {
            eprintln!("hostable-desktop: {e}\n\n{USAGE}");
            std::process::exit(2);
        }
    };
    if cli.help {
        print!("{USAGE}");
        return;
    }
    let setup = match game::prepare(&cli) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("hostable-desktop: {e}");
            std::process::exit(2);
        }
    };
    // Decide before any windowing plugin exists: the smoke test never
    // touches winit or wgpu.
    let app = if cli.smoke_test {
        smoke::app(
            setup,
            smoke::SmokeConfig {
                relay: cli.relay.clone(),
                players: cli.players,
                timeout: Duration::from_secs(60),
            },
        )
    } else {
        gui::app(setup, &cli)
    };
    let mut app = match app {
        Ok(app) => app,
        Err(e) => {
            eprintln!("hostable-desktop: {e}");
            std::process::exit(1);
        }
    };
    let code = match app.run() {
        AppExit::Success => 0,
        AppExit::Error(code) => i32::from(code.get()),
    };
    // Skip waiting on background tasks (the relay socket, phones).
    std::process::exit(code);
}
