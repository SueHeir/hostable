use super::*;
use hostable_host_core::{bot, Room};
use serde_json::json;

fn def() -> TableDef {
    serde_json::from_value(json!({
        "title": "Test table",
        "width": 800, "height": 600,
        "deal": 2,
        "objects": [
            {"kind": "board", "id": "felt", "x": 0, "y": 0, "w": 800, "h": 600},
            {"kind": "deck", "id": "deck", "x": 50, "y": 50, "cards": [
                {"id": "a", "face": "A"}, {"id": "b", "face": "B"}, {"id": "c", "face": "C"},
                {"id": "d", "face": "D"}, {"id": "e", "face": "E"}, {"id": "f", "face": "F"}
            ]},
            {"kind": "zone", "id": "pile", "x": 300, "y": 50, "w": 120, "h": 160, "label": "Pile"},
            {"kind": "die", "id": "d6", "x": 600, "y": 50, "sides": 6},
            {"kind": "counter", "id": "score", "x": 600, "y": 200, "label": "Score"},
            {"kind": "token", "id": "chip", "x": 600, "y": 300, "color": "#ff0000"}
        ]
    }))
    .unwrap()
}

fn game() -> (Tabletop, State) {
    let t = Tabletop::new(def(), (1, 4)).unwrap();
    let s = t
        .setup(&[Seat::new(0, "Ana"), Seat::new(1, "Bo")], 3)
        .unwrap()
        .state;
    (t, s)
}

fn act(t: &Tabletop, s: State, seat: SeatId, w: &str, v: Value) -> State {
    t.on_action(s, seat, &PhoneInput::with(w, v)).unwrap().state
}

fn deck_len(s: &State) -> usize {
    match &s.objects[1].thing {
        Thing::Pile { cards, .. } => cards.len(),
        _ => panic!(),
    }
}

#[test]
fn setup_deals_hands_from_the_shuffled_deck() {
    let (_, s) = game();
    assert_eq!(s.players[0].hand.len(), 2);
    assert_eq!(s.players[1].hand.len(), 2);
    assert_eq!(deck_len(&s), 2);
}

#[test]
fn hands_are_private() {
    let (t, s) = game();
    let table = t.table_view(&s).unwrap().to_string();
    let bo_view = serde_json::to_string(&t.phone_view(&s, 1).unwrap()).unwrap();
    for &c in &s.players[0].hand {
        let face = format!("\"{}\"", s.cards[c].face);
        assert!(!table.contains(&face), "table shows {face}");
        assert!(!bo_view.contains(&face), "Bo sees Ana's {face}");
    }
}

#[test]
fn the_touchpad_moves_the_cursor_and_tap_picks_up() {
    let (t, mut s) = game();
    s.players[0].cursor = (60.0, 60.0);
    let s = act(&t, s, 0, "pad", json!({"tap": true}));
    assert_eq!(s.players[0].holding, Some(1));
    let s = act(&t, s, 0, "pad", json!({"dx": 1.0, "dy": 0.0}));
    assert!((s.players[0].cursor.0 - (60.0 + 800.0 * CURSOR_SPEED)).abs() < 0.01);
    let mut s = s;
    for _ in 0..5 {
        s = act(&t, s, 0, "pad", json!({"dx": 1.0, "dy": 1.0}));
    }
    assert_eq!(s.players[0].cursor, (800.0, 600.0));
}

#[test]
fn cards_go_from_deck_to_hand_to_table_to_zone() {
    let (t, s) = game();
    let s = act(&t, s, 0, "pick", json!("deck"));
    let s = act(&t, s, 0, "draw", Value::Null);
    assert_eq!(s.players[0].hand.len(), 3);
    // Play a card face up onto the empty table.
    let mut s = s;
    s.players[0].cursor = (200.0, 400.0);
    let card = s.players[0].hand[0];
    let s = act(&t, s, 0, "play_up", json!(card.to_string()));
    let last = s.objects.last().unwrap();
    assert!(matches!(&last.thing, Thing::Card(c) if c.card == card && c.face_up));
    // Pick it up and move it into the pile zone.
    let id = last.id.clone();
    let s = act(&t, s, 0, "pick", json!(id));
    let mut s = s;
    s.players[0].cursor = (350.0, 100.0);
    let s = act(&t, s, 0, "move_here", Value::Null);
    assert!(matches!(&s.objects[2].thing, Thing::Pile { cards, .. } if cards.len() == 1));
    assert!(s.objects.iter().all(|o| o.id != id));
}

#[test]
fn dice_counters_flips_and_shuffles() {
    let (t, s) = game();
    let s = act(&t, s, 1, "pick", json!("d6"));
    let s = act(&t, s, 1, "roll", Value::Null);
    assert!(matches!(s.objects[3].thing, Thing::Die { value, .. } if (1..=6).contains(&value)));
    let s = act(&t, s, 1, "pick", json!("score"));
    let s = act(&t, s, 1, "plus", Value::Null);
    let s = act(&t, s, 1, "plus", Value::Null);
    let s = act(&t, s, 1, "minus", Value::Null);
    assert!(matches!(
        s.objects[4].thing,
        Thing::Counter { value: 1, .. }
    ));
    let s = act(&t, s, 1, "pick", json!("deck"));
    let s = act(&t, s, 1, "flip", Value::Null);
    assert!(
        matches!(&s.objects[1].thing, Thing::Pile { cards, .. } if cards.iter().all(|c| c.face_up))
    );
    let s = act(&t, s, 1, "shuffle", Value::Null);
    assert!(
        matches!(&s.objects[1].thing, Thing::Pile { cards, .. } if cards.iter().all(|c| !c.face_up))
    );
    let s = act(&t, s, 1, "deal", Value::Null);
    assert_eq!(s.players[0].hand.len(), 3);
}

#[test]
fn bad_tables_are_refused() {
    let mut d = def();
    d.objects.push(d.objects[0].clone());
    assert!(Tabletop::new(d, (1, 2)).is_err());
    let mut d = def();
    d.width = -1.0;
    assert!(Tabletop::new(d, (1, 2)).is_err());
    assert!(Tabletop::from_json(b"{}", (1, 2)).is_err());
}

#[test]
fn bots_can_play_freely_without_breaking_anything() {
    for seed in 0..10 {
        let mut room = Room::new(Tabletop::new(def(), (1, 4)).unwrap(), seed);
        for p in 1..=3 {
            room.player_joined(p, format!("P{p}"), String::new(), 0);
        }
        room.start(0).unwrap();
        let mut rng = Rng::new(seed);
        for step in 0..1500u64 {
            let p = 1 + rng.below(3);
            let Some(ui) = room.current_ui(p).cloned() else {
                continue;
            };
            let Some(input) = bot::random_input(&ui, &mut rng, &[]) else {
                continue;
            };
            if input.widget == "end_game" && step < 1400 {
                continue;
            }
            room.handle_input(p, input, step).unwrap();
            assert!(room.paused().is_none(), "{:?}", room.paused());
            let s = room.state().unwrap();
            let total: usize = s.players.iter().map(|p| p.hand.len()).sum::<usize>()
                + s.objects
                    .iter()
                    .map(|o| match &o.thing {
                        Thing::Pile { cards, .. } => cards.len(),
                        Thing::Card(_) => 1,
                        _ => 0,
                    })
                    .sum::<usize>();
            assert_eq!(total, 6, "cards appeared or vanished");
            hostable_host_core::check_table(&room.table_view().unwrap().unwrap()).unwrap();
        }
    }
}
