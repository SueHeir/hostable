mod common;

use std::time::Instant;

use hostable_host_core::bot::random_input;
use hostable_sdk::prelude::*;
use hostable_sdk::protocol::PlayerId;
use hostable_wizards_duel::{State, WizardsDuel};

const MAX_INPUTS: usize = 1000;

#[test]
fn bots_finish_duels_with_a_winner() {
    let mut report = Vec::new();
    for n in 2..=4 {
        let mut total = 0;
        let mut worst = 0;
        for seed in 0..24u64 {
            let (room, inputs) = common::play(n, seed, MAX_INPUTS);
            let results = room.finished().unwrap();
            let state: &State = room.state().unwrap();
            let winner = state.winner.expect("a winner");
            assert_eq!(results.ranking[0], vec![winner]);
            assert_eq!(results.ranking.iter().flatten().count(), n);
            assert_eq!(state.wizards.iter().filter(|w| w.hp > 0).count(), 1);
            total += inputs;
            worst = worst.max(inputs);
        }
        report.push(format!(
            "{n} players: {} inputs on average, {worst} at most",
            total / 24
        ));
    }
    eprintln!("{}", report.join("\n"));
}

#[test]
fn random_inputs_never_break_the_game() {
    for n in 2..=4 {
        for seed in 0..3u64 {
            let mut room = common::room(n, seed);
            let mut rng = Rng::new(seed + 1000);
            for _ in 0..250 {
                if room.finished().is_some() {
                    break;
                }
                for p in 0..n as PlayerId {
                    let player = 100 + p;
                    let Some(ui) = room.current_ui(player).cloned() else {
                        continue;
                    };
                    if let Some(input) = random_input(&ui, &mut rng, &[("end", 10)]) {
                        room.handle_input(player, input, 0).unwrap();
                    }
                }
                assert!(room.paused().is_none(), "{:?}", room.paused());
            }
        }
    }
}

#[test]
fn phone_views_are_cheap() {
    // A mid-game position: play a while with bots, then time the current
    // seat's screen, which works out every spell's targets.
    let mut room = common::room(4, 3);
    let mut rng = Rng::new(9);
    for _ in 0..60 {
        for p in 0..4 {
            if let Some(ui) = room.current_ui(100 + p).cloned() {
                if let Some(input) = hostable_wizards_duel::bot::choose(&ui, &mut rng) {
                    room.handle_input(100 + p, input, 0).unwrap();
                }
            }
        }
    }
    let state = room.state().unwrap().clone();
    let rules = WizardsDuel;
    let runs = 200;
    let start = Instant::now();
    for _ in 0..runs {
        let _ = rules.phone_view(&state, state.current as SeatId).unwrap();
    }
    let per = start.elapsed() / runs;
    eprintln!("phone_view for the current seat: {per:?}");
    assert!(per.as_millis() < 5, "{per:?}");
}
