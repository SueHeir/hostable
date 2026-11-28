use super::*;
use crate::cards::{find, starter, LAMPLIGHTER, STARTERS};
use hostable_host_core::{bot::random_input, check_table, replay, LoggedAction, Pace, Room};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn seats() -> Vec<Seat> {
    vec![Seat::new(0, "Ana"), Seat::new(1, "Bo")]
}

fn id(name: &str) -> CardId {
    find(name).unwrap_or_else(|| panic!("no card {name}"))
}

/// A game in the main phase on Ana's turn, both with 10 Spark, empty hands
/// and decks of filler.
fn duel() -> State {
    let mut s = new_game(&seats(), 1);
    let filler = id("Gutter Rat Pack");
    for p in &mut s.players {
        p.deck = vec![filler; 15];
        p.built = true;
        p.spark = 10;
        p.spark_max = 10;
    }
    s.phase = Phase::Main;
    s.active = 0;
    s.turn = 3;
    s
}

fn give(s: &mut State, seat: SeatId, name: &str) -> CardId {
    let c = id(name);
    s.players[seat as usize].hand.push(c);
    c
}

/// A ready Ally that has been in play since before this turn.
fn put_ally(s: &mut State, seat: SeatId, name: &str) -> u32 {
    let uid = s.next_uid;
    assert!(add_ally(s, seat, id(name)));
    s.ally_mut(uid).unwrap().fresh = false;
    uid
}

fn input(s: State, seat: SeatId, widget: &str, value: Value) -> State {
    step(s, seat, &PhoneInput::with(widget, value)).state
}

fn press(s: State, seat: SeatId, widget: &str) -> State {
    step(s, seat, &PhoneInput::press(widget)).state
}

/// Play a card from hand, picking `target` if it needs one.
fn play(s: State, seat: SeatId, name: &str, target: Option<&str>) -> State {
    let before = s.clone();
    let mut s = input(s, seat, "play", json!(format!("c{}", id(name))));
    if let Some(t) = target {
        s = input(s, seat, "target", json!(t));
    }
    assert_ne!(s, before, "{name} could not be played");
    s
}

/// Pass with whoever holds priority until the stack is empty.
fn settle(mut s: State) -> State {
    for _ in 0..64 {
        if s.stack.is_empty() || s.phase != Phase::Main {
            return s;
        }
        let seat = s.priority;
        s = press(s, seat, "pass");
    }
    panic!("the stack never emptied");
}

fn resolve(s: &State, seat: SeatId) -> i32 {
    s.players[seat as usize].resolve
}

fn ui_json(s: &State, seat: SeatId) -> String {
    serde_json::to_string(&view::phone(s, seat)).unwrap()
}

// ---------------------------------------------------------------------------
// The pool
// ---------------------------------------------------------------------------

#[test]
fn the_pool_has_64_distinct_cards_in_four_schools() {
    let pool = &CARDS[..POOL_SIZE];
    assert_eq!(pool.len(), 64);
    let mut names: Vec<&str> = pool.iter().map(|c| c.name).collect();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), 64);
    for school in [
        School::Ember,
        School::Quicksilver,
        School::Lantern,
        School::Velvet,
    ] {
        assert_eq!(pool.iter().filter(|c| c.school == school).count(), 16);
    }
    for cost in 0..=7 {
        assert!(pool.iter().any(|c| c.cost == cost), "no card costs {cost}");
    }
    for kind in [Kind::Ally, Kind::Feint, Kind::Gambit, Kind::Charm] {
        assert!(pool.iter().filter(|c| c.kind == kind).count() >= 4);
    }
    assert!(pool.iter().all(|c| !c.token));
    assert!(card(LAMPLIGHTER).token);
    for c in pool {
        let text = card_text(c);
        assert!(text.starts_with(c.kind.name()), "{}: {text}", c.name);
        assert!(text.len() > 8);
        if c.kind == Kind::Charm {
            assert!(c.charm.is_some());
        }
        // Allies' arrival effects are never targeted.
        if c.kind == Kind::Ally {
            assert!(c.effects.iter().all(|e| !e.targeted()), "{}", c.name);
        }
    }
    // Unknown ids fall back without panicking.
    assert!(card(9999).token);
}

#[test]
fn starter_decks_are_20_distinct_pool_cards_with_cheap_feints() {
    for (i, (name, names)) in STARTERS.iter().enumerate() {
        let deck = starter(i);
        assert_eq!(deck.len(), 20, "{name}: {names:?}");
        let mut d = deck.clone();
        d.sort();
        d.dedup();
        assert_eq!(d.len(), 20, "{name} repeats a card");
        let cheap_feints = deck
            .iter()
            .filter(|c| card(**c).kind == Kind::Feint && card(**c).cost <= 2)
            .count();
        assert!(cheap_feints >= 4, "{name}");
    }
}

#[test]
fn card_text_reads_like_the_effects() {
    assert_eq!(
        card_text(card(id("Ember Dart"))),
        "Feint. Deal 2 damage to any target."
    );
    assert_eq!(
        card_text(card(id("Cinder Whelp"))),
        "Ally 1/1. Swift (can strike the turn it arrives)."
    );
    assert_eq!(
        card_text(card(id("Lantern Over the Door"))),
        "Charm. At the start of your turn, gain 2 Resolve."
    );
    assert!(
        card_text(card(id("Spark Juggler"))).contains("Ability (1 Spark, spend this ready Ally")
    );
    assert!(card_text(card(id("Recant the Formula"))).contains("Negate the card or ability"));
}

// ---------------------------------------------------------------------------
// Effects, one test per kind
// ---------------------------------------------------------------------------

#[test]
fn damage_to_any_target_hits_players_and_allies() {
    let mut s = duel();
    give(&mut s, 0, "Ember Dart");
    let s = settle(play(s, 0, "Ember Dart", Some("foe")));
    assert_eq!(resolve(&s, 1), 18);
    assert_eq!(s.players[0].spark, 9);
    assert_eq!(s.players[0].gutter, vec![id("Ember Dart")]);

    let mut s = duel();
    let rat = put_ally(&mut s, 1, "Gutter Rat Pack"); // 2/1
    give(&mut s, 0, "Ember Dart");
    let s = settle(play(s, 0, "Ember Dart", Some(&format!("a{rat}"))));
    assert!(s.ally(rat).is_none());
    assert_eq!(s.players[1].gutter, vec![id("Gutter Rat Pack")]);
}

#[test]
fn damage_to_target_ally_marks_damage_until_end_of_turn() {
    let mut s = duel();
    let w = put_ally(&mut s, 1, "Watchtower Warden"); // 4/6
    give(&mut s, 0, "Alembic Burst");
    let s = settle(play(s, 0, "Alembic Burst", Some(&format!("a{w}"))));
    assert_eq!(s.ally(w).unwrap().damage, 3);
    // Players aren't legal targets for an Ally-only card.
    assert!(!s.target_ok(0, Tgt::Ally, Target::Player(1)));
    let s = press(s, 0, "end_turn");
    assert_eq!(s.ally(w).unwrap().damage, 0);
}

#[test]
fn damage_to_the_opponent_and_each_enemy_ally() {
    let mut s = duel();
    let a = put_ally(&mut s, 1, "Gutter Rat Pack");
    let b = put_ally(&mut s, 1, "Watchtower Warden");
    let mine = put_ally(&mut s, 0, "Cinder Whelp");
    give(&mut s, 0, "Scorch the Rooftops");
    let s = settle(play(s, 0, "Scorch the Rooftops", None));
    assert!(s.ally(a).is_none());
    assert_eq!(s.ally(b).unwrap().damage, 1);
    assert!(s.ally(mine).is_some());
    assert_eq!(resolve(&s, 1), 19);

    let mut s = duel();
    give(&mut s, 0, "Blazing Lunge");
    let s = settle(play(s, 0, "Blazing Lunge", None));
    assert_eq!(resolve(&s, 1), 17);
}

#[test]
fn damage_to_each_ally_hits_both_sides() {
    let mut s = duel();
    let a = put_ally(&mut s, 1, "Kiln Brawler"); // guard 1
    let b = put_ally(&mut s, 0, "Furnace Duellist"); // guard 2
    let c = put_ally(&mut s, 0, "Watchtower Warden"); // guard 6
    give(&mut s, 0, "Ashfall Volley");
    let s = settle(play(s, 0, "Ashfall Volley", None));
    assert!(s.ally(a).is_none() && s.ally(b).is_none());
    assert_eq!(s.ally(c).unwrap().damage, 2);
}

#[test]
fn loss_ignores_wards_but_damage_does_not() {
    let mut s = duel();
    s.players[1].ward = 5;
    give(&mut s, 0, "Curtain at Midnight");
    give(&mut s, 1, "Kindly Glow");
    let s = settle(play(s, 0, "Curtain at Midnight", None));
    assert_eq!(resolve(&s, 1), 16);
    assert_eq!(s.players[1].ward, 5);
    assert!(s.players[1].hand.is_empty(), "discarded at random");

    let mut s = duel();
    give(&mut s, 0, "Watchman's Shield");
    let s = settle(play(s, 0, "Watchman's Shield", None));
    assert_eq!(s.players[0].ward, 3);
    let mut s = press(s, 0, "end_turn");
    give(&mut s, 1, "Blazing Lunge");
    give(&mut s, 1, "Ember Dart");
    let s = settle(play(s, 1, "Blazing Lunge", None));
    assert_eq!((resolve(&s, 0), s.players[0].ward), (20, 0));
    let s = settle(play(s, 1, "Ember Dart", Some("foe")));
    assert_eq!(resolve(&s, 0), 18);
    // Wards end when your next turn starts.
    let mut s = duel();
    s.players[1].ward = 4;
    let s = press(s, 0, "end_turn");
    assert_eq!(s.players[1].ward, 0);
}

#[test]
fn hollow_bargain_costs_resolve_and_draws() {
    let mut s = duel();
    give(&mut s, 0, "Hollow Bargain");
    let s = settle(play(s, 0, "Hollow Bargain", None));
    assert_eq!(resolve(&s, 0), 18);
    assert_eq!(s.players[0].hand.len(), 2);
    assert_eq!(s.players[0].deck.len(), 13);
}

#[test]
fn healing_and_draining() {
    let mut s = duel();
    give(&mut s, 0, "Kindly Glow");
    let s = settle(play(s, 0, "Kindly Glow", None));
    assert_eq!(resolve(&s, 0), 22);

    let mut s = duel();
    give(&mut s, 0, "Siphon Kiss");
    let s = settle(play(s, 0, "Siphon Kiss", None));
    assert_eq!((resolve(&s, 0), resolve(&s, 1)), (22, 18));

    let mut s = duel();
    give(&mut s, 0, "Sanctuary Bell");
    let s = settle(play(s, 0, "Sanctuary Bell", None));
    assert_eq!((resolve(&s, 0), s.players[0].ward), (24, 2));
}

#[test]
fn drawing_cards() {
    let mut s = duel();
    give(&mut s, 0, "Distilled Insight");
    let s = settle(play(s, 0, "Distilled Insight", None));
    assert_eq!(s.players[0].hand.len(), 2);
    let mut s = s;
    give(&mut s, 0, "Glimmer of Foresight");
    let s = settle(play(s, 0, "Glimmer of Foresight", None));
    assert_eq!(s.players[0].hand.len(), 3);
}

#[test]
fn boosts_last_until_end_of_turn() {
    let mut s = duel();
    let w = put_ally(&mut s, 0, "Cinder Whelp");
    give(&mut s, 0, "Flashpowder Pinch");
    let s = settle(play(s, 0, "Flashpowder Pinch", Some(&format!("a{w}"))));
    assert_eq!(s.power(s.ally(w).unwrap()), 3);
    let s = press(s, 0, "end_turn");
    assert_eq!(s.power(s.ally(w).unwrap()), 1);

    let mut s = duel();
    let w = put_ally(&mut s, 0, "Lamp-post Sentinel");
    give(&mut s, 0, "Lamppost Stand");
    let s = settle(play(s, 0, "Lamppost Stand", Some(&format!("a{w}"))));
    assert_eq!(s.guard(s.ally(w).unwrap()), 7);
}

#[test]
fn team_boosts_touch_only_your_allies() {
    let mut s = duel();
    let a = put_ally(&mut s, 0, "Cinder Whelp");
    let b = put_ally(&mut s, 1, "Cinder Whelp");
    give(&mut s, 0, "Fury of the Kilns");
    let s = settle(play(s, 0, "Fury of the Kilns", None));
    assert_eq!(s.power(s.ally(a).unwrap()), 3);
    assert_eq!(s.power(s.ally(b).unwrap()), 1);
    let mut s = s;
    give(&mut s, 0, "Rally the Lanterns");
    let s = settle(play(s, 0, "Rally the Lanterns", None));
    let al = s.ally(a).unwrap();
    assert_eq!((s.power(al), s.guard(al)), (4, 2));
}

#[test]
fn rekindled_stride_readies_and_boosts_your_ally() {
    let mut s = duel();
    let a = put_ally(&mut s, 0, "Kiln Brawler");
    let foe = put_ally(&mut s, 1, "Kiln Brawler");
    s.ally_mut(a).unwrap().spent = true;
    give(&mut s, 0, "Rekindled Stride");
    // Only your own Allies are offered.
    let s1 = input(
        s.clone(),
        0,
        "play",
        json!(format!("c{}", id("Rekindled Stride"))),
    );
    assert!(!ui_json(&s1, 0).contains(&format!("\"a{foe}\"")));
    let s = settle(play(s, 0, "Rekindled Stride", Some(&format!("a{a}"))));
    let al = s.ally(a).unwrap();
    assert!(!al.spent);
    assert_eq!((s.power(al), s.guard(al)), (4, 2));
}

#[test]
fn silver_hush_spends_an_ally_so_it_cannot_strike_or_block() {
    // On Bo's turn he spends Ana's blocker, then strikes past it.
    let mut s = duel();
    s.active = 1;
    let guard = put_ally(&mut s, 0, "Watchtower Warden");
    let rat = put_ally(&mut s, 1, "Gutter Rat Pack");
    give(&mut s, 1, "Silver Hush");
    let s = settle(play(s, 1, "Silver Hush", Some(&format!("a{guard}"))));
    assert!(s.ally(guard).unwrap().spent);
    let s = settle(input(s, 1, "strike", json!(format!("a{rat}"))));
    // Nobody can block, so there is no block step.
    assert_eq!(s.phase, Phase::Main);
    assert!(s.combat.is_none());
    assert_eq!(resolve(&s, 0), 18);
    // A spent Ally can't strike on its owner's turn either.
    let mut s = duel();
    let a = put_ally(&mut s, 0, "Kiln Brawler");
    s.ally_mut(a).unwrap().spent = true;
    assert!(s.strikers(0).is_empty());
}

#[test]
fn returning_allies_to_hand() {
    let mut s = duel();
    let a = put_ally(&mut s, 1, "Kiln Brawler");
    summon(&mut s, 1, 1);
    let token = s.allies.last().unwrap().uid;
    give(&mut s, 0, "Reflux Tincture");
    let s = settle(play(s, 0, "Reflux Tincture", Some(&format!("a{a}"))));
    assert!(s.ally(a).is_none());
    assert_eq!(s.players[1].hand, vec![id("Kiln Brawler")]);
    // A token returned to hand just fades.
    let mut s = s;
    give(&mut s, 0, "Volatile Solution");
    let s = settle(play(s, 0, "Volatile Solution", Some(&format!("a{token}"))));
    assert!(s.ally(token).is_none());
    assert_eq!(s.players[1].hand.len(), 1);
    assert_eq!(s.players[0].hand.len(), 1, "drew a card");

    let mut s = duel();
    put_ally(&mut s, 1, "Kiln Brawler");
    put_ally(&mut s, 1, "Cinder Whelp");
    let mine = put_ally(&mut s, 0, "Cinder Whelp");
    give(&mut s, 0, "Grand Transmutation");
    let s = settle(play(s, 0, "Grand Transmutation", None));
    assert_eq!(s.allies_of(1).count(), 0);
    assert!(s.ally(mine).is_some());
    assert_eq!(s.players[1].hand.len(), 2);
    assert_eq!(s.players[0].hand.len(), 2);
}

#[test]
fn destroying_an_ally() {
    let mut s = duel();
    let a = put_ally(&mut s, 1, "Watchtower Warden");
    give(&mut s, 0, "Kiss of Nightshade");
    let s = settle(play(s, 0, "Kiss of Nightshade", Some(&format!("a{a}"))));
    assert!(s.ally(a).is_none());
    assert_eq!(s.players[1].gutter, vec![id("Watchtower Warden")]);
    let mut s = s;
    let b = put_ally(&mut s, 1, "Kiln Brawler");
    give(&mut s, 0, "Dusk Decree");
    let s = settle(play(s, 0, "Dusk Decree", Some(&format!("a{b}"))));
    assert!(s.ally(b).is_none());
}

#[test]
fn discard_mill_and_recall() {
    let mut s = duel();
    give(&mut s, 1, "Ember Dart");
    give(&mut s, 1, "Kiln Brawler");
    give(&mut s, 1, "Blazing Lunge");
    give(&mut s, 0, "Pickpocket's Trick");
    let s = settle(play(s, 0, "Pickpocket's Trick", None));
    assert_eq!(s.players[1].hand.len(), 2);
    assert_eq!(s.players[1].gutter.len(), 1);
    let mut s = s;
    give(&mut s, 0, "Moonless Heist");
    let s = settle(play(s, 0, "Moonless Heist", None));
    assert_eq!(s.players[1].hand.len(), 0);

    let mut s = duel();
    give(&mut s, 0, "Alley Whispers");
    let s = settle(play(s, 0, "Alley Whispers", None));
    assert_eq!(
        (s.players[1].deck.len(), s.players[1].gutter.len()),
        (12, 3)
    );

    let mut s = duel();
    s.players[0].gutter = vec![id("Ember Dart"), id("Kiln Brawler")];
    give(&mut s, 0, "Echo from the Gutter");
    let s = settle(play(s, 0, "Echo from the Gutter", None));
    assert_eq!(s.players[0].hand, vec![id("Kiln Brawler")]);
    assert_eq!(
        s.players[0].gutter,
        vec![id("Ember Dart"), id("Echo from the Gutter")]
    );
}

#[test]
fn summoning_lamplighters_up_to_the_limit() {
    let mut s = duel();
    give(&mut s, 0, "Call the Watch");
    let s = settle(play(s, 0, "Call the Watch", None));
    assert_eq!(s.allies_of(0).count(), 2);
    assert!(s.allies_of(0).all(|a| a.card == LAMPLIGHTER && a.fresh));
    let mut s = s;
    for _ in 0..3 {
        put_ally(&mut s, 0, "Cinder Whelp");
    }
    give(&mut s, 0, "Call the Watch");
    let s = settle(play(s, 0, "Call the Watch", None));
    assert_eq!(s.allies_of(0).count(), MAX_ALLIES);
    // A full side can't play another Ally.
    let mut s = s;
    let c = give(&mut s, 0, "Kiln Brawler");
    assert!(!s.can_play(0, c));
}

#[test]
fn allies_enter_with_arrival_effects() {
    let mut s = duel();
    give(&mut s, 0, "Crucible Knight");
    let s = settle(play(s, 0, "Crucible Knight", None));
    assert_eq!(resolve(&s, 1), 19);
    assert_eq!(s.allies_of(0).count(), 1);
    let mut s = s;
    give(&mut s, 0, "Night Watch Captain");
    let s = settle(play(s, 0, "Night Watch Captain", None));
    assert_eq!(s.allies_of(0).count(), 3);
    let mut s = s;
    s.players[0].spark = 10;
    give(&mut s, 0, "Velvet Duchess");
    let s = settle(play(s, 0, "Velvet Duchess", None));
    assert_eq!((resolve(&s, 0), resolve(&s, 1)), (23, 16));
    let mut s = s;
    give(&mut s, 0, "Glass Familiar");
    let s = settle(play(s, 0, "Glass Familiar", None));
    assert_eq!(s.players[0].hand.len(), 1);
    let mut s = duel();
    let a = put_ally(&mut s, 1, "Cinder Whelp");
    let b = put_ally(&mut s, 1, "Watchtower Warden");
    give(&mut s, 0, "Forge Colossus");
    let s = settle(play(s, 0, "Forge Colossus", None));
    assert!(s.ally(a).is_none());
    assert_eq!(s.ally(b).unwrap().damage, 2);
    let mut s = s;
    s.players[0].gutter.push(id("Ember Dart"));
    s.players[0].spark = 10;
    give(&mut s, 0, "Retort Archivist");
    let s = settle(play(s, 0, "Retort Archivist", None));
    assert!(s.players[0].hand.contains(&id("Ember Dart")));
}

#[test]
fn abilities_spend_the_ally_and_use_the_stack() {
    let mut s = duel();
    let j = put_ally(&mut s, 0, "Spark Juggler");
    let s = input(s, 0, "ability", json!(format!("a{j}")));
    assert_eq!(s.selecting, Some(Source::Ability(j)));
    let s = input(s, 0, "target", json!("foe"));
    assert_eq!(s.stack.len(), 1);
    assert!(s.ally(j).unwrap().spent);
    assert_eq!(s.players[0].spark, 9);
    let s = settle(s);
    assert_eq!(resolve(&s, 1), 19);
    // Spent now, so it can't be used again.
    assert!(!s.can_use(0, j));

    let mut s = duel();
    let t = put_ally(&mut s, 0, "Tidewater Scholar");
    let fence = put_ally(&mut s, 0, "Night Market Fence");
    let h = put_ally(&mut s, 0, "Hearth-Lamp Keeper");
    let s = settle(input(s, 0, "ability", json!(format!("a{t}"))));
    let s = settle(input(s, 0, "ability", json!(format!("a{fence}"))));
    let s = settle(input(s, 0, "ability", json!(format!("a{h}"))));
    assert_eq!(s.players[0].hand.len(), 1);
    assert_eq!(s.players[1].gutter.len(), 1);
    assert_eq!(resolve(&s, 0), 22);
    assert_eq!(s.players[0].spark, 6);

    // Even a newly arrived Ally may use its ability, as long as it's ready.
    let mut s = duel();
    let uid = s.next_uid;
    add_ally(&mut s, 0, id("Spark Juggler"));
    assert!(s.can_use(0, uid));
    s.players[0].spark = 0;
    assert!(!s.can_use(0, uid), "needs the Spark");
}

#[test]
fn charms_work_while_in_play() {
    // Start-of-turn charms.
    let mut s = duel();
    for name in [
        "Lantern Over the Door",
        "Smouldering Brazier",
        "Lantern Parade",
    ] {
        give(&mut s, 0, name);
        s = settle(play(s, 0, name, None));
    }
    assert_eq!(s.players[0].charms.len(), 3);
    let s = press(press(s, 0, "end_turn"), 1, "end_turn");
    assert_eq!(resolve(&s, 0), 22);
    assert_eq!(resolve(&s, 1), 19);
    assert_eq!(s.allies_of(0).count(), 1);

    // Oiled Blades: +1 power.
    let mut s = duel();
    let a = put_ally(&mut s, 0, "Cinder Whelp");
    give(&mut s, 0, "Oiled Blades");
    let s = settle(play(s, 0, "Oiled Blades", None));
    assert_eq!(s.power(s.ally(a).unwrap()), 2);

    // Sublimation Coil: Feints cost 1 less, never below 0.
    let mut s = duel();
    give(&mut s, 0, "Sublimation Coil");
    let mut s = settle(play(s, 0, "Sublimation Coil", None));
    assert_eq!(s.cost(0, id("Ember Dart")), 0);
    assert_eq!(s.cost(0, id("Kindly Glow")), 0);
    assert_eq!(s.cost(0, id("Kiln Brawler")), 2);
    s.players[0].spark = 0;
    give(&mut s, 0, "Ember Dart");
    let s = settle(play(s, 0, "Ember Dart", Some("foe")));
    assert_eq!(resolve(&s, 1), 18);

    // Whispering Gallery: the opponent loses 1 per Feint.
    let mut s = duel();
    give(&mut s, 0, "Whispering Gallery");
    let mut s = settle(play(s, 0, "Whispering Gallery", None));
    give(&mut s, 1, "Kindly Glow");
    give(&mut s, 0, "Blazing Lunge");
    let s = play(s, 0, "Blazing Lunge", None);
    let s = play(s, 1, "Kindly Glow", None);
    assert_eq!(resolve(&s, 1), 19);
    let s = settle(s);
    assert_eq!(resolve(&s, 1), 21 - 3);
    // Own Feints are free of the toll.
    let mut s = s;
    give(&mut s, 0, "Kindly Glow");
    let s = settle(play(s, 0, "Kindly Glow", None));
    assert_eq!(resolve(&s, 0), 22);
}

#[test]
fn charm_and_ally_limits() {
    let mut s = duel();
    for name in [
        "Lantern Over the Door",
        "Smouldering Brazier",
        "Lantern Parade",
        "Oiled Blades",
    ] {
        s.players[0].spark = 10;
        give(&mut s, 0, name);
        s = settle(play(s, 0, name, None));
    }
    let c = give(&mut s, 0, "Whispering Gallery");
    assert!(!s.can_play(0, c));
}

/// Every pool card can be played and resolved in a busy position.
#[test]
fn every_card_plays_and_resolves() {
    for cid in 0..POOL_SIZE as CardId {
        let mut s = duel();
        put_ally(&mut s, 0, "Kiln Brawler");
        put_ally(&mut s, 1, "Watchtower Warden");
        s.players[0].gutter.push(id("Ember Dart"));
        s.players[1].hand.push(id("Kindly Glow"));
        let c = card(cid);
        if let Some(n) = c.negation() {
            // Something for it to answer: Bo plays on his turn.
            s.active = 1;
            let bait = match n {
                Neg::Strike => None,
                Neg::Feint => Some(give(&mut s, 1, "Ember Dart")),
                Neg::CardOrAbility => Some(give(&mut s, 1, "Blazing Lunge")),
            };
            s = match bait {
                Some(b) if card(b).needs_target() => {
                    let s = input(s, 1, "play", json!(format!("c{b}")));
                    input(s, 1, "target", json!("foe"))
                }
                Some(b) => input(s, 1, "play", json!(format!("c{b}"))),
                None => {
                    let uid = s.allies_of(1).next().unwrap().uid;
                    input(s, 1, "strike", json!(format!("a{uid}")))
                }
            };
            assert_eq!(s.stack.len(), 1, "{}", c.name);
        }
        s.players[0].hand.push(cid);
        assert!(s.can_play(0, cid), "{} can't be played", c.name);
        let mut s = input(s, 0, "play", json!(format!("c{cid}")));
        if s.selecting.is_some() {
            let t = s.targets(0, c.target)[0];
            let v = match t {
                Target::Player(0) => "me".to_string(),
                Target::Player(_) => "foe".to_string(),
                Target::Ally(u) => format!("a{u}"),
            };
            s = input(s, 0, "target", json!(v));
        }
        assert!(
            s.stack
                .iter()
                .any(|i| i.kind == ItemKind::Card { card: cid }),
            "{} isn't on the stack",
            c.name
        );
        let s = settle(s);
        assert!(!s.players[0].hand.contains(&cid) || c.effects.contains(&Eff::Recall));
        let _ = view::table(&s);
        let _ = view::phone(&s, 0);
    }
}

// ---------------------------------------------------------------------------
// The stack
// ---------------------------------------------------------------------------

#[test]
fn a_response_to_a_response_resolves_last_in_first_out() {
    let mut s = duel();
    give(&mut s, 0, "Blazing Lunge");
    give(&mut s, 0, "Thought Better Of");
    give(&mut s, 1, "Recant the Formula");
    let s = play(s, 0, "Blazing Lunge", None);
    assert_eq!((s.priority, s.passes), (1, 1));
    let s = play(s, 1, "Recant the Formula", None);
    assert_eq!(s.priority, 0);
    let s = play(s, 0, "Thought Better Of", None);
    assert_eq!(s.stack.len(), 3);
    assert_eq!(s.deepest_chain, 3);
    // Bo passes: Thought Better Of resolves and negates Recant.
    let s = press(s, 1, "pass");
    assert_eq!(s.stack.len(), 1);
    assert_eq!(s.players[1].gutter, vec![id("Recant the Formula")]);
    // Priority went back to the active player.
    assert_eq!((s.priority, s.passes), (0, 0));
    let s = press(press(s, 0, "pass"), 1, "pass");
    assert!(s.stack.is_empty());
    assert_eq!(resolve(&s, 1), 17);
    let order: Vec<&String> = s.log.iter().filter(|l| l.ends_with("resolves.")).collect();
    assert_eq!(
        order,
        vec!["Thought Better Of resolves.", "Blazing Lunge resolves."]
    );
    assert_eq!(view::table(&s).extra["deepest_chain"], json!(3));
}

#[test]
fn a_negation_removes_the_item_below_it() {
    let mut s = duel();
    give(&mut s, 0, "Blazing Lunge");
    give(&mut s, 1, "Recant the Formula");
    let s = play(s, 0, "Blazing Lunge", None);
    let s = settle(play(s, 1, "Recant the Formula", None));
    assert_eq!(resolve(&s, 1), 20);
    assert_eq!(s.players[0].gutter, vec![id("Blazing Lunge")]);
    assert!(s.log.iter().any(|l| l == "Blazing Lunge is negated."));
}

#[test]
fn levy_negates_only_when_the_owner_cannot_pay() {
    let mut s = duel();
    give(&mut s, 0, "Blazing Lunge");
    give(&mut s, 1, "Quicksilver Levy");
    let s = play(s, 0, "Blazing Lunge", None);
    let s = settle(play(s, 1, "Quicksilver Levy", None));
    assert_eq!(resolve(&s, 1), 17);
    assert_eq!(s.players[0].spark, 10 - 2 - 2);

    let mut s = duel();
    s.players[0].spark = 3;
    give(&mut s, 0, "Blazing Lunge");
    give(&mut s, 1, "Quicksilver Levy");
    let s = play(s, 0, "Blazing Lunge", None);
    let s = settle(play(s, 1, "Quicksilver Levy", None));
    assert_eq!(resolve(&s, 1), 20);
}

#[test]
fn feint_negation_only_answers_feints_and_strike_negation_only_strikes() {
    let mut s = duel();
    give(&mut s, 0, "Blazing Lunge");
    let st = give(&mut s, 1, "Thought Better Of");
    let fr = give(&mut s, 1, "Flare Riposte");
    let s = play(s, 0, "Blazing Lunge", None);
    assert!(!s.can_play(1, st));
    assert!(!s.can_play(1, fr));
    assert!(!s.can_respond_now(1));

    let mut s = duel();
    let a = put_ally(&mut s, 0, "Kiln Brawler");
    let fr = give(&mut s, 1, "Flare Riposte");
    let s = input(s, 0, "strike", json!(format!("a{a}")));
    assert!(s.can_play(1, fr));
    let s = settle(play(s, 1, "Flare Riposte", None));
    assert_eq!(s.phase, Phase::Main);
    assert!(s.combat.is_none());
    assert!(s.ally(a).unwrap().spent, "the striker stays spent");
    assert_eq!((resolve(&s, 0), resolve(&s, 1)), (18, 20));
}

#[test]
fn a_fizzled_card_does_nothing() {
    let mut s = duel();
    let a = put_ally(&mut s, 1, "Kiln Brawler");
    give(&mut s, 0, "Volatile Solution");
    give(&mut s, 1, "Reflux Tincture");
    let s = play(s, 0, "Volatile Solution", Some(&format!("a{a}")));
    // Bo returns his own Ally first; the Gambit loses its target.
    let s = settle(play(s, 1, "Reflux Tincture", Some(&format!("a{a}"))));
    assert_eq!(s.players[0].hand.len(), 0, "no card drawn");
    assert!(s.log.iter().any(|l| l.contains("fizzles")));
}

#[test]
fn reaction_windows_wait_on_the_priority_seat_with_fresh_prompts() {
    let mut s = duel();
    give(&mut s, 0, "Blazing Lunge");
    give(&mut s, 1, "Ember Dart");
    let turn = waits(&s);
    assert_eq!(turn.len(), 1);
    assert!(!turn[0].reaction);
    assert_eq!(turn[0].on_timeout, OnTimeout::Rules);
    let s = play(s, 0, "Blazing Lunge", None);
    let w1 = waits(&s);
    assert_eq!(w1.len(), 1);
    assert_eq!(w1[0].seat, 1);
    assert!(w1[0].reaction);
    assert_eq!(
        w1[0].on_timeout,
        OnTimeout::Input {
            input: PhoneInput::press("pass")
        }
    );
    assert_eq!(w1[0].live_timeout_ms, REACT_LIVE_MS);
    let s = play(s, 1, "Ember Dart", Some("foe"));
    let w2 = waits(&s);
    assert_eq!(w2[0].seat, 0);
    let s = press(s, 0, "pass");
    let w3 = waits(&s);
    assert_eq!(w3[0].seat, 0, "the active player gets priority again");
    assert!(w3[0].reaction);
    assert_ne!(w3[0].prompt, w2[0].prompt);
    // A rejected input keeps the same window (and deadline).
    let again = press(s.clone(), 1, "pass");
    assert_eq!(waits(&again), w3);
}

#[test]
fn passing_while_picking_a_target_cancels_the_pick() {
    let mut s = duel();
    give(&mut s, 0, "Blazing Lunge");
    give(&mut s, 1, "Ember Dart");
    let s = play(s, 0, "Blazing Lunge", None);
    let s = input(s, 1, "play", json!(format!("c{}", id("Ember Dart"))));
    assert!(s.selecting.is_some());
    // The window's timeout sends "pass".
    let s = press(s, 1, "pass");
    assert!(s.selecting.is_none());
    assert!(s.stack.is_empty());
    assert_eq!(resolve(&s, 1), 17);
    assert_eq!(s.players[1].hand.len(), 1);
}

// ---------------------------------------------------------------------------
// can_respond
// ---------------------------------------------------------------------------

#[test]
fn can_respond_needs_priority_an_affordable_legal_answer() {
    let mut s = duel();
    give(&mut s, 0, "Blazing Lunge");
    let s0 = play(s.clone(), 0, "Blazing Lunge", None);
    let r = Riposte;
    // Nothing in hand.
    assert!(!r.can_respond(&s0, 1).unwrap());
    // Ana doesn't hold priority.
    let mut s1 = s0.clone();
    s1.players[0].hand.push(id("Ember Dart"));
    assert!(!r.can_respond(&s1, 0).unwrap());
    // A Feint Bo can afford.
    let mut s2 = s0.clone();
    s2.players[1].hand.push(id("Ember Dart"));
    assert!(r.can_respond(&s2, 1).unwrap());
    s2.players[1].spark = 0;
    assert!(!r.can_respond(&s2, 1).unwrap());
    // A Gambit or Ally is never a response.
    let mut s3 = s0.clone();
    s3.players[1].hand = vec![id("Blazing Lunge"), id("Kiln Brawler")];
    assert!(!r.can_respond(&s3, 1).unwrap());
    // A 0-cost Feint with no Spark.
    s3.players[1].spark = 0;
    s3.players[1].hand.push(id("Kindly Glow"));
    assert!(r.can_respond(&s3, 1).unwrap());
    // An Ally ability counts.
    let mut s4 = s0.clone();
    put_ally(&mut s4, 1, "Spark Juggler");
    assert!(r.can_respond(&s4, 1).unwrap());
    // A Feint that needs an Ally target, with no Allies about.
    let mut s5 = s0.clone();
    s5.players[1].hand.push(id("Alembic Burst"));
    assert!(!r.can_respond(&s5, 1).unwrap());
    // Outside a window, nobody can respond.
    assert!(!r.can_respond(&s, 0).unwrap());
    assert!(!r.can_respond(&s, 7).unwrap());
}

// ---------------------------------------------------------------------------
// Deck building
// ---------------------------------------------------------------------------

fn built(seed: u64, a: usize, b: usize) -> State {
    let s = Riposte.setup(&seats(), seed).unwrap().state;
    let s = press(s, 0, &format!("starter{a}"));
    press(s, 1, &format!("starter{b}"))
}

#[test]
fn deck_builder_takes_20_distinct_cards_or_a_starter() {
    let s = Riposte.setup(&seats(), 5).unwrap().state;
    let ui = view::phone(&s, 0);
    assert!(ui.node_count() < 500, "{}", ui.node_count());
    let twenty: Vec<String> = (0..20).map(|i| format!("c{}", i * 3)).collect();
    assert!(ui.accepts(&PhoneInput::with("deck", json!(twenty))));
    assert!(!ui.accepts(&PhoneInput::with("deck", json!(twenty[..19]))));
    let mut dup = twenty.clone();
    dup[19] = dup[0].clone();
    assert!(!ui.accepts(&PhoneInput::with("deck", json!(dup))));
    let mut token = twenty.clone();
    token[0] = format!("c{LAMPLIGHTER}");
    assert!(!ui.accepts(&PhoneInput::with("deck", json!(token))));
    assert!(ui.accepts(&PhoneInput::press("starter2")));
    // The rules reject the same things on their own.
    assert_eq!(input(s.clone(), 0, "deck", json!(dup)), s);
    assert_eq!(input(s.clone(), 0, "deck", json!(token)), s);
    assert_eq!(press(s.clone(), 0, "starter9"), s);

    let s = input(s, 0, "deck", json!(twenty));
    assert!(s.players[0].built);
    assert_eq!(s.phase, Phase::Building);
    let mine: Vec<CardId> = (0..20).map(|i| (i * 3) as CardId).collect();
    let mut deck = s.players[0].deck.clone();
    deck.sort();
    assert_eq!(deck, mine);
    // A built seat can't build again, and sees no builder.
    assert_eq!(press(s.clone(), 0, "starter0"), s);
    assert!(!view::phone(&s, 0).has_input("deck"));
    assert_eq!(waits(&s).len(), 1);

    let s = press(s, 1, "starter1");
    assert_eq!(s.phase, Phase::Main);
    let active = s.active as usize;
    assert_eq!(s.players[active].hand.len(), 6);
    assert_eq!(s.players[1 - active].hand.len(), 5);
    assert_eq!(s.players[active].spark, 1);
    let total: usize = s.players[0].hand.len() + s.players[0].deck.len();
    assert_eq!(total, 20);
}

#[test]
fn deck_building_times_out_into_a_starter() {
    let s = Riposte.setup(&seats(), 9).unwrap();
    assert_eq!(s.waiting_on.len(), 2);
    assert!(s.waiting_on.iter().all(|w| w.prompt == "Build your deck"));
    let s = Riposte.on_timeout(s.state, 1, 0).unwrap().state;
    assert!(s.players[1].built);
    assert_eq!(s.players[1].deck.len(), 20);
    let o = Riposte.on_timeout(s, 0, 0).unwrap();
    assert_eq!(o.state.phase, Phase::Main);
    assert!(o.waiting_on[0].prompt.starts_with("Turn 1"));
}

// ---------------------------------------------------------------------------
// Turns and combat
// ---------------------------------------------------------------------------

#[test]
fn spark_grows_to_ten_and_refills_and_allies_ready() {
    let mut s = built(3, 0, 1);
    let first = s.active;
    let a = put_ally(&mut s, first, "Kiln Brawler");
    s.ally_mut(a).unwrap().spent = true;
    s.players[first as usize].spark = 0;
    let s2 = press(press(s.clone(), first, "end_turn"), 1 - first, "end_turn");
    assert_eq!(s2.active, first);
    assert!(
        !s2.ally(a).unwrap().spent,
        "Allies ready at the start of your turn"
    );
    assert_eq!(s2.players[first as usize].spark_max, 2);
    assert_eq!(s2.players[first as usize].spark, 2, "Spark refills");
    for _ in 0..30 {
        if s.phase != Phase::Main {
            break;
        }
        let seat = s.active;
        s = press(s, seat, "end_turn");
    }
    // Someone decked out eventually.
    assert_eq!(s.phase, Phase::Over);
    assert!(s.players.iter().any(|p| p.decked));
    let mut s = built(3, 0, 1);
    for _ in 0..24 {
        let seat = s.active;
        s = press(s, seat, "end_turn");
    }
    assert_eq!(s.players[0].spark_max, 10);
    assert_eq!(s.players[0].spark, 10);
}

#[test]
fn drawing_from_an_empty_deck_loses() {
    let mut s = duel();
    s.players[1].deck.clear();
    let o = step(s, 0, &PhoneInput::press("end_turn"));
    assert_eq!(o.state.phase, Phase::Over);
    let r = o.finished.unwrap();
    assert_eq!(r.ranking, vec![vec![0], vec![1]]);
    assert!(o.waiting_on.is_empty());
    // And a draw effect on an empty deck loses too.
    let mut s = duel();
    s.players[0].deck = vec![id("Ember Dart")];
    give(&mut s, 0, "Distilled Insight");
    let s = settle(play(s, 0, "Distilled Insight", None));
    assert_eq!(s.result.unwrap().ranking, vec![vec![1], vec![0]]);
}

#[test]
fn falling_to_zero_resolve_ends_the_game_and_both_falling_is_a_draw() {
    let mut s = duel();
    s.players[1].resolve = 3;
    give(&mut s, 0, "Blazing Lunge");
    let s = settle(play(s, 0, "Blazing Lunge", None));
    assert_eq!(s.phase, Phase::Over);
    assert_eq!(s.result.as_ref().unwrap().ranking, vec![vec![0], vec![1]]);
    assert!(view::phone(&s, 0).inputs().is_empty());

    let mut s = duel();
    s.players[0].resolve = 1;
    give(&mut s, 0, "Hollow Bargain");
    let s = settle(play(s, 0, "Hollow Bargain", None));
    assert_eq!(s.result.as_ref().unwrap().ranking, vec![vec![1], vec![0]]);

    let mut s = duel();
    s.players[0].resolve = 0;
    s.players[1].decked = true;
    assert!(check_over(&mut s));
    assert_eq!(s.result.unwrap().ranking, vec![vec![0, 1]]);
}

#[test]
fn strikes_go_on_the_stack_then_blocks_then_a_clash() {
    let mut s = duel();
    let brawler = put_ally(&mut s, 0, "Kiln Brawler"); // 3/1
    let whelp = put_ally(&mut s, 0, "Cinder Whelp"); // 1/1
    let warden = put_ally(&mut s, 1, "Watchtower Warden"); // 4/6
    let s = input(
        s,
        0,
        "strike",
        json!([format!("a{brawler}"), format!("a{whelp}")]),
    );
    assert!(s.struck);
    assert_eq!(s.stack.len(), 1);
    assert!(s.ally(brawler).unwrap().spent);
    assert_eq!(s.priority, 1);
    let s = press(s, 1, "pass");
    assert_eq!(s.phase, Phase::Blocking);
    let w = waits(&s);
    assert_eq!(w[0].seat, 1);
    assert!(!w[0].reaction);
    assert_eq!(
        w[0].on_timeout,
        OnTimeout::Input {
            input: PhoneInput::press("blocks_done")
        }
    );
    let ui = view::phone(&s, 1);
    let block = format!("a{warden}>a{brawler}");
    assert!(ui.accepts(&PhoneInput::with("block", block.clone())));
    let s = input(s, 1, "block", json!(block));
    // The Warden is used up; it can't block twice.
    assert!(s.block_options().is_empty());
    let s = press(s, 1, "blocks_done");
    assert_eq!(s.phase, Phase::Main);
    assert_eq!(s.stack.last().unwrap().kind, ItemKind::Clash);
    assert_eq!(s.priority, 0, "the attacker acts first before damage");
    let s = settle(s);
    assert!(s.ally(brawler).is_none());
    assert_eq!(s.ally(warden).unwrap().damage, 3);
    assert_eq!(resolve(&s, 1), 19);
    // Once a turn.
    assert!(s.strikers(0).is_empty());
}

#[test]
fn fresh_allies_wait_a_turn_unless_swift_and_elusive_cannot_be_blocked() {
    let mut s = duel();
    give(&mut s, 0, "Kiln Brawler");
    give(&mut s, 0, "Cinder Whelp");
    let s = settle(play(s, 0, "Kiln Brawler", None));
    let s = settle(play(s, 0, "Cinder Whelp", None));
    let names: Vec<&str> = s
        .strikers(0)
        .iter()
        .map(|u| card(s.ally(*u).unwrap().card).name)
        .collect();
    assert_eq!(names, vec!["Cinder Whelp"]);

    let mut s = duel();
    let sprite = put_ally(&mut s, 0, "Bead of Mercury");
    put_ally(&mut s, 1, "Watchtower Warden");
    let s = settle(input(s, 0, "strike", json!(format!("a{sprite}"))));
    // Nobody can block, so there is no block step.
    assert_eq!(s.phase, Phase::Main);
    assert!(s.combat.is_none());
    assert_eq!(resolve(&s, 1), 19);
}

#[test]
fn steadfast_strikers_stay_ready_and_spent_allies_cannot_block() {
    let mut s = duel();
    let f = put_ally(&mut s, 0, "Gilded Fencer");
    let tired = put_ally(&mut s, 1, "Watchtower Warden");
    s.ally_mut(tired).unwrap().spent = true;
    let s = settle(input(s, 0, "strike", json!(format!("a{f}"))));
    assert!(!s.ally(f).unwrap().spent);
    assert_eq!(s.phase, Phase::Main, "the spent Warden can't block");
    assert_eq!(resolve(&s, 1), 17);
}

#[test]
fn tricks_during_the_clash_change_the_damage() {
    let mut s = duel();
    let b = put_ally(&mut s, 0, "Kiln Brawler");
    let w = put_ally(&mut s, 1, "Lamp-post Sentinel"); // 1/4
    give(&mut s, 0, "Flashpowder Pinch");
    let s = settle(input(s, 0, "strike", json!(format!("a{b}"))));
    let s = input(s, 1, "block", json!(format!("a{w}>a{b}")));
    let s = press(s, 1, "blocks_done");
    let s = settle(play(s, 0, "Flashpowder Pinch", Some(&format!("a{b}"))));
    assert!(s.ally(w).is_none(), "5 power beats 4 guard");
    assert!(s.ally(b).is_none(), "and the Sentinel hit back for 1");
}

#[test]
fn turn_timeout_ends_the_turn_and_block_timeout_means_no_blocks() {
    let s = duel();
    let s = time_out(s, 0);
    assert_eq!(s.active, 1);
    // A timeout for a seat that isn't being waited on changes nothing.
    assert_eq!(time_out(s.clone(), 0), s);
    let mut s = s;
    let a = put_ally(&mut s, 1, "Kiln Brawler");
    let s = settle(input(s, 1, "strike", json!(format!("a{a}"))));
    let s = time_out(s, 0);
    assert_eq!(s.phase, Phase::Main);
    let s = settle(s);
    assert_eq!(resolve(&s, 0), 17);
}

// ---------------------------------------------------------------------------
// Robustness
// ---------------------------------------------------------------------------

#[test]
fn garbage_input_changes_nothing() {
    let mut s = duel();
    give(&mut s, 0, "Ember Dart");
    let a = put_ally(&mut s, 0, "Kiln Brawler");
    let junk = [
        PhoneInput::press("play"),
        PhoneInput::with("play", "c9999"),
        PhoneInput::with("play", "zzz"),
        PhoneInput::with("play", json!({"x": 1})),
        PhoneInput::with("target", "foe"),
        PhoneInput::with("strike", json!([])),
        PhoneInput::with("strike", json!(["a999"])),
        PhoneInput::with("strike", json!([format!("a{a}"), format!("a{a}")])),
        PhoneInput::with("ability", "a1"),
        PhoneInput::with("block", "a1>a2"),
        PhoneInput::press("pass"),
        PhoneInput::press("blocks_done"),
        PhoneInput::press("starter0"),
        PhoneInput::with("deck", json!(["c1"])),
        PhoneInput::press("cancel"),
        PhoneInput::press("nonsense"),
    ];
    for j in &junk {
        for seat in [0, 1, 5] {
            let o = Riposte.on_action(s.clone(), seat, j).unwrap();
            assert_eq!(o.state, s, "{j:?} from {seat}");
        }
    }
    // Bo can't end Ana's turn.
    assert_eq!(press(s.clone(), 1, "end_turn"), s);
    assert!(Riposte.setup(&[Seat::new(0, "A")], 1).is_err());
}

#[test]
fn fuzzing_with_random_inputs_never_panics() {
    for seed in 0..12u64 {
        let mut s = Riposte.setup(&seats(), seed).unwrap().state;
        let mut rng = Rng::new(seed);
        for n in 0..1500 {
            if s.phase == Phase::Over {
                break;
            }
            let seat = rng.below(2);
            let ui = view::phone(&s, seat);
            assert!(ui.node_count() < 500);
            let input = if rng.below(10) == 0 {
                PhoneInput::with(
                    ["play", "target", "pass", "strike"][rng.below(4) as usize],
                    "a1",
                )
            } else {
                match random_input(&ui, &mut rng, &[("pass", 30), ("end_turn", 10)]) {
                    Some(i) => i,
                    None => {
                        s = time_out(s, rng.below(2));
                        continue;
                    }
                }
            };
            s = Riposte.on_action(s, seat, &input).unwrap().state;
            if n % 50 == 0 {
                s = time_out(s, rng.below(2));
            }
            check_table(&Riposte.table_view(&s).unwrap()).unwrap();
        }
    }
}

/// Hidden information never reaches the table or the other phone.
#[test]
fn views_never_show_the_other_hand_or_deck_order() {
    // Disjoint decks, right after the deal.
    let s = built(11, 0, 1);
    for seat in 0..2u32 {
        let foe = 1 - seat;
        let table = serde_json::to_string(&Riposte.table_view(&s).unwrap()).unwrap();
        let phone = ui_json(&s, seat);
        for c in s.players[foe as usize]
            .hand
            .iter()
            .chain(&s.players[foe as usize].deck)
        {
            let name = card(*c).name;
            assert!(!table.contains(name), "table shows {name}");
            assert!(!phone.contains(name), "seat {seat} sees {name}");
        }
        for c in &s.players[seat as usize].deck {
            assert!(!table.contains(card(*c).name));
        }
    }
    // At every point of a bot game, changing hidden cards changes no view
    // except the owner's.
    room_game(4, Pace::Live, 4000);
    for (n, state) in GAME_STATES.with(|g| g.borrow().clone()).iter().enumerate() {
        if state.phase == Phase::Building || n % 3 != 0 {
            continue;
        }
        for hidden in 0..2usize {
            let mut other_state = state.clone();
            let p = &mut other_state.players[hidden];
            let k = p.hand.len();
            p.hand = (0..k)
                .map(|i| ((i * 7 + 5) % POOL_SIZE) as CardId)
                .collect();
            p.deck.reverse();
            if !p.deck.is_empty() {
                p.deck.rotate_left(1);
            }
            assert_eq!(
                Riposte.table_view(state).unwrap(),
                Riposte.table_view(&other_state).unwrap()
            );
            let viewer = 1 - hidden as u32;
            assert_eq!(
                view::phone(state, viewer),
                view::phone(&other_state, viewer)
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Whole games in a room
// ---------------------------------------------------------------------------

thread_local! {
    static GAME_STATES: std::cell::RefCell<Vec<State>> = const { std::cell::RefCell::new(Vec::new()) };
}

struct Played {
    room: Room<Riposte>,
    inputs: usize,
}

fn room_game(seed: u64, pace: Pace, bound: usize) -> Played {
    GAME_STATES.with(|g| g.borrow_mut().clear());
    let mut room = Room::new(Riposte, seed).with_pace(pace);
    room.player_joined(10, "Ana".into(), "#e4572e".into(), 0);
    room.player_joined(11, "Bo".into(), String::new(), 0);
    room.start(0).unwrap();
    let mut rng = Rng::new(seed.wrapping_mul(31).wrapping_add(7));
    let mut now = 0u64;
    let mut inputs = 0;
    while room.finished().is_none() {
        assert!(room.paused().is_none(), "paused: {:?}", room.paused());
        assert!(inputs < bound, "seed {seed} took over {bound} inputs");
        let state = room.state().unwrap().clone();
        assert!(serde_json::to_string(&state).unwrap().len() < 100_000);
        check_table(&room.table_view().unwrap().unwrap()).unwrap();
        GAME_STATES.with(|g| g.borrow_mut().push(state));
        now += 3;
        let Some(seat) = room.waits().first().map(|w| w.wait.seat) else {
            panic!("seed {seed}: nobody is being waited on");
        };
        let player = room.player_in(seat).unwrap();
        let ui = room.current_ui(player).unwrap().clone();
        assert!(ui.node_count() < 500);
        // The host shows a timer above the screen; the bot copes.
        let shown = Widget::column(vec![
            Widget::Timer {
                ends_in_ms: 1000,
                total_ms: Some(8000),
                label: "Respond?".into(),
            },
            ui.clone(),
        ]);
        match bot::choose(&shown, &mut rng) {
            Some(input) => {
                assert!(ui.accepts(&input), "seed {seed}: bot sent {input:?}");
                room.handle_input(player, input, now).unwrap();
                inputs += 1;
            }
            None => {
                now = room.next_deadline().unwrap_or(now).max(now);
                room.tick(now);
            }
        }
    }
    Played { room, inputs }
}

fn deepest(room: &Room<Riposte>) -> u64 {
    room.table_view().unwrap().unwrap()["extra"]["deepest_chain"]
        .as_u64()
        .unwrap()
}

#[test]
fn bot_games_finish_live_with_nested_chains() {
    let mut total = 0;
    let mut deep = 0;
    for seed in 0..32 {
        let p = room_game(seed, Pace::Live, 2500);
        total += p.inputs;
        if deepest(&p.room) >= 2 {
            deep += 1;
        }
        let r = p.room.finished().unwrap();
        assert!(!r.ranking.is_empty());
    }
    eprintln!(
        "live: {} inputs per game, {deep}/32 reached depth 2+",
        total / 32
    );
    assert!(deep >= 8, "only {deep} games had a nested chain");
}

#[test]
fn bot_games_finish_async_and_seats_that_cannot_respond_are_passed() {
    let mut total = 0;
    let mut deep = 0;
    for seed in 100..132 {
        let p = room_game(seed, Pace::Async, 2500);
        total += p.inputs;
        if deepest(&p.room) >= 2 {
            deep += 1;
        }
        let passed = p
            .room
            .log()
            .iter()
            .filter(|a| matches!(a, LoggedAction::Passed { .. }))
            .count();
        assert!(passed > 0, "seed {seed}: nobody was auto-passed");
    }
    eprintln!(
        "async: {} inputs per game, {deep}/32 reached depth 2+",
        total / 32
    );
    assert!(deep >= 8);
}

#[test]
fn games_replay_exactly_from_seed_and_log() {
    for seed in [1u64, 2, 3, 200] {
        let pace = if seed == 200 { Pace::Async } else { Pace::Live };
        let p = room_game(seed, pace, 2500);
        let replayed = replay(&Riposte, &p.room.seats(), seed, p.room.log()).unwrap();
        assert_eq!(&replayed, p.room.state().unwrap());
    }
}

#[test]
fn timeouts_alone_finish_the_game() {
    for pace in [Pace::Live, Pace::Async] {
        let mut room = Room::new(Riposte, 77).with_pace(pace);
        room.player_joined(1, "Ana".into(), String::new(), 0);
        room.player_joined(2, "Bo".into(), String::new(), 0);
        room.start(0).unwrap();
        let mut ticks = 0;
        while room.finished().is_none() {
            let next = room.next_deadline().expect("always a deadline");
            room.tick(next);
            ticks += 1;
            assert!(ticks < 200, "still going after {ticks} timeouts");
            assert!(room.paused().is_none());
        }
        let s = room.state().unwrap();
        assert!(s.players.iter().any(|p| p.decked));
    }
}
