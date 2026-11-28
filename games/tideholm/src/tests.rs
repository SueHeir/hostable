use super::*;
use crate::board::topo;
use hostable_host_core::{bot::random_input, check_table, replay, Effect, Room, MAX_UI_NODES};
use hostable_sdk::rules_api::Json;

type PlayerId = u32;

fn seats(n: usize) -> Vec<Seat> {
    ["Ana", "Bo", "Cy", "Dee"][..n]
        .iter()
        .enumerate()
        .map(|(i, name)| Seat::new(i as SeatId, *name))
        .collect()
}

fn act(s: &State, seat: SeatId, input: PhoneInput) -> Outcome<State> {
    Tideholm.on_action(s.clone(), seat, &input).unwrap()
}

fn press(s: &State, seat: SeatId, id: &str) -> State {
    act(s, seat, PhoneInput::press(id)).state
}

fn pick(s: &State, seat: SeatId, id: &str, value: impl Into<Value>) -> State {
    act(s, seat, PhoneInput::with(id, value)).state
}

/// A game past setup with an empty board, in seat 0's main phase.
fn blank(n: usize) -> State {
    let mut s = new_game(&seats(n), 11);
    s.phase = Phase::Main;
    s.current = 0;
    s.turn = 1;
    s.dice = Some((3, 4));
    s
}

fn hut(s: &mut State, seat: SeatId, v: usize) {
    s.buildings[v] = Some(Building { seat, lodge: false });
    s.players[seat as usize].huts += 1;
}

fn road(s: &mut State, seat: SeatId, e: usize) {
    s.causeways[e] = Some(seat);
    s.players[seat as usize].causeways += 1;
}

fn total_cards(s: &State) -> [u32; 5] {
    std::array::from_fn(|r| {
        s.bank[r] as u32 + s.players.iter().map(|p| p.res[r] as u32).sum::<u32>()
    })
}

const CENTER: usize = 9;

#[test]
fn board_has_the_right_shape_and_unique_labels() {
    let t = topo();
    assert_eq!(t.hexes.len(), 19);
    assert_eq!(t.verts.len(), 54);
    assert_eq!(t.edges.len(), 72);
    assert_eq!(t.hexes[CENTER], (0, 0));
    let mut vl = t.vert_labels.clone();
    vl.sort();
    vl.dedup();
    assert_eq!(vl.len(), 54);
    let mut el = t.edge_labels.clone();
    el.sort();
    el.dedup();
    assert_eq!(el.len(), 72);
    let s = new_game(&seats(4), 5);
    let mut kinds = [0; 6];
    for tile in &s.tiles {
        kinds[tile.res.unwrap_or(5)] += 1;
        assert_eq!(tile.res.is_none(), tile.number == 0);
        assert_ne!(tile.number, 7);
    }
    assert_eq!(kinds, [4, 3, 4, 4, 3, 1]);
    assert_eq!(s.tiles[s.squall].res, None);
    assert_eq!(s.deck.len(), 23);
}

#[test]
fn setup_is_snake_order_and_the_second_hut_pays() {
    let mut s = Tideholm.setup(&seats(3), 3).unwrap().state;
    let mut order = Vec::new();
    while let Phase::Setup { hut, .. } = s.phase {
        let seat = s.setup_seat().unwrap();
        if hut.is_none() {
            order.push(seat);
            let v = (0..54).find(|&v| s.hut_spot_free(v)).unwrap();
            s = pick(&s, seat, "place_hut", v.to_string());
        } else {
            let e = setup_causeway_spots(&s)[0];
            // A causeway away from the hut is refused.
            let far = (0..72)
                .find(|e| !setup_causeway_spots(&s).contains(e))
                .unwrap();
            let same = pick(&s, seat, "place_causeway", far.to_string());
            assert_eq!(same, s);
            s = pick(&s, seat, "place_causeway", e.to_string());
        }
    }
    assert_eq!(order, vec![0, 1, 2, 2, 1, 0]);
    assert_eq!(s.phase, Phase::PreRoll);
    for (i, p) in s.players.iter().enumerate() {
        assert_eq!((p.huts, p.causeways), (2, 2), "seat {i}");
    }
    // Starting resources come from the second hut's land hexes.
    let given: u32 = s.players.iter().map(Player::total).sum();
    assert!(given > 0);
    assert_eq!(total_cards(&s), [19; 5]);
}

#[test]
fn production_pays_huts_one_lodges_two_and_the_squall_blocks() {
    let mut s = blank(3);
    let h = (0..19).find(|&h| s.tiles[h].res.is_some()).unwrap();
    let r = s.tiles[h].res.unwrap();
    let n = s.tiles[h].number;
    let v = topo().hex_verts[h];
    hut(&mut s, 0, v[0]);
    s.buildings[v[3]] = Some(Building {
        seat: 1,
        lodge: true,
    });
    s.squall = (h + 1) % 19;
    s.produce(n);
    assert!(s.players[0].res[r] >= 1);
    assert!(s.players[1].res[r] >= 2);
    let before = s.players.clone();
    s.squall = h;
    let other: u32 = (0..19)
        .filter(|&x| x != h && s.tiles[x].number == n)
        .map(|x| {
            topo().hex_verts[x]
                .iter()
                .filter(|&&vv| s.buildings[vv].is_some())
                .count() as u32
        })
        .sum();
    s.produce(n);
    if other == 0 {
        assert_eq!(s.players, before, "the Squall blocks the hex");
    }
}

fn roll_seven(s: &State) -> State {
    for seed in 0..10_000u64 {
        let mut t = s.clone();
        t.rng = Rng::new(seed);
        t.phase = Phase::PreRoll;
        t.dice = None;
        let out = Tideholm
            .on_action(t, 0, &PhoneInput::press("roll"))
            .unwrap();
        if out.state.dice.is_some_and(|(a, b)| a + b == 7) {
            return out.state;
        }
    }
    panic!("no seven");
}

#[test]
fn a_seven_makes_big_hands_discard_at_once_then_moves_the_squall() {
    let mut s = blank(3);
    s.players[0].res = [3, 3, 3, 0, 0]; // 9 -> discard 4
    s.players[1].res = [2, 2, 2, 2, 2]; // 10 -> discard 5
    s.players[2].res = [1, 1, 1, 1, 1]; // 5 -> safe
    for p in &s.players {
        for r in 0..5 {
            s.bank[r] -= p.res[r];
        }
    }
    let h = (0..19).find(|&h| h != s.squall).unwrap();
    hut(&mut s, 2, topo().hex_verts[h][0]);
    let s = roll_seven(&s);
    assert_eq!(
        s.phase,
        Phase::Discard {
            owed: vec![4, 5, 0]
        }
    );
    let waits = waits_for(&s);
    assert_eq!(waits.len(), 2);
    assert!(waits.iter().all(|w| w.reaction));
    assert!(Tideholm.can_respond(&s, 0).unwrap());
    assert!(!Tideholm.can_respond(&s, 2).unwrap());
    // The discard is a hand widget with min = max = owed.
    match find_input(&Tideholm.phone_view(&s, 1).unwrap(), "discard") {
        Some(Widget::Hand {
            min, max, cards, ..
        }) => {
            assert_eq!((*min, *max, cards.len()), (5, 5, 10))
        }
        other => panic!("{other:?}"),
    }
    // Wrong count is refused.
    let same = pick(&s, 1, "discard", json!(["reed:0", "clay:0"]));
    assert_eq!(same, s);
    // Seat 1 first, then seat 0 by timeout (random).
    let s = pick(
        &s,
        1,
        "discard",
        json!(["reed:0", "reed:1", "clay:0", "clay:1", "salt:0"]),
    );
    assert_eq!(s.players[1].res, [0, 0, 1, 2, 2]);
    assert!(matches!(s.phase, Phase::Discard { .. }));
    let s = Tideholm.on_timeout(s, 0, 0).unwrap().state;
    assert_eq!(s.players[0].total(), 5);
    assert_eq!(s.phase, Phase::MoveSquall { back: Back::Main });
    // Can't leave it where it is.
    let same = pick(&s, 0, "squall_hex", s.squall.to_string());
    assert_eq!(same, s);
    let before = s.players[2].total();
    let s = pick(&s, 0, "squall_hex", h.to_string());
    assert_eq!(s.squall, h);
    // Only seat 2 is beside it, so the card is taken at once.
    assert_eq!(s.players[2].total(), before - 1);
    assert_eq!(s.players[0].total(), 6);
    assert_eq!(s.phase, Phase::Main);
    assert_eq!(total_cards(&s), [19; 5]);
}

#[test]
fn robbing_with_two_neighbours_asks_who() {
    let mut s = blank(3);
    let h = (0..19).find(|&h| h != s.squall).unwrap();
    let v = topo().hex_verts[h];
    hut(&mut s, 1, v[0]);
    hut(&mut s, 2, v[3]);
    s.players[1].res = [1, 0, 0, 0, 0];
    s.players[2].res = [0, 1, 0, 0, 0];
    s.phase = Phase::MoveSquall { back: Back::Main };
    let s = pick(&s, 0, "squall_hex", h.to_string());
    assert!(matches!(s.phase, Phase::Steal { ref from, .. } if from == &vec![1, 2]));
    let same = pick(&s, 0, "steal_from", "0");
    assert_eq!(same, s);
    let s = pick(&s, 0, "steal_from", "2");
    assert_eq!(s.players[0].res, [0, 1, 0, 0, 0]);
    assert_eq!(s.players[2].total(), 0);
    assert_eq!(s.phase, Phase::Main);
}

fn find_input<'a>(ui: &'a Widget, id: &str) -> Option<&'a Widget> {
    ui.find_input(id)
}

#[test]
fn huts_follow_the_distance_rule_and_need_a_causeway() {
    let t = topo();
    let mut s = blank(3);
    let v = t.hex_verts[CENTER];
    let e = t.hex_edges[CENTER];
    hut(&mut s, 0, v[0]);
    road(&mut s, 0, e[0]); // v0 - v1
    road(&mut s, 0, e[1]); // v1 - v2
    s.players[0].res = [5, 5, 5, 5, 5];
    // Next to a building: refused.
    let same = pick(&s, 0, "build_hut", v[1].to_string());
    assert_eq!(same, s);
    // Not on a causeway: refused.
    let same = pick(&s, 0, "build_hut", v[4].to_string());
    assert_eq!(same, s);
    // Two steps away on a causeway: fine.
    let s2 = pick(&s, 0, "build_hut", v[2].to_string());
    assert_eq!(
        s2.buildings[v[2]],
        Some(Building {
            seat: 0,
            lodge: false
        })
    );
    assert_eq!(s2.players[0].res, [4, 4, 5, 4, 5]);
    // Causeways must connect; not through an opponent's building.
    let same = pick(&s, 0, "build_causeway", e[4].to_string());
    assert_eq!(same, s);
    let mut s3 = s.clone();
    hut(&mut s3, 1, v[2]);
    assert!(!s3.causeway_legal(0, e[2]));
    assert!(s.causeway_legal(0, e[2]));
    let s4 = pick(&s, 0, "build_causeway", e[2].to_string());
    assert_eq!(s4.causeways[e[2]], Some(0));
    // A lodge only upgrades your own hut.
    let same = pick(&s, 0, "build_lodge", v[2].to_string());
    assert_eq!(same, s);
    let s5 = pick(&s, 0, "build_lodge", v[0].to_string());
    assert_eq!(
        s5.buildings[v[0]],
        Some(Building {
            seat: 0,
            lodge: true
        })
    );
    assert_eq!((s5.players[0].huts, s5.players[0].lodges), (0, 1));
    assert_eq!(s5.visible_points(0), 2);
    // Unaffordable: refused, with a warning.
    let mut poor = s.clone();
    poor.players[0].res = [0; 5];
    let out = act(
        &poor,
        0,
        PhoneInput::with("build_causeway", e[2].to_string()),
    );
    assert_eq!(out.state, poor);
    assert!(out.events.iter().any(|ev| matches!(
        ev,
        Event::Feedback {
            seat: 0,
            kind: Feedback::Warning
        }
    )));
}

#[test]
fn longest_causeway_needs_five_and_can_be_taken_or_broken() {
    let t = topo();
    let mut s = blank(3);
    let v = t.hex_verts[CENTER];
    let e = t.hex_edges[CENTER];
    hut(&mut s, 0, v[0]);
    for &edge in &e[0..4] {
        road(&mut s, 0, edge);
    }
    s.update_longest();
    assert_eq!(s.players[0].causeway_len, 4);
    assert_eq!(s.longest, None);
    s.players[0].res = COST_CAUSEWAY;
    let s = pick(&s, 0, "build_causeway", e[4].to_string());
    assert_eq!(s.players[0].causeway_len, 5);
    assert_eq!(s.longest, Some(0));
    assert_eq!(s.visible_points(0), 3);
    // An opponent's hut in the middle breaks it.
    let mut b = s.clone();
    hut(&mut b, 1, v[2]);
    b.update_longest();
    assert_eq!(b.players[0].causeway_len, 3);
    assert_eq!(b.longest, None);
    // A longer one elsewhere takes it.
    let mut c = s.clone();
    let ring = &t.hex_edges[0];
    for &edge in ring.iter() {
        road(&mut c, 1, edge);
    }
    c.update_longest();
    assert_eq!(c.players[1].causeway_len, 6);
    assert_eq!(c.longest, Some(1));
}

fn play_ferryman(s: &State, seat: SeatId) -> State {
    let mut s = s.clone();
    s.current = seat;
    s.phase = Phase::Main;
    s.played_card = false;
    s.players[seat as usize].cards.push(Card::Ferryman);
    let s = pick(&s, seat, "play_card", "ferryman");
    assert_eq!(s.phase, Phase::MoveSquall { back: Back::Main });
    let to = (0..19).find(|&h| h != s.squall).unwrap();
    pick(&s, seat, "squall_hex", to.to_string())
}

#[test]
fn largest_fleet_goes_to_three_ferrymen_and_can_be_overtaken() {
    let mut s = blank(3);
    for _ in 0..2 {
        s = play_ferryman(&s, 0);
    }
    assert_eq!(s.fleet, None);
    s = play_ferryman(&s, 0);
    assert_eq!(s.players[0].ferrymen, 3);
    assert_eq!(s.fleet, Some(0));
    assert_eq!(s.visible_points(0), 2);
    for _ in 0..3 {
        s = play_ferryman(&s, 1);
    }
    assert_eq!(s.fleet, Some(0), "a tie doesn't take it");
    s = play_ferryman(&s, 1);
    assert_eq!(s.fleet, Some(1));
    // Only one card per turn, and not one bought this turn.
    let mut t = blank(3);
    t.players[0].cards = vec![Card::Ferryman, Card::Ferryman];
    let t = pick(&t, 0, "play_card", "ferryman");
    let t = pick(&t, 0, "squall_hex", ((t.squall + 1) % 19).to_string());
    let same = pick(&t, 0, "play_card", "ferryman");
    assert_eq!(same, t);
    let mut u = blank(3);
    u.players[0].fresh = vec![Card::Surveyor];
    let same = pick(&u, 0, "play_card", "surveyor");
    assert_eq!(same, u);
}

#[test]
fn ferryman_before_the_roll_returns_to_the_roll() {
    let mut s = blank(3);
    s.phase = Phase::PreRoll;
    s.dice = None;
    s.players[0].cards.push(Card::Ferryman);
    let s = pick(&s, 0, "play_card", "ferryman");
    assert_eq!(
        s.phase,
        Phase::MoveSquall {
            back: Back::PreRoll
        }
    );
    let s = pick(&s, 0, "squall_hex", ((s.squall + 1) % 19).to_string());
    assert_eq!(s.phase, Phase::PreRoll);
}

#[test]
fn surveyor_lays_two_free_causeways() {
    let t = topo();
    let mut s = blank(3);
    hut(&mut s, 0, t.hex_verts[CENTER][0]);
    s.players[0].cards.push(Card::Surveyor);
    let s = pick(&s, 0, "play_card", "surveyor");
    assert!(matches!(s.phase, Phase::Surveyor { left: 2, .. }));
    let e = t.hex_edges[CENTER];
    let s = pick(&s, 0, "surveyor_edge", e[0].to_string());
    let s = pick(&s, 0, "surveyor_edge", e[1].to_string());
    assert_eq!(s.causeways[e[0]], Some(0));
    assert_eq!(s.causeways[e[1]], Some(0));
    assert_eq!(s.players[0].res, [0; 5]);
    assert_eq!(s.phase, Phase::Main);
}

#[test]
fn bountiful_tide_takes_two_from_the_bank() {
    let mut s = blank(3);
    s.players[0].cards.push(Card::BountifulTide);
    let s = pick(&s, 0, "play_card", "bountiful_tide");
    assert!(matches!(s.phase, Phase::Bounty { .. }));
    let same = pick(&s, 0, "bounty", json!(["salt:1"]));
    assert_eq!(same, s);
    let s = pick(&s, 0, "bounty", json!(["salt:1", "salt:2"]));
    assert_eq!(s.players[0].res, [0, 0, 2, 0, 0]);
    assert_eq!(s.bank[2], 17);
    assert_eq!(s.phase, Phase::Main);
}

#[test]
fn market_day_collects_one_resource_from_everyone() {
    let mut s = blank(4);
    s.players[1].res = [0, 0, 0, 3, 1];
    s.players[2].res = [0, 0, 0, 2, 0];
    s.players[3].res = [1, 0, 0, 0, 0];
    s.players[0].cards.push(Card::MarketDay);
    let s = pick(&s, 0, "play_card", "market_day");
    let s = pick(&s, 0, "market", "fish");
    assert_eq!(s.players[0].res, [0, 0, 0, 5, 0]);
    assert_eq!(s.players[1].res, [0, 0, 0, 0, 1]);
    assert_eq!(s.players[2].res, [0; 5]);
    assert_eq!(s.players[3].res, [1, 0, 0, 0, 0]);
}

#[test]
fn old_lore_is_hidden_until_it_wins() {
    let mut s = blank(3);
    s.players[0].res = COST_ALMANAC;
    s.deck = vec![Card::OldLore];
    let s = press(&s, 0, "buy_almanac");
    assert_eq!(s.players[0].fresh, vec![Card::OldLore]);
    assert_eq!(s.visible_points(0), 0);
    assert_eq!(s.total_points(0), 1);
    let table = Tideholm.table_view(&s).unwrap().to_string();
    assert!(!table.contains("Old Lore"));
    assert!(!table.contains("Lore"));
    assert_eq!(
        Tideholm.table_view(&s).unwrap()["extra"]["points"],
        json!([0, 0, 0])
    );
    // It can't be played.
    let mut t = s.clone();
    t.players[0].cards = vec![Card::OldLore];
    t.players[0].fresh.clear();
    let same = pick(&t, 0, "play_card", "old_lore");
    assert_eq!(same, t);
    // An empty deck can't be bought from.
    let mut u = blank(3);
    u.players[0].res = COST_ALMANAC;
    u.deck.clear();
    assert_eq!(press(&u, 0, "buy_almanac"), u);
}

#[test]
fn bank_trades_four_for_one() {
    let mut s = blank(3);
    s.players[0].res = [4, 0, 0, 0, 0];
    s.bank[0] -= 4;
    let same = pick(&s, 0, "bank_trade", "reed>reed");
    assert_eq!(same, s);
    let s = pick(&s, 0, "bank_trade", "reed>copper");
    assert_eq!(s.players[0].res, [0, 0, 0, 0, 1]);
    assert_eq!(total_cards(&s), [19; 5]);
    let same = pick(&s, 0, "bank_trade", "copper>reed");
    assert_eq!(same, s);
}

fn trading_state() -> State {
    let mut s = blank(3);
    s.players[0].res = [2, 0, 0, 0, 0];
    s.players[1].res = [0, 1, 0, 0, 0];
    s.players[2].res = [0, 2, 1, 0, 0];
    s
}

#[test]
fn a_player_trade_with_an_accept() {
    let s = trading_state();
    let out = act(
        &s,
        0,
        PhoneInput::with("offer", json!(["give:reed:1", "want:clay:1"])),
    );
    let s = out.state;
    assert!(matches!(s.phase, Phase::TradeRespond { .. }));
    // Both others are asked in a reaction window that declines on timeout.
    assert_eq!(out.waiting_on.len(), 2);
    for w in &out.waiting_on {
        assert!(w.reaction);
        assert_eq!(
            w.on_timeout,
            OnTimeout::Input {
                input: PhoneInput::press("trade_decline")
            }
        );
    }
    let ui = Tideholm.phone_view(&s, 1).unwrap();
    assert!(ui.has_input("trade_accept"));
    assert!(ui.has_input("trade_decline"));
    assert!(ui.has_input("counter"));
    let s = press(&s, 1, "trade_accept");
    let s = press(&s, 2, "trade_decline");
    assert!(matches!(s.phase, Phase::TradeConfirm { .. }));
    let ui = Tideholm.phone_view(&s, 0).unwrap();
    assert!(ui.accepts(&PhoneInput::with("trade_pick", "1")));
    assert!(!ui.accepts(&PhoneInput::with("trade_pick", "2")));
    let s = pick(&s, 0, "trade_pick", "1");
    assert_eq!(s.players[0].res, [1, 1, 0, 0, 0]);
    assert_eq!(s.players[1].res, [1, 0, 0, 0, 0]);
    assert_eq!(s.trades_completed, 1);
    assert_eq!(s.phase, Phase::Main);
    assert_eq!(
        Tideholm.table_view(&s).unwrap()["extra"]["trades_completed"],
        json!(1)
    );
}

#[test]
fn a_player_trade_with_a_counter_offer() {
    let s = trading_state();
    let s = pick(&s, 0, "offer", json!(["give:reed:1", "want:clay:1"]));
    // Seat 2 counters: "I give 2 Clay, I want 2 Reed".
    let s = pick(
        &s,
        2,
        "counter",
        json!(["give:clay:1", "give:clay:2", "want:reed:1", "want:reed:2"]),
    );
    // Seat 1 runs out of time: declines.
    let out = Tideholm
        .on_action(s, 1, &PhoneInput::press("trade_decline"))
        .unwrap();
    let s = out.state;
    assert_eq!(out.waiting_on.len(), 1);
    assert_eq!(out.waiting_on[0].seat, 0);
    let ui = Tideholm.phone_view(&s, 0).unwrap();
    assert!(ui.accepts(&PhoneInput::with("trade_pick", "2")));
    let s = pick(&s, 0, "trade_pick", "2");
    assert_eq!(s.players[0].res, [0, 2, 0, 0, 0]);
    assert_eq!(s.players[2].res, [2, 0, 1, 0, 0]);
    assert_eq!(s.trades_completed, 1);
}

#[test]
fn trades_can_be_withdrawn_and_are_capped_per_turn() {
    let s = trading_state();
    let s = pick(&s, 0, "offer", json!(["give:reed:1", "want:clay:1"]));
    let s = press(&s, 0, "trade_cancel");
    assert_eq!(s.phase, Phase::Main);
    let s = pick(&s, 0, "offer", json!(["give:reed:1", "want:salt:1"]));
    let s = press(&s, 1, "trade_decline");
    let s = press(&s, 2, "trade_decline");
    assert_eq!(s.phase, Phase::Main);
    assert_eq!(s.proposals, 2);
    let same = pick(&s, 0, "offer", json!(["give:reed:1", "want:clay:1"]));
    assert_eq!(same, s);
    // Can't offer what you don't hold or both sides of one resource.
    let mut t = trading_state();
    t.proposals = 0;
    assert_eq!(
        pick(&t, 0, "offer", json!(["give:salt:1", "want:clay:1"])),
        t
    );
    assert_eq!(
        pick(&t, 0, "offer", json!(["give:reed:1", "want:reed:2"])),
        t
    );
}

#[test]
fn ten_points_on_your_turn_wins() {
    let t = topo();
    let mut s = blank(3);
    // 4 lodges (8) + Old Lore (1) = 9; one more hut wins.
    let spots = [0usize, 2, 4, 6];
    for &h in &spots {
        let v = t.hex_verts[h][0];
        s.buildings[v] = Some(Building {
            seat: 0,
            lodge: true,
        });
        s.players[0].lodges += 1;
    }
    s.players[0].cards.push(Card::OldLore);
    let v = t.hex_verts[CENTER][3];
    road(&mut s, 0, t.vert_edges[v][0]);
    s.players[0].res = COST_HUT;
    assert_eq!(s.total_points(0), 9);
    let out = act(&s, 0, PhoneInput::with("build_hut", v.to_string()));
    let res = out.finished.expect("finished");
    assert_eq!(res.ranking[0], vec![0]);
    assert!(res.summary.contains("Ana"));
    assert!(matches!(out.state.phase, Phase::Over { winner: 0 }));
    assert!(out.waiting_on.is_empty());
    // Another seat's points only count on their own turn.
    let mut u = blank(3);
    u.players[1].cards = vec![Card::OldLore; 5];
    for &h in &spots[..3] {
        u.buildings[t.hex_verts[h][0]] = Some(Building {
            seat: 1,
            lodge: true,
        });
        u.players[1].lodges += 1;
    }
    assert_eq!(u.total_points(1), 11);
    let u = press(&u, 0, "end_turn");
    let u2 = u.clone();
    assert!(matches!(u2.phase, Phase::Over { winner: 1 }));
}

#[test]
fn odd_inputs_leave_the_state_alone() {
    let s = blank(3);
    let junk = [
        PhoneInput::press("roll"),
        PhoneInput::with("build_hut", "999"),
        PhoneInput::with("build_hut", json!(["1"])),
        PhoneInput::with("bank_trade", "nonsense"),
        PhoneInput::with("offer", "x"),
        PhoneInput::with("offer", json!(["give:reed:1", "give:reed:1"])),
        PhoneInput::with("discard", json!(["reed:0"])),
        PhoneInput::with("trade_pick", "-1"),
        PhoneInput::press("who_knows"),
    ];
    for input in &junk {
        for seat in [0, 1, 7] {
            let out = Tideholm.on_action(s.clone(), seat, input).unwrap();
            assert_eq!(out.state, s, "{input:?}");
            assert!(out
                .events
                .iter()
                .all(|e| !matches!(e, Event::Feedback { seat, .. } if *seat >= 3)));
            // A rejected input keeps the turn timer listed.
            assert_eq!(out.waiting_on, waits_for(&s));
        }
    }
    assert!(Tideholm.phone_view(&s, 9).unwrap().inputs().is_empty());
    assert!(Tideholm.setup(&seats(2), 1).is_err());
    // A huge pick list during Bountiful Tide is refused, not overflowed.
    let mut b = blank(3);
    b.phase = Phase::Bounty { back: Back::Main };
    let huge: Vec<String> = (0..200)
        .map(|i| format!("reed:{i}"))
        .chain((0..100).map(|i| format!("clay:{i}")))
        .collect();
    let out = Tideholm
        .on_action(b.clone(), 0, &PhoneInput::with("bounty", json!(huge)))
        .unwrap();
    assert_eq!(out.state, b);
    // With fewer than 2 cards in the bank the card fizzles.
    let mut c = blank(3);
    c.bank = [1, 0, 0, 0, 0];
    c.players[0].cards.push(Card::BountifulTide);
    let c = pick(&c, 0, "play_card", "bountiful_tide");
    assert_eq!(c.phase, Phase::Main);
}

// ---------------------------------------------------------------------------
// Rooms

fn room_for<R: Rules>(rules: R, n: usize, seed: u64) -> Room<R> {
    let mut room = Room::new(rules, seed);
    for (i, s) in seats(n).iter().enumerate() {
        room.player_joined(i as PlayerId + 1, s.name.clone(), String::new(), 0);
    }
    room.start(0).unwrap();
    room
}

struct Played {
    moves: usize,
    idle: usize,
    finished: bool,
}

/// Drive a room with the bot. Seats being waited on act first.
fn drive<R: Rules>(room: &mut Room<R>, n: usize, seed: u64, max: usize, checks: bool) -> Played
where
    R::State: Serialize,
{
    let mut rng = Rng::new(seed ^ 0x5eed);
    let mut now = 0u64;
    let mut moves = 0;
    let mut idle = 0;
    while room.finished().is_none() && moves < max {
        assert!(room.paused().is_none(), "paused: {:?}", room.paused());
        let mut order: Vec<PlayerId> = room
            .waits()
            .iter()
            .filter_map(|w| room.player_in(w.wait.seat))
            .collect();
        rng.shuffle(&mut order);
        for p in 1..=n as PlayerId {
            if !order.contains(&p) {
                order.push(p);
            }
        }
        let mut acted = false;
        for p in order {
            let Some(ui) = room.current_ui(p).cloned() else {
                continue;
            };
            if checks {
                assert!(ui.node_count() <= MAX_UI_NODES);
            }
            if let Some(input) = bot::choose(&ui, &mut rng) {
                assert!(ui.accepts(&input), "{input:?} not on {ui:?}");
                now += 50;
                let effects = room.handle_input(p, input.clone(), now).unwrap();
                let warned = effects.iter().any(|e| {
                    matches!(
                        e,
                        Effect::Event(Event::Feedback {
                            kind: Feedback::Warning,
                            ..
                        })
                    )
                });
                assert!(!warned, "the rules rejected the bot's {input:?}");
                moves += 1;
                acted = true;
                break;
            }
        }
        if !acted {
            idle += 1;
            assert!(idle < 10_000, "stuck");
            now = room.next_deadline().unwrap_or(now) + 1;
            room.tick(now);
        }
        if checks {
            let table = room.table_view().unwrap().unwrap();
            check_table(&table).unwrap();
            let size = serde_json::to_string(room.state().unwrap()).unwrap().len();
            assert!(size < 100_000, "state is {size} bytes");
            // Every timeout the rules handle moves the game on.
            let typed: State =
                serde_json::from_value(serde_json::to_value(room.state().unwrap()).unwrap())
                    .unwrap();
            for w in waits_for(&typed) {
                if w.on_timeout == OnTimeout::Rules {
                    let next = Tideholm.on_timeout(typed.clone(), w.seat, 0).unwrap().state;
                    assert_ne!(next, typed, "timeout for {} in {:?}", w.seat, typed.phase);
                }
            }
        }
    }
    Played {
        moves,
        idle,
        finished: room.finished().is_some(),
    }
}

#[test]
fn bots_finish_games_for_many_seeds() {
    let mut total = 0;
    let mut games = 0;
    let mut trades = 0;
    let mut idle = 0;
    let mut most = 0;
    for n in [3, 4] {
        for seed in 0..24u64 {
            let mut room = room_for(Tideholm, n, seed);
            let played = drive(&mut room, n, seed, 3000, seed < 6);
            assert_eq!(played.idle, 0, "the bot left a turn to time out");
            assert!(
                played.finished,
                "{n} players seed {seed}: not finished after {} moves (turn {})",
                played.moves,
                room.state().unwrap().turn
            );
            trades += room.state().unwrap().trades_completed;
            total += played.moves;
            idle += played.idle;
            most = most.max(played.moves);
            games += 1;
        }
    }
    let avg = total / games;
    eprintln!(
        "bots: {games} games, average {avg} moves (most {most}), {idle} timeouts, {trades} player trades"
    );
    assert!(trades > 0);
}

#[test]
fn a_bot_game_through_json_state_like_the_sandbox() {
    let mut room = room_for(Json(Tideholm), 4, 99);
    let played = drive(&mut room, 4, 99, 3000, true);
    assert!(played.finished);
    let state = serde_json::to_string(room.state().unwrap()).unwrap();
    assert!(state.len() < 100_000);
}

#[test]
fn games_replay_exactly_from_seed_and_log() {
    for seed in [1u64, 2, 3] {
        let mut room = room_for(Tideholm, 4, seed);
        drive(&mut room, 4, seed, 3000, false);
        let replayed = replay(&Tideholm, &room.seats(), seed, room.log()).unwrap();
        assert_eq!(&replayed, room.state().unwrap());
    }
    let a = Tideholm.setup(&seats(3), 42).unwrap().state;
    let b = Tideholm.setup(&seats(3), 42).unwrap().state;
    assert_eq!(a, b);
    assert_ne!(a.tiles, Tideholm.setup(&seats(3), 43).unwrap().state.tiles);
}

#[test]
fn nobody_sees_another_seats_hand() {
    let mut room = room_for(Tideholm, 4, 7);
    drive(&mut room, 4, 7, 120, false);
    let mut s = room.state().unwrap().clone();
    // Distinctive hands and cards.
    s.players[0].res = [7, 0, 0, 0, 0];
    s.players[1].res = [0, 6, 0, 0, 0];
    s.players[2].res = [0, 0, 5, 0, 0];
    s.players[3].res = [0, 0, 0, 4, 0];
    s.players[1].cards = vec![Card::OldLore, Card::MarketDay];
    s.players[2].fresh = vec![Card::BountifulTide];
    s.players[1].notes = vec!["secret note for Bo".into()];
    let table = Tideholm.table_view(&s).unwrap().to_string();
    for p in &s.players {
        assert!(!table.contains(&view::hand_counts_text(p)));
    }
    assert!(!table.contains("hand_counts"));
    assert!(!table.contains("Old Lore"));
    assert!(!table.contains("Market Day"));
    assert!(!table.contains("Bountiful"));
    assert!(!table.contains("secret note"));
    for seat in 0..4u32 {
        let ui = serde_json::to_string(&Tideholm.phone_view(&s, seat).unwrap()).unwrap();
        assert!(ui.contains(&view::hand_counts_text(&s.players[seat as usize])));
        for other in 0..4u32 {
            if other != seat {
                assert!(!ui.contains(&view::hand_counts_text(&s.players[other as usize])));
            }
        }
        assert_eq!(ui.contains("Old Lore"), seat == 1);
        assert_eq!(ui.contains("Market Day"), seat == 1);
        assert_eq!(ui.contains("Bountiful"), seat == 2);
        assert_eq!(ui.contains("secret note"), seat == 1);
    }
}

#[test]
fn random_fuzzing_never_panics_or_pauses() {
    for seed in 0..12u64 {
        let n = 3 + (seed % 2) as usize;
        let mut room = room_for(Tideholm, n, seed);
        let mut rng = Rng::new(seed);
        let mut now = 0;
        for step in 0..1500 {
            if room.finished().is_some() {
                break;
            }
            assert!(room.paused().is_none(), "paused: {:?}", room.paused());
            let p = rng.range(1, n as u32) as PlayerId;
            if step % 37 == 0 {
                now += 200_000;
                room.tick(now);
                continue;
            }
            let Some(ui) = room.current_ui(p).cloned() else {
                continue;
            };
            assert!(ui.node_count() <= MAX_UI_NODES);
            let input = if rng.below(4) == 0 {
                random_input(&ui, &mut rng, &[("end_turn", 30)])
            } else {
                bot::choose(&ui, &mut rng)
            };
            if let Some(input) = input {
                now += 10;
                room.handle_input(p, input, now).unwrap();
            }
            // Junk the host would drop, sent straight to the rules.
            let junk = PhoneInput::with("build_hut", json!({"x": 1}));
            let s = room.state().unwrap().clone();
            assert_eq!(Tideholm.on_action(s.clone(), 0, &junk).unwrap().state, s);
        }
        check_table(&room.table_view().unwrap().unwrap()).unwrap();
    }
}

#[test]
fn timeouts_alone_keep_the_game_moving() {
    for n in [3, 4] {
        let mut room = room_for(Tideholm, n, 5);
        for _ in 0..400 {
            assert!(room.paused().is_none());
            if room.finished().is_some() {
                break;
            }
            let now = room.next_deadline().expect("always waiting on someone") + 1;
            room.tick(now);
        }
        let s = room.state().unwrap();
        assert!(!matches!(s.phase, Phase::Setup { .. }), "setup finished");
        assert!(s.turn > 50, "turn {}", s.turn);
        assert_eq!(total_cards(s), [19; 5]);
        let buildings = s.buildings.iter().flatten().count();
        assert_eq!(buildings, 2 * n);
    }
}

#[test]
fn a_seven_in_a_room_waits_on_every_discarding_seat_at_once() {
    let mut s = blank(4);
    s.players[1].res = [2, 2, 2, 2, 0];
    s.players[3].res = [0, 2, 2, 2, 2];
    let s = roll_seven(&s);
    let waits = waits_for(&s);
    let seats: Vec<SeatId> = waits.iter().map(|w| w.seat).collect();
    assert_eq!(seats, vec![1, 3]);
    // On timeout each discards at random.
    let s = Tideholm.on_timeout(s, 3, 0).unwrap().state;
    let s = Tideholm.on_timeout(s, 1, 0).unwrap().state;
    assert_eq!(s.players[1].total(), 4);
    assert_eq!(s.players[3].total(), 4);
    assert!(matches!(s.phase, Phase::MoveSquall { .. }));
}

#[test]
fn the_bot_reads_a_timer_wrapped_screen() {
    let mut room = room_for(Tideholm, 3, 1);
    let ui = room.current_ui(1).unwrap().clone();
    let wrapped = Widget::column(vec![
        Widget::Timer {
            ends_in_ms: 1000,
            total_ms: Some(1000),
            label: "Place your first hut".into(),
        },
        ui,
    ]);
    let mut rng = Rng::new(1);
    let input = bot::choose(&wrapped, &mut rng).unwrap();
    assert!(wrapped.accepts(&input));
    assert_eq!(input.widget, "place_hut");
    room.handle_input(1, input, 10).unwrap();
    assert!(matches!(
        room.state().unwrap().phase,
        Phase::Setup { hut: Some(_), .. }
    ));
}

#[test]
fn can_respond_matches_who_owes_an_action() {
    let s = trading_state();
    let s = pick(&s, 0, "offer", json!(["give:reed:1", "want:clay:1"]));
    assert!(Tideholm.can_respond(&s, 1).unwrap());
    assert!(Tideholm.can_respond(&s, 2).unwrap());
    let mut broke = s.clone();
    broke.players[2].res = [0; 5];
    assert!(!Tideholm.can_respond(&broke, 2).unwrap());
    assert!(!Tideholm.can_respond(&s, 9).unwrap());
}

#[test]
fn bots_finish_an_async_game_with_auto_passes() {
    use hostable_host_core::Pace;
    for seed in [4u64, 8] {
        let mut room = Room::new(Tideholm, seed).with_pace(Pace::Async);
        for (i, s) in seats(4).iter().enumerate() {
            room.player_joined(i as PlayerId + 1, s.name.clone(), String::new(), 0);
        }
        room.start(0).unwrap();
        let played = drive(&mut room, 4, seed, 3000, false);
        assert!(played.finished, "async seed {seed}");
        assert_eq!(played.idle, 0);
    }
}
