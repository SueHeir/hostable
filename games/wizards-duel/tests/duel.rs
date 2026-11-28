mod common;

use hostable_host_core::{replay, LoggedAction, Pace};
use hostable_sdk::prelude::*;
use hostable_wizards_duel::{
    arena, bot, claim_value, read_input, targets, vertex_value, State, Step, Wizard, WizardsDuel,
    TURN_LIVE_MS,
};
use wizards_rules::{Action, Aim, Game, Mana, Terrain, Vertex};

fn setup(n: usize, seed: u64) -> State {
    WizardsDuel.setup(&common::seats(n), seed).unwrap().state
}

/// Everything a player could observe about two games is the same.
fn assert_same_game(a: &Game, b: &Game) {
    let pa: Vec<Wizard> = a.players.iter().map(Wizard::from_player).collect();
    let pb: Vec<Wizard> = b.players.iter().map(Wizard::from_player).collect();
    assert_eq!(pa, pb);
    assert_eq!(a.board.def, b.board.def);
    assert_eq!(a.zones, b.zones);
    assert_eq!(
        (a.current, a.turn, a.over, a.cleared),
        (b.current, b.turn, b.over, b.cleared)
    );
    let mut vertices: Vec<Vertex> = a.board.vertices().into_iter().collect();
    vertices.sort();
    let mut vb: Vec<Vertex> = b.board.vertices().into_iter().collect();
    vb.sort();
    assert_eq!(vertices, vb);
    for v in &vertices {
        assert_eq!(a.piece_at(*v), b.piece_at(*v), "{v:?}");
        assert_eq!(a.board.is_rock(*v), b.board.is_rock(*v));
    }
    for def in arena::spells() {
        assert_eq!(
            a.spell_targets(&def.id),
            b.spell_targets(&def.id),
            "{}",
            def.id
        );
        assert_eq!(a.auto_pay(&def.id), b.auto_pay(&def.id), "{}", def.id);
    }
    for m in Mana::ALL {
        assert_eq!(a.payment_options(m), b.payment_options(m), "{m:?}");
    }
}

fn round_trip(state: &State) -> State {
    let text = serde_json::to_string(state).unwrap();
    let back: State = serde_json::from_str(&text).unwrap();
    assert_eq!(&back, state);
    assert_eq!(serde_json::to_string(&back).unwrap(), text);
    back
}

/// Bots play through the rules directly while the same actions drive a
/// live `wizards-rules` game built once at the start. After every input the
/// state's JSON round-trips and rebuilds a game equal to the live one.
#[test]
fn the_state_mirror_rebuilds_the_live_game() {
    let (mut claims, mut spends, mut zones, mut casts) = (0, 0, 0, 0);
    for seed in 0..12u64 {
        let n = 2 + (seed % 3) as usize;
        let mut state = setup(n, seed);
        let mut live = Game::new(
            state.board.clone(),
            arena::book(),
            state.wizards.iter().map(Wizard::to_player).collect(),
        );
        live.current = state.current;
        let mut rng = Rng::new(seed);
        for _ in 0..400 {
            if state.over {
                break;
            }
            let seat = state.current as SeatId;
            let ui = WizardsDuel.phone_view(&state, seat).unwrap();
            let input = bot::choose(&ui, &mut rng).expect("the current seat can act");
            let step = read_input(&state, &state.game(), seat, &input).unwrap();
            if let Step::Act(action) = &step {
                if matches!(action, Action::EndTurn { claim } if !claim.is_empty()) {
                    claims += 1;
                }
                if matches!(action, Action::Cast { .. }) {
                    casts += 1;
                }
                live.apply(action.clone()).unwrap();
            }
            state = WizardsDuel.on_action(state, seat, &input).unwrap().state;
            spends += state
                .wizards
                .iter()
                .filter(|w| !w.board_used.is_empty())
                .count();
            zones += state.zones.len();
            let back = round_trip(&state);
            assert_same_game(&back.game(), &live);
            // The phone's shared-search targets match `wizards-rules`.
            let dist = live.board.distances_from(live.players[live.current].at);
            for def in arena::spells() {
                assert_eq!(
                    targets(&live, &def, &dist),
                    live.spell_targets(&def.id),
                    "{}",
                    def.id
                );
            }
        }
        assert!(state.over, "seed {seed} didn't finish");
    }
    assert!(claims > 0 && spends > 0 && casts > 0, "{claims} {spends}");
    assert!(zones > 0, "no game left a lingering zone to check");

    // Terrain and rocks change the board definition; the mirror keeps it.
    let mut state = setup(2, 5);
    let mut live = state.game();
    live.board
        .set_terrain(wizards_rules::Hex::new(0, 0), Terrain::Ice);
    let rock = state.board.rocks[0];
    assert!(live.board.remove_rock(rock));
    state.sync(&live);
    let back = round_trip(&state);
    let rebuilt = back.game();
    assert_same_game(&rebuilt, &live);
    assert!(!rebuilt.board.is_rock(rock));
    assert_eq!(
        rebuilt.board.terrain_at(wizards_rules::Hex::new(0, 0)),
        Some(Terrain::Ice)
    );
}

/// Put the current wizard (seat 0) next to seat 1, with red claimed.
fn face_off() -> State {
    let mut state = setup(2, 1);
    state.current = 0;
    let at = state.wizards[0].at;
    let game = state.game();
    let next = game
        .board
        .neighbors(at)
        .into_iter()
        .find(|v| !game.is_blocked(*v))
        .unwrap();
    state.wizards[1].at = next;
    state.wizards[0].spells[0] = "fire_swipe".into();
    state.wizards[1].spells[0] = "drain".into();
    // Known, but purple isn't anywhere near.
    state.wizards[0].spells[3] = "barrier".into();
    state.wizards[1].spells[3] = "ice_ward".into();
    state.wizards[0].claimed.push(wizards_rules::Claim {
        mana: Mana::Red,
        from: wizards_rules::Hex::new(3, 0),
        used: false,
    });
    state
}

#[test]
fn inputs_map_to_the_right_actions() {
    let state = face_off();
    let game = state.game();
    let bo = state.wizards[1].at;
    let me = state.wizards[0].at;
    let step_to = game
        .board
        .neighbors(me)
        .into_iter()
        .find(|v| !game.is_blocked(*v))
        .unwrap();
    let read = |s: &State, input: PhoneInput| read_input(s, &s.game(), 0, &input);

    assert_eq!(
        read(&state, PhoneInput::with("move", vertex_value(step_to))),
        Ok(Step::Act(Action::Move(step_to)))
    );
    assert_eq!(
        read(&state, PhoneInput::with("end", "end")),
        Ok(Step::Act(Action::EndTurn { claim: vec![] }))
    );
    let hex = me.touching()[0];
    assert_eq!(
        read(&state, PhoneInput::with("end", claim_value(hex))),
        Ok(Step::Act(Action::EndTurn { claim: vec![hex] }))
    );
    assert_eq!(
        read(&state, PhoneInput::with("cast", "fire_swipe")),
        Ok(Step::Aim("fire_swipe".into()))
    );

    // Picking a spell opens its targets; picking one casts it, paid the
    // way `auto_pay` pays.
    let out = WizardsDuel
        .on_action(state.clone(), 0, &PhoneInput::with("cast", "fire_swipe"))
        .unwrap();
    let aiming = out.state;
    assert_eq!(aiming.aiming.as_deref(), Some("fire_swipe"));
    let ui = WizardsDuel.phone_view(&aiming, 0).unwrap();
    assert!(ui.has_input("aim") && ui.has_input("cancel") && !ui.has_input("move"));
    let pay = game.auto_pay("fire_swipe").unwrap();
    assert_eq!(
        read(&aiming, PhoneInput::with("aim", vertex_value(bo))),
        Ok(Step::Act(Action::Cast {
            spell: "fire_swipe".into(),
            target: Aim::Vertex(bo),
            pay,
        }))
    );
    let hit = WizardsDuel
        .on_action(
            aiming.clone(),
            0,
            &PhoneInput::with("aim", vertex_value(bo)),
        )
        .unwrap();
    assert_eq!(hit.state.wizards[1].hp, state.wizards[1].hp - 1);
    assert!(hit.state.wizards[0].claimed[0].used);
    assert_eq!(hit.state.aiming, None);
    assert!(hit
        .events
        .iter()
        .any(|e| matches!(e, Event::Animate { .. })));
    assert!(hit.state.log.iter().any(|l| l == "Ana casts Fire swipe."));
    let back = WizardsDuel
        .on_action(aiming, 0, &PhoneInput::press("cancel"))
        .unwrap();
    assert_eq!(back.state.aiming, None);

    // A step moves the wizard and spends a step; ending the turn passes
    // it on with the timer.
    let moved = WizardsDuel
        .on_action(
            state.clone(),
            0,
            &PhoneInput::with("move", vertex_value(step_to)),
        )
        .unwrap()
        .state;
    assert_eq!(moved.wizards[0].at, step_to);
    assert_eq!(moved.wizards[0].steps_left, state.wizards[0].steps_left - 1);
    let ended = WizardsDuel
        .on_action(moved, 0, &PhoneInput::with("end", "end"))
        .unwrap();
    assert_eq!(ended.state.current, 1);
    assert_eq!(ended.waiting_on.len(), 1);
    assert_eq!(ended.waiting_on[0].seat, 1);
    assert_eq!(ended.waiting_on[0].live_timeout_ms, TURN_LIVE_MS);
}

#[test]
fn bad_inputs_leave_the_state_unchanged() {
    let state = face_off();
    let far = state.wizards[0].at.touching()[0].corners()[0];
    let mut aiming = state.clone();
    aiming.aiming = Some("fire_swipe".into());
    let cases: Vec<(&State, SeatId, PhoneInput)> = vec![
        (&state, 1, PhoneInput::with("end", "end")),
        (&state, 99, PhoneInput::with("end", "end")),
        (&state, 0, PhoneInput::with("move", 7)),
        (&state, 0, PhoneInput::with("move", "v:x:y:N")),
        (&state, 0, PhoneInput::with("move", "v:40:40:N")),
        (
            &state,
            0,
            PhoneInput::with("move", vertex_value(state.wizards[1].at)),
        ),
        (&state, 0, PhoneInput::with("cast", "meteor")),
        (&state, 0, PhoneInput::with("cast", "drain")),
        // Known but unpayable.
        (&state, 0, PhoneInput::with("cast", "barrier")),
        (&state, 0, PhoneInput::with("aim", vertex_value(far))),
        (&state, 0, PhoneInput::press("cancel")),
        (&state, 0, PhoneInput::with("end", "claim:40:40")),
        (&state, 0, PhoneInput::with("end", json!(["end"]))),
        (&state, 0, PhoneInput::press("nonsense")),
        (&aiming, 0, PhoneInput::with("aim", "h:0:0")),
        (&aiming, 0, PhoneInput::with("aim", vertex_value(far))),
        (&aiming, 0, PhoneInput::with("end", "end")),
    ];
    for (before, seat, input) in cases {
        let out = WizardsDuel.on_action(before.clone(), seat, &input).unwrap();
        assert_eq!(&out.state, before, "{seat} {input:?}");
        assert_eq!(out.waiting_on.len(), 1, "the turn timer stays");
        assert_eq!(out.waiting_on[0].seat, 0);
        if seat < 2 {
            assert!(
                out.events.iter().any(|e| matches!(
                    e,
                    Event::Feedback {
                        kind: Feedback::Warning,
                        ..
                    }
                )),
                "{input:?}"
            );
        } else {
            assert!(out.events.is_empty());
        }
    }
    // Once the duel is over, nothing changes it.
    let mut over = state.clone();
    over.over = true;
    over.winner = Some(0);
    let out = WizardsDuel
        .on_action(over.clone(), 0, &PhoneInput::with("end", "end"))
        .unwrap();
    assert_eq!(out.state, over);
    assert!(out.finished.is_some());
    assert!(WizardsDuel
        .phone_view(&over, 0)
        .unwrap()
        .inputs()
        .is_empty());
    assert!(WizardsDuel
        .phone_view(&over, 1)
        .unwrap()
        .inputs()
        .is_empty());
}

/// The same state with every wizard's secrets wiped.
fn without_secrets(state: &State, keep: Option<usize>) -> State {
    let mut s = state.clone();
    for (i, w) in s.wizards.iter_mut().enumerate() {
        if Some(i) != keep {
            w.spells.clear();
            w.claimed.clear();
            w.board_used.clear();
        }
    }
    if keep != Some(s.current) {
        s.aiming = None;
    }
    s
}

#[test]
fn spells_and_claimed_mana_stay_on_their_own_phone() {
    // Dealt hands never overlap, so another seat's spell names on a
    // screen would be a leak.
    for n in 2..=4 {
        let state = setup(n, 11);
        let table = WizardsDuel.table_view(&state).unwrap().to_string();
        for (a, wa) in state.wizards.iter().enumerate() {
            let phone =
                serde_json::to_string(&WizardsDuel.phone_view(&state, a as SeatId).unwrap())
                    .unwrap();
            for id in &wa.spells {
                let name = arena::spell_name(id);
                assert!(!table.contains(&name), "table shows {name}");
                assert!(phone.contains(&name), "seat {a} should see {name}");
                for b in 0..n {
                    if b != a {
                        assert!(!state.wizards[b].spells.contains(id));
                        let other = serde_json::to_string(
                            &WizardsDuel.phone_view(&state, b as SeatId).unwrap(),
                        )
                        .unwrap();
                        assert!(!other.contains(&name), "seat {b} sees {name}");
                    }
                }
            }
        }
    }
    // Throughout real games, wiping everyone else's secrets changes
    // nothing on a seat's phone, and wiping all of them changes nothing on
    // the table. No event mentions a claim.
    for seed in 0..6u64 {
        let n = 2 + (seed % 3) as usize;
        let mut state = setup(n, seed);
        let mut rng = Rng::new(seed);
        while !state.over {
            let table = WizardsDuel.table_view(&state).unwrap();
            assert_eq!(
                table,
                WizardsDuel
                    .table_view(&without_secrets(&state, None))
                    .unwrap()
            );
            for seat in 0..n {
                let phone = WizardsDuel.phone_view(&state, seat as SeatId).unwrap();
                let wiped = without_secrets(&state, Some(seat));
                assert_eq!(
                    phone,
                    WizardsDuel.phone_view(&wiped, seat as SeatId).unwrap()
                );
            }
            let seat = state.current as SeatId;
            let ui = WizardsDuel.phone_view(&state, seat).unwrap();
            let input = bot::choose(&ui, &mut rng).unwrap();
            let out = WizardsDuel.on_action(state, seat, &input).unwrap();
            for e in &out.events {
                let text = serde_json::to_string(e).unwrap();
                assert!(!text.contains("Claimed"), "{text}");
            }
            state = out.state;
        }
    }
}

#[test]
fn games_replay_exactly_from_seed_and_log() {
    for (n, seed) in [(2, 3), (3, 8), (4, 21)] {
        let (room, _) = common::play(n, seed, 1000);
        let log: Vec<LoggedAction> = room.log().to_vec();
        let replayed = replay(&WizardsDuel, &common::seats(n), seed, &log).unwrap();
        let played: &State = room.state().unwrap();
        assert_eq!(&replayed, played);
        assert_eq!(
            serde_json::to_string(&replayed).unwrap(),
            serde_json::to_string(played).unwrap()
        );
        // And the same seed sets up the same duel.
        assert_eq!(setup(n, seed), setup(n, seed));
    }
}

#[test]
fn setups_differ_by_seed_but_are_fair() {
    let a = setup(4, 1);
    let b = setup(4, 2);
    assert_ne!(
        (a.wizards.clone(), a.current),
        (b.wizards.clone(), b.current)
    );
    for n in 2..=4 {
        let s = setup(n, 9);
        assert_eq!(s.wizards.len(), n);
        let game = s.game();
        for w in &s.wizards {
            assert_eq!(w.spells.len(), arena::FAMILIES.len());
            assert_eq!(w.hp, hostable_wizards_duel::DUEL_HP);
            assert!(arena::start_corners().contains(&w.at));
        }
        // Everyone starts equally far from their nearest rival.
        let nearest: Vec<u32> = s
            .wizards
            .iter()
            .map(|w| {
                s.wizards
                    .iter()
                    .filter(|o| o.at != w.at)
                    .filter_map(|o| game.board.distance(w.at, o.at))
                    .min()
                    .unwrap()
            })
            .collect();
        assert!(nearest.iter().all(|d| *d == nearest[0]), "{nearest:?}");
    }
}

#[test]
fn timeouts_alone_keep_the_duel_going() {
    for pace in [Pace::Live, Pace::Async] {
        let mut room = common::room_with_pace(3, 4, pace);
        let step = match pace {
            Pace::Live => TURN_LIVE_MS,
            Pace::Async => hostable_wizards_duel::TURN_ASYNC_MS,
        };
        let first: &State = room.state().unwrap();
        let (start_seat, start_turn) = (first.current, first.turn);
        let mut seen = std::collections::BTreeSet::new();
        let mut now = 0;
        for _ in 0..9 {
            now += step + 1;
            room.tick(now);
            assert!(room.paused().is_none());
            assert!(room.finished().is_none());
            seen.insert(room.state().unwrap().current);
        }
        let state: &State = room.state().unwrap();
        assert_eq!(seen.len(), 3, "every seat got a turn");
        assert_eq!(state.current, start_seat);
        assert_eq!(state.turn, start_turn + 3);
        let timeouts = room
            .log()
            .iter()
            .filter(|a| matches!(a, LoggedAction::Timeout { .. }))
            .count();
        assert_eq!(timeouts, 9);
        assert!(state.log.iter().any(|l| l.ends_with("ran out of time.")));
    }
    // A stale timeout for a seat whose turn it isn't changes nothing.
    let state = setup(2, 1);
    let other = (state.current as SeatId + 1) % 2;
    let out = WizardsDuel.on_timeout(state.clone(), other, 5).unwrap();
    assert_eq!(out.state, state);
    assert_eq!(out.waiting_on.len(), 1);
}

#[test]
fn screens_show_controls_only_to_the_wizard_whose_turn_it_is() {
    let state = setup(3, 2);
    for seat in 0..3 {
        let ui = WizardsDuel.phone_view(&state, seat).unwrap();
        if seat as usize == state.current {
            assert!(ui.has_input("end") && ui.has_input("move"));
        } else {
            assert!(ui.inputs().is_empty(), "{ui:?}");
        }
    }
    assert!(WizardsDuel
        .phone_view(&state, 7)
        .unwrap()
        .inputs()
        .is_empty());
    // Moves are labelled by compass direction and say where rivals are.
    let ui = WizardsDuel
        .phone_view(&state, state.current as SeatId)
        .unwrap();
    let Some(Widget::Choice { options, .. }) = ui.find_input("move") else {
        panic!("no move choice");
    };
    for o in options {
        assert!(o.label.starts_with("Step "), "{}", o.label);
        assert!(o.detail.contains(" away)"), "{}", o.detail);
    }
}

#[test]
fn the_table_has_a_scene_and_renderer_data() {
    let state = setup(4, 6);
    let table: TableView = serde_json::from_value(WizardsDuel.table_view(&state).unwrap()).unwrap();
    let scene = table.scene.unwrap();
    let hexes = scene
        .shapes
        .iter()
        .filter(|s| matches!(s, Shape::Hex { .. }))
        .count();
    assert_eq!(hexes, 37);
    for name in common::NAMES {
        assert!(scene.shapes.iter().any(|s| s.label() == name));
    }
    assert_eq!(table.players.len(), 4);
    assert!(table.players[state.current].active);
    let extra = &table.extra;
    assert_eq!(extra["board"]["hexes"].as_array().unwrap().len(), 37);
    assert_eq!(extra["wizards"].as_array().unwrap().len(), 4);
    assert_eq!(extra["current"], json!(state.current));
    assert_eq!(
        extra["wizards"][0]["hp"],
        json!(hostable_wizards_duel::DUEL_HP)
    );
    let board: wizards_rules::BoardDef = serde_json::from_value(extra["board"].clone()).unwrap();
    assert_eq!(board, state.board);
    let at: Vertex = serde_json::from_value(extra["wizards"][1]["at"].clone()).unwrap();
    assert_eq!(at, state.wizards[1].at);
}
