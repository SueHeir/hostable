#![allow(dead_code)]

use hostable_host_core::{check_table, Pace, Room};
use hostable_sdk::prelude::*;
use hostable_sdk::protocol::PlayerId;
use hostable_wizards_duel::{bot, WizardsDuel};

pub const NAMES: [&str; 4] = ["Ana", "Bo", "Cy", "Dee"];

pub fn seats(n: usize) -> Vec<Seat> {
    (0..n).map(|i| Seat::new(i as SeatId, NAMES[i])).collect()
}

pub fn room(n: usize, seed: u64) -> Room<WizardsDuel> {
    room_with_pace(n, seed, Pace::Live)
}

pub fn room_with_pace(n: usize, seed: u64, pace: Pace) -> Room<WizardsDuel> {
    let mut room = Room::new(WizardsDuel, seed).with_pace(pace);
    for (i, name) in NAMES.iter().enumerate().take(n) {
        room.player_joined(100 + i as PlayerId, name.to_string(), String::new(), 0);
    }
    room.start(0).unwrap();
    room
}

/// Play a whole game with bots. Returns the room and the number of inputs.
pub fn play(n: usize, seed: u64, max_inputs: usize) -> (Room<WizardsDuel>, usize) {
    let mut room = room(n, seed);
    let mut rng = Rng::new(seed.wrapping_mul(31).wrapping_add(7));
    let mut inputs = 0;
    while room.finished().is_none() {
        assert!(room.paused().is_none(), "paused: {:?}", room.paused());
        let mut acted = false;
        for p in 0..n as PlayerId {
            let player = 100 + p;
            let Some(ui) = room.current_ui(player).cloned() else {
                continue;
            };
            // The host puts a turn timer above the screen; the bot must
            // play the same through it.
            let wrapped = Widget::column(vec![
                Widget::Timer {
                    ends_in_ms: 30_000,
                    total_ms: Some(60_000),
                    label: "Your turn".into(),
                },
                ui.clone(),
            ]);
            let mut twin = rng.clone();
            let through_timer = bot::choose(&wrapped, &mut twin);
            if let Some(input) = bot::choose(&ui, &mut rng) {
                assert!(ui.accepts(&input), "{input:?}");
                assert_eq!(through_timer.as_ref(), Some(&input));
                assert!(wrapped.accepts(&input));
                room.handle_input(player, input, 0).unwrap();
                inputs += 1;
                acted = true;
                let table = room.table_view().unwrap().unwrap();
                check_table(&table).unwrap();
                break;
            }
        }
        assert!(acted, "nobody could act (seed {seed}, {n} players)");
        assert!(
            inputs <= max_inputs,
            "seed {seed} with {n} players took over {max_inputs} inputs"
        );
    }
    (room, inputs)
}
