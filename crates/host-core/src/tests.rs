use super::*;
use hostable_buzzer::{Buzzer, Phase, State as BuzzState};
use hostable_protocol::{ButtonStyle, HostToPhone};
use hostable_rules_api::{Json, RulesResult};
use serde_json::json;

fn sent_to(effects: &[Effect]) -> Vec<PlayerId> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::Send(HostToRelay::ToPlayer {
                player,
                message: HostToPhone::Ui { .. },
            }) => Some(*player),
            _ => None,
        })
        .collect()
}

fn ui_for(effects: &[Effect], who: PlayerId) -> Option<Widget> {
    effects.iter().rev().find_map(|e| match e {
        Effect::Send(HostToRelay::ToPlayer {
            player,
            message: HostToPhone::Ui { ui, .. },
        }) if *player == who => Some(ui.clone()),
        _ => None,
    })
}

fn room_with(names: &[&str]) -> Room<Buzzer> {
    let mut room = Room::new(Buzzer, 7);
    for (i, name) in names.iter().enumerate() {
        room.player_joined(100 + i as PlayerId, name.to_string(), String::new(), 0);
    }
    room
}

#[test]
fn joining_shows_the_lobby_and_start_needs_enough_players() {
    let mut room = Room::new(Buzzer, 7);
    let effects = room.player_joined(100, "Ana".into(), "#E4572E".into(), 0);
    assert_eq!(sent_to(&effects), vec![100]);
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::Send(HostToRelay::ToPlayer {
            message: HostToPhone::Seat { seat: Some(0), .. },
            ..
        })
    )));
    assert_eq!(room.seats()[0].color, "#e4572e");
    assert_eq!(
        room.start(0),
        Err(RoomError::TooFewPlayers { have: 1, need: 2 })
    );
    room.player_joined(101, "Bo".into(), "red; drop".into(), 0);
    assert_eq!(room.seats()[1].color, "");
    let effects = room.start(0).unwrap();
    assert_eq!(sent_to(&effects), vec![100, 101]);
    assert_eq!(room.start(0), Err(RoomError::AlreadyStarted));
}

#[test]
fn late_joiners_become_spectators_who_see_no_controls() {
    let mut room = room_with(&["Ana", "Bo"]);
    room.start(0).unwrap();
    let effects = room.player_joined(102, "Cy".into(), String::new(), 0);
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::Send(HostToRelay::ToPlayer {
            player: 102,
            message: HostToPhone::Seat { seat: None, .. }
        })
    )));
    let ui = ui_for(&effects, 102).unwrap();
    assert!(ui.inputs().is_empty());
    assert_eq!(room.seats().len(), 2);
    // Their buzz goes nowhere.
    let effects = room
        .handle_input(102, PhoneInput::press("buzz"), 0)
        .unwrap();
    assert!(effects.is_empty());
    assert!(room.log().is_empty());
}

#[test]
fn input_runs_the_rules_and_only_changed_screens_are_sent() {
    let mut room = room_with(&["Ana", "Bo"]);
    room.start(0).unwrap();
    let effects = room
        .handle_input(101, PhoneInput::press("buzz"), 0)
        .unwrap();
    assert_eq!(room.state().unwrap().phase, Phase::Buzzed { by: 1 });
    assert!(effects
        .iter()
        .any(|e| matches!(e, Effect::Event(Event::Sound { .. }))));
    // Bo's phone buzzes too.
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::Send(HostToRelay::ToPlayer {
            player: 101,
            message: HostToPhone::Feedback { .. }
        })
    )));
    assert_eq!(sent_to(&effects), vec![100, 101]);
    assert_eq!(room.log().len(), 1);
}

#[test]
fn input_for_a_widget_not_on_screen_is_dropped() {
    let mut room = room_with(&["Ana", "Bo"]);
    room.start(0).unwrap();
    // Bo isn't the leader, so his screen never has a Next round button.
    let effects = room
        .handle_input(101, PhoneInput::press("next_round"), 0)
        .unwrap();
    assert!(effects.is_empty());
    assert!(room.log().is_empty());
    // A button press with a smuggled value is dropped too.
    room.handle_input(101, PhoneInput::with("buzz", "extra"), 0)
        .unwrap();
    assert!(room.log().is_empty());
}

#[test]
fn a_returning_player_gets_their_screen_again() {
    let mut room = room_with(&["Ana", "Bo"]);
    room.start(0).unwrap();
    room.player_away(101);
    let effects = room
        .handle_input(100, PhoneInput::press("buzz"), 0)
        .unwrap();
    assert_eq!(sent_to(&effects), vec![100]);
    let effects = room.player_returned(101, 0).unwrap();
    assert_eq!(sent_to(&effects), vec![101]);
}

#[test]
fn a_snapshot_survives_a_host_restart() {
    let mut room = room_with(&["Ana", "Bo"]);
    room.start(0).unwrap();
    room.handle_input(101, PhoneInput::press("buzz"), 0)
        .unwrap();
    let saved = serde_json::to_string(&room.snapshot()).unwrap();

    let mut room = Room::restore(Buzzer, serde_json::from_str(&saved).unwrap());
    assert_eq!(room.state().unwrap().scores, vec![0, 1]);
    assert_eq!(room.log().len(), 1);
    let effects = room.player_returned(100, 0).unwrap();
    assert_eq!(sent_to(&effects), vec![100]);
    room.handle_input(100, PhoneInput::press("next_round"), 0)
        .unwrap();
    assert_eq!(room.state().unwrap().round, 2);
}

#[test]
fn a_game_replays_from_its_seed_and_log() {
    let mut room = room_with(&["Ana", "Bo"]);
    room.start(0).unwrap();
    for _ in 0..3 {
        room.handle_input(101, PhoneInput::press("buzz"), 0)
            .unwrap();
        room.handle_input(100, PhoneInput::press("next_round"), 0)
            .unwrap();
    }
    let replayed: BuzzState = replay(&Buzzer, &room.seats(), 7, room.log()).unwrap();
    assert_eq!(&replayed, room.state().unwrap());
}

#[test]
fn table_view_goes_only_to_phones_that_ask() {
    let mut room = room_with(&["Ana", "Bo"]);
    room.start(0).unwrap();
    let effects = room
        .handle_input(100, PhoneInput::with(INPUT_WANT_TABLE, true), 0)
        .unwrap();
    let tables: Vec<PlayerId> = effects
        .iter()
        .filter_map(|e| match e {
            Effect::Send(HostToRelay::ToPlayer {
                player,
                message: HostToPhone::Table { .. },
            }) => Some(*player),
            _ => None,
        })
        .collect();
    assert_eq!(tables, vec![100]);
    // The table is resent only when it changes.
    let effects = room
        .handle_input(100, PhoneInput::with(INPUT_AUTO_PASS, true), 0)
        .unwrap();
    assert!(!effects.iter().any(|e| matches!(
        e,
        Effect::Send(HostToRelay::ToPlayer {
            message: HostToPhone::Table { .. },
            ..
        })
    )));
}

#[test]
fn push_subscriptions_are_checked_and_reported() {
    let mut room = room_with(&["Ana", "Bo"]);
    let bad = room
        .handle_input(
            100,
            PhoneInput::with(INPUT_PUSH, json!({"endpoint": "http://x"})),
            0,
        )
        .unwrap();
    assert!(!bad
        .iter()
        .any(|e| matches!(e, Effect::PushSubscribed { .. })));
    let sub = json!({"endpoint": "https://push.example/abc", "keys": {"p256dh": "k", "auth": "a"}});
    let good = room
        .handle_input(100, PhoneInput::with(INPUT_PUSH, sub.clone()), 0)
        .unwrap();
    assert!(good.iter().any(|e| *e
        == Effect::PushSubscribed {
            player: 100,
            subscription: sub.clone()
        }));
    assert_eq!(room.push_subscription(100), Some(&sub));
}

// ---------------------------------------------------------------------------
// Reaction windows
// ---------------------------------------------------------------------------

/// Seat 0 casts; every other seat may respond (or pass) within a window.
/// Seats holding no counters can't respond. Responses nest: a response
/// opens a window for everyone else. Turn timer on seat 0's turn.
#[derive(Default)]
struct Reactor;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct RState {
    counters: Vec<u32>,
    stack: Vec<SeatId>,
    resolved: Vec<Vec<SeatId>>,
    passed: Vec<SeatId>,
    timeouts: u32,
    seats: u32,
}

impl RState {
    fn windows(&self) -> Vec<Wait> {
        if self.stack.is_empty() {
            return vec![Wait::turn(0, 30_000, 86_400_000)];
        }
        (0..self.seats)
            .filter(|s| *s != *self.stack.last().unwrap() && !self.passed.contains(s))
            .map(|s| Wait::reaction(s, "Respond?", 5_000, 3_600_000, PhoneInput::press("pass")))
            .collect()
    }
}

impl Rules for Reactor {
    type State = RState;
    fn seats(&self) -> (u32, u32) {
        (2, 4)
    }
    fn setup(&self, seats: &[Seat], _: u64) -> RulesResult<Outcome<RState>> {
        let s = RState {
            counters: vec![1; seats.len()],
            stack: vec![],
            resolved: vec![],
            passed: vec![],
            timeouts: 0,
            seats: seats.len() as u32,
        };
        let w = s.windows();
        let mut o = Outcome::new(s);
        o.waiting_on = w;
        Ok(o)
    }
    fn on_action(
        &self,
        mut s: RState,
        seat: SeatId,
        input: &PhoneInput,
    ) -> RulesResult<Outcome<RState>> {
        match input.widget.as_str() {
            "cast" if s.stack.is_empty() && seat == 0 => s.stack.push(0),
            "respond" if !s.stack.is_empty() && s.counters[seat as usize] > 0 => {
                s.counters[seat as usize] -= 1;
                s.stack.push(seat);
                s.passed.clear();
            }
            "pass" if !s.stack.is_empty() => {
                s.passed.push(seat);
                let everyone_else_passed = (0..s.seats)
                    .filter(|x| x != s.stack.last().unwrap())
                    .all(|x| s.passed.contains(&x));
                if everyone_else_passed {
                    s.resolved.push(std::mem::take(&mut s.stack));
                    s.passed.clear();
                }
            }
            "fail" => return Err(RulesError::Crashed("boom".into())),
            _ => {}
        }
        let w = s.windows();
        let mut o = Outcome::new(s);
        o.waiting_on = w;
        Ok(o)
    }
    fn on_timeout(&self, mut s: RState, _: SeatId, _: u64) -> RulesResult<Outcome<RState>> {
        s.timeouts += 1;
        let w = s.windows();
        let mut o = Outcome::new(s);
        o.waiting_on = w;
        Ok(o)
    }
    fn table_view(&self, s: &RState) -> RulesResult<Value> {
        let mut t = TableView::new("Reactor");
        t.status = format!("stack {:?}", s.stack);
        Ok(t.to_value())
    }
    fn phone_view(&self, s: &RState, seat: SeatId) -> RulesResult<Widget> {
        let mut c = vec![Widget::button("fail", "Fail", ButtonStyle::Danger)];
        if s.stack.is_empty() && seat == 0 {
            c.push(Widget::button("cast", "Cast", ButtonStyle::Primary));
        }
        if !s.stack.is_empty() && *s.stack.last().unwrap() != seat && !s.passed.contains(&seat) {
            if s.counters[seat as usize] > 0 {
                c.push(Widget::button("respond", "Respond", ButtonStyle::Primary));
            }
            c.push(Widget::button("pass", "Pass", ButtonStyle::Normal));
        }
        Ok(Widget::column(c))
    }
    fn can_respond(&self, s: &RState, seat: SeatId) -> RulesResult<bool> {
        Ok(s.counters[seat as usize] > 0)
    }
}

fn reactor(n: usize, pace: Pace) -> Room<Reactor> {
    let mut room = Room::new(Reactor, 1).with_pace(pace);
    for i in 0..n {
        room.player_joined(i as PlayerId, format!("P{i}"), String::new(), 0);
    }
    room.start(0).unwrap();
    room
}

fn press(room: &mut Room<Reactor>, player: PlayerId, w: &str, now: u64) -> Vec<Effect> {
    room.handle_input(player, PhoneInput::press(w), now)
        .unwrap()
}

#[test]
fn waiting_seats_see_their_prompt_and_a_countdown() {
    let mut room = reactor(2, Pace::Live);
    let effects = press(&mut room, 0, "cast", 1_000);
    let ui = ui_for(&effects, 1).unwrap();
    let Widget::Column { children } = &ui else {
        panic!("{ui:?}")
    };
    assert_eq!(
        children[0],
        Widget::Timer {
            ends_in_ms: 5_000,
            total_ms: Some(5_000),
            label: "Respond?".into()
        }
    );
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::Notify {
            player: 1,
            seat: 1,
            ..
        }
    )));
    assert_eq!(room.next_deadline(), Some(6_000));
}

#[test]
fn a_reaction_window_times_out_into_its_pass_input() {
    let mut room = reactor(2, Pace::Live);
    press(&mut room, 0, "cast", 0);
    assert!(room
        .tick(4_999)
        .iter()
        .all(|e| !matches!(e, Effect::Event(_))));
    assert_eq!(room.state().unwrap().stack, vec![0]);
    room.tick(5_000);
    assert_eq!(room.state().unwrap().resolved, vec![vec![0]]);
    assert!(matches!(
        room.log().last(),
        Some(LoggedAction::Passed { seat: 1, .. })
    ));
    // Back to a turn timer on seat 0, counted from the timeout.
    assert_eq!(room.next_deadline(), Some(35_000));
}

#[test]
fn turn_timers_call_on_timeout_and_async_uses_the_long_deadline() {
    let mut room = reactor(2, Pace::Async);
    assert_eq!(room.next_deadline(), Some(86_400_000));
    room.tick(86_400_000 - 1);
    assert_eq!(room.state().unwrap().timeouts, 0);
    room.tick(86_400_000);
    assert_eq!(room.state().unwrap().timeouts, 1);
    assert!(matches!(
        room.log().last(),
        Some(LoggedAction::Timeout { seat: 0, .. })
    ));
}

#[test]
fn nested_responses_resolve_as_one_chain() {
    let mut room = reactor(3, Pace::Live);
    press(&mut room, 0, "cast", 0);
    press(&mut room, 1, "respond", 100);
    // Now seats 0 and 2 may respond to seat 1's response.
    let seats: Vec<SeatId> = room.waits().iter().map(|w| w.wait.seat).collect();
    assert_eq!(seats, vec![0, 2]);
    press(&mut room, 2, "respond", 200);
    assert_eq!(room.state().unwrap().stack, vec![0, 1, 2]);
    press(&mut room, 0, "pass", 300);
    press(&mut room, 1, "pass", 400);
    assert_eq!(room.state().unwrap().resolved, vec![vec![0, 1, 2]]);
}

#[test]
fn input_answers_in_any_order_and_keeps_other_deadlines() {
    let mut room = reactor(3, Pace::Live);
    press(&mut room, 0, "cast", 0);
    press(&mut room, 2, "pass", 1_000);
    // Seat 1 is still on its original deadline.
    assert_eq!(room.waits().len(), 1);
    assert_eq!(room.waits()[0].deadline_ms, 5_000);
    press(&mut room, 1, "pass", 1_500);
    assert_eq!(room.state().unwrap().resolved, vec![vec![0]]);
}

#[test]
fn async_games_pass_at_once_for_seats_that_cannot_respond() {
    let mut room = reactor(3, Pace::Async);
    press(&mut room, 0, "cast", 0);
    press(&mut room, 1, "respond", 10);
    press(&mut room, 2, "respond", 20);
    // Seat 1 and 2 spent their only counter; seat 0 still has one.
    press(&mut room, 0, "respond", 30);
    // Seats 1 and 2 can't respond to seat 0 now, so both passed on their own.
    assert_eq!(room.state().unwrap().resolved, vec![vec![0, 1, 2, 0]]);
    let passes = room
        .log()
        .iter()
        .filter(|a| matches!(a, LoggedAction::Passed { .. }))
        .count();
    assert_eq!(passes, 3);
}

#[test]
fn live_players_can_opt_into_auto_pass() {
    let mut room = reactor(2, Pace::Live);
    press(&mut room, 0, "cast", 0);
    press(&mut room, 1, "respond", 10);
    press(&mut room, 0, "respond", 20);
    // Seat 1 has no counters left but hasn't opted in, so we wait.
    assert_eq!(room.state().unwrap().stack, vec![0, 1, 0]);
    room.handle_input(1, PhoneInput::with(INPUT_AUTO_PASS, true), 30)
        .unwrap();
    assert_eq!(room.state().unwrap().resolved, vec![vec![0, 1, 0]]);
}

#[test]
fn a_failing_call_pauses_the_game_without_losing_state() {
    let mut room = reactor(2, Pace::Live);
    press(&mut room, 0, "cast", 0);
    let before = room.state().unwrap().clone();
    let effects = press(&mut room, 1, "fail", 10);
    assert!(effects
        .iter()
        .any(|e| matches!(e, Effect::Paused(m) if m.contains("boom"))));
    assert_eq!(room.state().unwrap(), &before);
    assert!(room.paused().is_some());
    // While paused, nothing happens and deadlines don't fire.
    press(&mut room, 1, "pass", 20);
    room.tick(1_000_000);
    assert_eq!(room.state().unwrap(), &before);
    let ui = room.current_ui(0).unwrap();
    assert!(ui.inputs().is_empty());
    room.resume();
    press(&mut room, 1, "pass", 30);
    assert_eq!(room.state().unwrap().resolved, vec![vec![0]]);
}

#[test]
fn waits_survive_a_restart_with_their_deadlines() {
    let mut room = reactor(2, Pace::Live);
    press(&mut room, 0, "cast", 1_000);
    let snap = serde_json::to_string(&room.snapshot()).unwrap();
    let mut room = Room::restore(Reactor, serde_json::from_str(&snap).unwrap());
    assert_eq!(room.next_deadline(), Some(6_000));
    room.tick(6_000);
    assert_eq!(room.state().unwrap().resolved, vec![vec![0]]);
}

#[test]
fn outcomes_naming_missing_seats_are_rejected() {
    struct Liar;
    impl Rules for Liar {
        type State = u8;
        fn seats(&self) -> (u32, u32) {
            (2, 2)
        }
        fn setup(&self, _: &[Seat], _: u64) -> RulesResult<Outcome<u8>> {
            Ok(Outcome::new(0))
        }
        fn on_action(&self, s: u8, _: SeatId, _: &PhoneInput) -> RulesResult<Outcome<u8>> {
            Ok(Outcome::new(s).waiting(Wait::turn(9, 1, 1)))
        }
        fn table_view(&self, _: &u8) -> RulesResult<Value> {
            Ok(TableView::new("x").to_value())
        }
        fn phone_view(&self, _: &u8, _: SeatId) -> RulesResult<Widget> {
            Ok(Widget::button("go", "Go", ButtonStyle::Normal))
        }
    }
    let mut room = Room::new(Liar, 0);
    room.player_joined(1, "A".into(), String::new(), 0);
    room.player_joined(2, "B".into(), String::new(), 0);
    room.start(0).unwrap();
    let effects = room.handle_input(1, PhoneInput::press("go"), 0).unwrap();
    assert!(effects
        .iter()
        .any(|e| matches!(e, Effect::Paused(m) if m.contains("seat 9"))));
}

#[test]
fn json_rules_run_in_a_dynamic_room() {
    let mut room: Room<Box<DynRulesBox>> = Room::new(Box::new(Json(Buzzer)), 3);
    room.player_joined(1, "A".into(), String::new(), 0);
    room.player_joined(2, "B".into(), String::new(), 0);
    room.start(0).unwrap();
    room.handle_input(2, PhoneInput::press("buzz"), 0).unwrap();
    assert_eq!(room.state().unwrap()["scores"], json!([0, 1]));
    assert!(check_table(&room.table_view().unwrap().unwrap()).is_ok());
}

type DynRulesBox = hostable_rules_api::DynRules;

#[test]
fn a_game_turn_timer_overrides_turns_and_caps_reactions() {
    let mut room = Room::new(Reactor, 1)
        .with_pace(Pace::Async)
        .with_turn_timer(Some(1_000));
    room.player_joined(0, "A".into(), String::new(), 0);
    room.player_joined(1, "B".into(), String::new(), 0);
    room.start(0).unwrap();
    assert_eq!(room.next_deadline(), Some(1_000));
    room.tick(1_000);
    assert_eq!(room.state().unwrap().timeouts, 1);
    press(&mut room, 0, "cast", 1_500);
    // The reaction window's hour-long async timeout is capped too.
    assert_eq!(room.next_deadline(), Some(2_500));
    let snap = serde_json::to_string(&room.snapshot()).unwrap();
    let room = Room::restore(Reactor, serde_json::from_str(&snap).unwrap());
    assert_eq!(room.turn_timer(), Some(1_000));
}
