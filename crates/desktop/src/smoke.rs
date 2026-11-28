//! `--smoke-test`: no window. The same [`HostPlugin`](crate::host::HostPlugin)
//! hosts the game while simulated phones join over the relay, play by
//! pressing whatever their screens offer, and the room is checked at the
//! end. The process exits 0 only if the game finished with a winner and
//! every phone saw its final screen.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bevy::app::{AppExit, ScheduleRunnerPlugin};
use bevy::prelude::*;
use futures_util::{SinkExt, StreamExt};
use hostable_host_core::bot::random_input;
use hostable_host_runtime::Command;
use hostable_protocol::{HostToPhone, PhoneToRelay, PlayerId, RelayToPhone, Widget};
use hostable_rules_api::Rng;
use tokio_tungstenite::tungstenite::Message;

use crate::game::GameSetup;
use crate::host::{HostLink, HostPlugin, HostState};

/// Buttons a phone presses first when its screen has them.
const PREFER: [(&str, u32); 2] = [("buzz", 100), ("next_round", 100)];
/// Press again if a press changed nothing for this long.
const RETRY: Duration = Duration::from_millis(800);
/// The least time a phone takes to press.
const THINK: Duration = Duration::from_millis(60);

/// What one simulated phone has seen.
#[derive(Debug, Default, Clone)]
pub struct PhoneStatus {
    pub name: String,
    pub player: Option<PlayerId>,
    pub last_ui: Option<Widget>,
    pub screens: u32,
    pub presses: u32,
    pub relay_errors: Vec<String>,
    pub error: Option<String>,
}

pub type SharedPhone = Arc<Mutex<PhoneStatus>>;

fn send_text(msg: &PhoneToRelay) -> Message {
    Message::Text(
        serde_json::to_string(msg)
            .expect("phone messages serialize")
            .into(),
    )
}

/// One phone: join room `code` on `relay`, then keep pressing what the
/// screen offers until the socket closes.
pub async fn run_phone(relay: String, code: String, status: SharedPhone, seed: u64) {
    let fail = |why: String| status.lock().expect("phone lock").error = Some(why);
    let url = format!("{}/ws/phone", relay.trim_end_matches('/'));
    let mut ws = match tokio_tungstenite::connect_async(url).await {
        Ok((ws, _)) => ws,
        Err(e) => return fail(format!("could not connect: {e}")),
    };
    let name = status.lock().expect("phone lock").name.clone();
    let join = PhoneToRelay::Join {
        code,
        name,
        color: String::new(),
    };
    if let Err(e) = ws.send(send_text(&join)).await {
        return fail(format!("could not join: {e}"));
    }
    let mut rng = Rng::new(seed);
    // The screen to act on (cleared after acting, like a person who
    // tapped and waits for the next screen).
    let mut fresh: Option<Widget> = None;
    let mut last_press: Option<Instant> = None;
    loop {
        let msg = tokio::select! {
            m = ws.next() => Some(m),
            _ = tokio::time::sleep(RETRY) => {
                // Nothing new: if a press changed nothing, try again from
                // the last screen shown.
                if last_press.is_some_and(|t| t.elapsed() >= RETRY) {
                    fresh = status.lock().expect("phone lock").last_ui.clone();
                }
                None
            }
        };
        match msg {
            Some(Some(Ok(Message::Text(text)))) => match serde_json::from_str(&text) {
                Ok(RelayToPhone::Joined { player, .. }) | Ok(RelayToPhone::Rejoined { player }) => {
                    status.lock().expect("phone lock").player = Some(player);
                }
                Ok(RelayToPhone::FromHost {
                    message: HostToPhone::Ui { ui, .. },
                }) => {
                    let mut s = status.lock().expect("phone lock");
                    s.last_ui = Some(ui.clone());
                    s.screens += 1;
                    fresh = Some(ui);
                    last_press = None;
                }
                Ok(RelayToPhone::Error { message }) => {
                    // Rate limits or "the game has ended" after the close;
                    // the check at the end decides whether play went well.
                    status
                        .lock()
                        .expect("phone lock")
                        .relay_errors
                        .push(message);
                }
                Ok(_) => {}
                Err(e) => return fail(format!("bad message from relay: {e}")),
            },
            Some(Some(Ok(Message::Close(_)))) | Some(None) => return,
            Some(Some(Err(e))) => return fail(format!("socket error: {e}")),
            Some(Some(Ok(_))) | None => {}
        }
        if let Some(ui) = fresh.take() {
            if let Some(input) = random_input(&ui, &mut rng, &PREFER) {
                // A moment to react, like a person (and so the progress log
                // shows each round).
                tokio::time::sleep(THINK + Duration::from_millis(rng.below(40) as u64)).await;
                if let Err(e) = ws.send(send_text(&PhoneToRelay::Input(input))).await {
                    return fail(format!("could not send: {e}"));
                }
                status.lock().expect("phone lock").presses += 1;
                last_press = Some(Instant::now());
            }
        }
    }
}

/// Smoke-test settings.
#[derive(Resource, Clone)]
pub struct SmokeConfig {
    pub relay: String,
    pub players: usize,
    pub timeout: Duration,
}

/// How the smoke test ended.
#[derive(Resource, Debug, Clone, PartialEq, Default)]
pub struct SmokeOutcome(pub Option<Result<String, String>>);

#[derive(Resource)]
struct SmokeRun {
    began: Instant,
    phones: Vec<SharedPhone>,
    announced: BTreeSet<PlayerId>,
    start_sent: bool,
    last_status: String,
    last_inspect: Instant,
    seen_version: u64,
}

pub struct SmokePlugin(pub SmokeConfig);

impl Plugin for SmokePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(self.0.clone())
            .insert_resource(SmokeOutcome::default())
            .insert_resource(SmokeRun {
                began: Instant::now(),
                phones: Vec::new(),
                announced: BTreeSet::new(),
                start_sent: false,
                last_status: String::new(),
                last_inspect: Instant::now(),
                seen_version: 0,
            })
            .add_systems(Update, drive_smoke);
    }
}

/// Names for the simulated players.
pub fn phone_name(i: usize) -> String {
    const NAMES: [&str; 8] = ["Ana", "Bo", "Cy", "Di", "Ed", "Flo", "Gus", "Hal"];
    if i < NAMES.len() {
        NAMES[i].to_string()
    } else {
        format!("Phone {}", i + 1)
    }
}

/// What still stops the smoke test from passing, or `None` when it passed.
pub fn check_finished(state: &HostState, phones: &[PhoneStatus]) -> Option<String> {
    let info = &state.info;
    let Some(results) = &info.finished else {
        return Some("the game has not finished".into());
    };
    if results.ranking.first().is_none_or(|w| w.is_empty()) {
        return Some("the game finished without a winner".into());
    }
    for phone in phones {
        let Some(player) = phone.player else {
            return Some(format!("{} never joined", phone.name));
        };
        let Some(expected) = info.screens.get(&player) else {
            return Some(format!("the room has no screen for {}", phone.name));
        };
        if phone.last_ui.as_ref() != Some(expected) {
            return Some(format!("{} has not seen its final screen", phone.name));
        }
    }
    None
}

fn drive_smoke(
    mut run: ResMut<SmokeRun>,
    config: Res<SmokeConfig>,
    link: Res<HostLink>,
    state: Res<HostState>,
    mut outcome: ResMut<SmokeOutcome>,
    mut exit: MessageWriter<AppExit>,
) {
    if outcome.0.is_some() {
        return;
    }
    let mut finish = |result: Result<String, String>, exit: &mut MessageWriter<AppExit>| {
        match &result {
            Ok(msg) => println!("smoke: PASS: {msg}"),
            Err(msg) => println!("smoke: FAIL: {msg}"),
        }
        link.send(Command::Close);
        exit.write(if result.is_ok() {
            AppExit::Success
        } else {
            AppExit::from_code(1)
        });
        outcome.0 = Some(result);
    };
    let phones: Vec<PhoneStatus> = run
        .phones
        .iter()
        .map(|p| p.lock().expect("phone lock").clone())
        .collect();
    if run.began.elapsed() > config.timeout {
        let why = check_finished(&state, &phones).unwrap_or_default();
        return finish(
            Err(format!("timed out after {:?}: {why}", config.timeout)),
            &mut exit,
        );
    }
    if let Some(Err(e)) = &state.ended {
        return finish(Err(format!("the host stopped: {e}")), &mut exit);
    }
    if let Some(p) = phones.iter().find(|p| p.error.is_some()) {
        let why = p.error.clone().unwrap_or_default();
        return finish(Err(format!("{}: {why}", p.name)), &mut exit);
    }
    if let Some(why) = &state.paused {
        return finish(Err(format!("the game paused: {why}")), &mut exit);
    }
    let Some(code) = state.code.clone() else {
        return;
    };
    if run.phones.is_empty() {
        println!("smoke: room {code} open on {}", config.relay);
        for i in 0..config.players {
            let status = Arc::new(Mutex::new(PhoneStatus {
                name: phone_name(i),
                ..PhoneStatus::default()
            }));
            run.phones.push(status.clone());
            link.runtime().spawn(run_phone(
                config.relay.clone(),
                code.clone(),
                status,
                0x5eed + i as u64,
            ));
        }
        return;
    }
    if state.version != run.seen_version {
        run.seen_version = state.version;
        let info = &state.info;
        for p in &info.players {
            if run.announced.insert(p.player) {
                let seat = p
                    .seat
                    .map_or("spectator".to_string(), |s| format!("seat {s}"));
                println!("smoke: {} joined ({seat})", p.name);
            }
        }
        if !run.start_sent && !info.started && info.seated() >= config.players {
            run.start_sent = true;
            println!(
                "smoke: {} seated, starting {}",
                info.seated(),
                state.game_name
            );
            link.send(Command::Start);
        }
        if let Some(Ok(table)) = &info.table {
            if table.status != run.last_status {
                run.last_status = table.status.clone();
                println!("smoke: {}", table.status);
            }
        }
        if let Some(Err(e)) = &info.table {
            return finish(Err(format!("table view failed: {e}")), &mut exit);
        }
    }
    if state.info.finished.is_some() {
        if check_finished(&state, &phones).is_none() {
            let winners = state.info.winners().join(" and ");
            let summary = state
                .info
                .finished
                .as_ref()
                .map(|r| r.summary.clone())
                .unwrap_or_default();
            let presses: u32 = phones.iter().map(|p| p.presses).sum();
            return finish(
                Ok(format!(
                    "winner {winners} ({summary}); {} phones saw their final screens; {presses} presses",
                    phones.len()
                )),
                &mut exit,
            );
        }
        // Phones may still be receiving their last screen; look again.
        if run.last_inspect.elapsed() > Duration::from_millis(250) {
            run.last_inspect = Instant::now();
            link.inspect();
        }
    }
}

/// The headless app: no window, no renderer, just the host and the phones.
pub fn app(setup: GameSetup, config: SmokeConfig) -> Result<App, String> {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(Duration::from_millis(5))))
        .add_plugins(HostPlugin::new(setup, None)?)
        .add_plugins(SmokePlugin(config));
    Ok(app)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Cli;
    use crate::game::prepare;
    use hostable_relay::server::{Config, Relay};

    /// A relay on its own runtime, as a separate process would be.
    fn relay() -> (tokio::runtime::Runtime, String) {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let listener = rt
            .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
            .unwrap();
        let addr = listener.local_addr().unwrap();
        rt.spawn(Relay::new(Config::default()).serve(listener));
        (rt, format!("ws://{addr}"))
    }

    fn run_to_exit(app: &mut App, limit: Duration) -> AppExit {
        app.finish();
        app.cleanup();
        let began = Instant::now();
        loop {
            app.update();
            if let Some(exit) = app.should_exit() {
                return exit;
            }
            assert!(began.elapsed() < limit, "the smoke app never exited");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn a_headless_host_plays_the_buzzer_with_simulated_phones() {
        let (relay_rt, base) = relay();
        let cli = Cli::parse(["--smoke-test", "--relay", &base, "--players", "3"]).unwrap();
        let setup = prepare(&cli).unwrap();
        let config = SmokeConfig {
            relay: base,
            players: 3,
            timeout: Duration::from_secs(45),
        };
        let mut app = app(setup, config).unwrap();
        let exit = run_to_exit(&mut app, Duration::from_secs(60));
        let outcome = app.world().resource::<SmokeOutcome>().0.clone();
        assert_eq!(exit, AppExit::Success, "{outcome:?}");
        let msg = outcome.unwrap().unwrap();
        assert!(msg.contains("winner"), "{msg}");
        let state = app.world().resource::<HostState>();
        assert!(state.info.finished.is_some());
        assert_eq!(state.info.seated(), 3);
        drop(app);
        relay_rt.shutdown_background();
    }

    #[test]
    fn an_unreachable_relay_fails_instead_of_hanging() {
        // A port nothing listens on.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let base = format!("ws://127.0.0.1:{port}");
        let cli = Cli::parse(["--smoke-test", "--relay", &base]).unwrap();
        let config = SmokeConfig {
            relay: base,
            players: 2,
            timeout: Duration::from_secs(20),
        };
        let mut app = app(prepare(&cli).unwrap(), config).unwrap();
        let exit = run_to_exit(&mut app, Duration::from_secs(30));
        assert_eq!(exit, AppExit::from_code(1));
        let outcome = app.world().resource::<SmokeOutcome>().0.clone();
        assert!(
            matches!(outcome, Some(Err(ref m)) if m.contains("host stopped")),
            "{outcome:?}"
        );
    }

    #[test]
    fn the_finish_check_wants_a_winner_and_final_screens() {
        let state = HostState::default();
        assert!(check_finished(&state, &[])
            .unwrap()
            .contains("not finished"));
        let mut state = HostState::default();
        state.info.finished = Some(hostable_rules_api::Results {
            ranking: vec![vec![0], vec![1]],
            summary: "Ana wins!".into(),
        });
        let screen = Widget::text("You win!");
        state.info.screens.insert(7, screen.clone());
        let mut phone = PhoneStatus {
            name: "Ana".into(),
            player: Some(7),
            ..PhoneStatus::default()
        };
        assert!(check_finished(&state, std::slice::from_ref(&phone))
            .unwrap()
            .contains("final screen"));
        phone.last_ui = Some(screen);
        assert_eq!(check_finished(&state, &[phone]), None);
    }
}
