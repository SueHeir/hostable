//! A bot that plays Tideholm from a phone screen alone.
//!
//! It reads only the widget tree a seat would see (possibly wrapped by the
//! host in a column with a timer) and returns an input that screen accepts.
//! It builds whenever it can, trades toward what it needs, plays Almanac
//! cards and otherwise ends its turn.

use hostable_sdk::prelude::*;

use crate::{COST_CAUSEWAY, COST_HUT, COST_LODGE, RES, RES_KEYS};

/// Every widget in the tree, including disabled ones.
fn walk<'a>(w: &'a Widget, out: &mut Vec<&'a Widget>) {
    out.push(w);
    match w {
        Widget::Column { children }
        | Widget::Row { children }
        | Widget::Grid { children, .. }
        | Widget::Sheet { children, .. } => {
            for c in children {
                walk(c, out);
            }
        }
        Widget::Tabs { tabs } => {
            for t in tabs {
                for c in &t.children {
                    walk(c, out);
                }
            }
        }
        _ => {}
    }
}

fn find<'a>(ui: &'a Widget, id: &str) -> Option<&'a Widget> {
    let mut all = Vec::new();
    walk(ui, &mut all);
    all.into_iter().find(|w| match w {
        Widget::Text { id: Some(i), .. } => i == id,
        other => other.input_id() == Some(id),
    })
}

/// The seat's own resource counts, from the `hand_counts` text.
fn hand(ui: &Widget) -> [u8; 5] {
    let mut out = [0u8; 5];
    if let Some(Widget::Text { value, .. }) = find(ui, "hand_counts") {
        for part in value.split('·') {
            let mut it = part.split_whitespace();
            if let (Some(name), Some(n)) = (it.next(), it.next()) {
                if let Some(r) = RES.iter().position(|x| *x == name) {
                    out[r] = n.parse().unwrap_or(0);
                }
            }
        }
    }
    out
}

fn options(w: &Widget) -> Vec<&ChoiceOption> {
    match w {
        Widget::Choice { options, .. } => options.iter().filter(|o| !o.disabled).collect(),
        _ => Vec::new(),
    }
}

fn all_options(w: Option<&Widget>) -> usize {
    match w {
        Some(Widget::Choice { options, .. }) => options.len(),
        _ => 0,
    }
}

fn pick<'a>(opts: &[&'a ChoiceOption], rng: &mut Rng) -> Option<&'a ChoiceOption> {
    if opts.is_empty() {
        None
    } else {
        Some(opts[rng.below(opts.len() as u32) as usize])
    }
}

fn leading_number(s: &str) -> u32 {
    s.split_whitespace()
        .next()
        .and_then(|n| n.parse().ok())
        .unwrap_or(0)
}

/// The option with the most pips (from a `"N pips: ..."` detail).
fn best_spot<'a>(opts: &[&'a ChoiceOption], rng: &mut Rng) -> Option<&'a ChoiceOption> {
    let best = opts.iter().map(|o| leading_number(&o.detail)).max()?;
    let top: Vec<&ChoiceOption> = opts
        .iter()
        .copied()
        .filter(|o| leading_number(&o.detail) == best)
        .collect();
    pick(&top, rng)
}

fn single(ui: &Widget, id: &str, value: &str) -> Option<PhoneInput> {
    let input = PhoneInput::with(id, value);
    ui.accepts(&input).then_some(input)
}

fn press(ui: &Widget, id: &str) -> Option<PhoneInput> {
    let input = PhoneInput::press(id);
    ui.accepts(&input).then_some(input)
}

fn choose_in(ui: &Widget, id: &str, rng: &mut Rng) -> Option<PhoneInput> {
    let w = ui.find_input(id)?;
    let o = pick(&options(w), rng)?;
    single(ui, id, &o.value)
}

fn list(ui: &Widget, id: &str, values: Vec<String>) -> Option<PhoneInput> {
    let input = PhoneInput::with(
        id,
        Value::Array(values.into_iter().map(Value::String).collect()),
    );
    ui.accepts(&input).then_some(input)
}

/// What the bot is saving for, given what the screen shows.
fn target(ui: &Widget) -> [u8; 5] {
    if all_options(find(ui, "build_lodge")) > 0 {
        let hand = hand(ui);
        // Lodges when copper and salt are near; huts otherwise.
        if all_options(find(ui, "build_hut")) == 0 || hand[4] + hand[2] >= 3 {
            return COST_LODGE;
        }
    }
    if all_options(find(ui, "build_hut")) > 0 {
        COST_HUT
    } else {
        COST_CAUSEWAY
    }
}

/// An offer (or counter) of one surplus card for one missing card.
fn swap_values(have: [u8; 5], goal: [u8; 5], rng: &mut Rng) -> Option<Vec<String>> {
    let surplus: Vec<usize> = (0..5).filter(|&r| have[r] > goal[r]).collect();
    let short: Vec<usize> = (0..5).filter(|&r| have[r] < goal[r]).collect();
    if surplus.is_empty() || short.is_empty() {
        return None;
    }
    let g = surplus[rng.below(surplus.len() as u32) as usize];
    let w = short[rng.below(short.len() as u32) as usize];
    Some(vec![
        format!("give:{}:1", RES_KEYS[g]),
        format!("want:{}:1", RES_KEYS[w]),
    ])
}

fn bank_trade(ui: &Widget, have: [u8; 5], goal: [u8; 5]) -> Option<PhoneInput> {
    let w = ui.find_input("bank_trade")?;
    for o in options(w) {
        let (a, b) = o.value.split_once('>')?;
        let a = RES_KEYS.iter().position(|k| *k == a)?;
        let b = RES_KEYS.iter().position(|k| *k == b)?;
        if have[b] < goal[b] && have[a] >= goal[a] + 4 {
            return single(ui, "bank_trade", &o.value);
        }
    }
    None
}

/// Pick a sensible input for this screen, or `None` if there is nothing to
/// do (e.g. waiting for others).
pub fn choose(ui: &Widget, rng: &mut Rng) -> Option<PhoneInput> {
    let have = hand(ui);

    // Setup.
    if let Some(w) = ui.find_input("place_hut") {
        let o = best_spot(&options(w), rng)?;
        return single(ui, "place_hut", &o.value);
    }
    if ui.has_input("place_causeway") {
        return choose_in(ui, "place_causeway", rng);
    }

    // The Squall.
    if let Some(Widget::Hand { cards, min, .. }) = ui.find_input("discard") {
        // Throw away what is most plentiful first.
        let mut counts = have;
        let mut values = Vec::new();
        for _ in 0..*min {
            let r = (0..5).max_by_key(|&r| (counts[r], r))?;
            if counts[r] == 0 {
                break;
            }
            counts[r] -= 1;
            values.push(format!("{}:{}", RES_KEYS[r], counts[r]));
        }
        if let Some(input) = list(ui, "discard", values) {
            return Some(input);
        }
        let mut all: Vec<String> = cards.iter().map(|c| c.value.clone()).collect();
        rng.shuffle(&mut all);
        all.truncate(*min as usize);
        return list(ui, "discard", all);
    }
    if let Some(w) = ui.find_input("squall_hex") {
        let opts = options(w);
        let theirs: Vec<&ChoiceOption> = opts
            .iter()
            .copied()
            .filter(|o| !o.detail.starts_with("(yours)") && !o.detail.ends_with("nobody"))
            .collect();
        let o = pick(&theirs, rng).or_else(|| pick(&opts, rng))?;
        return single(ui, "squall_hex", &o.value);
    }
    if ui.has_input("steal_from") {
        return choose_in(ui, "steal_from", rng);
    }

    // Almanac side steps.
    if ui.has_input("surveyor_edge") {
        return choose_in(ui, "surveyor_edge", rng);
    }
    if ui.has_input("surveyor_done") {
        return press(ui, "surveyor_done");
    }
    if let Some(w) = ui.find_input("bounty") {
        let goal = target(ui);
        let opts = options(w);
        let mut values: Vec<String> = Vec::new();
        let mut need = goal;
        for _ in 0..2 {
            let r = (0..5).find(|&r| have[r] < need[r]);
            let chosen = r
                .and_then(|r| {
                    opts.iter()
                        .find(|o| {
                            o.value.starts_with(&format!("{}:", RES_KEYS[r]))
                                && !values.contains(&o.value)
                        })
                        .map(|o| (r, o.value.clone()))
                })
                .or_else(|| {
                    opts.iter()
                        .find(|o| !values.contains(&o.value))
                        .map(|o| (5, o.value.clone()))
                });
            let Some((r, v)) = chosen else { break };
            if r < 5 {
                need[r] = need[r].saturating_sub(1);
            }
            values.push(v);
        }
        return list(ui, "bounty", values);
    }
    if ui.has_input("market") {
        let goal = target(ui);
        let r = (0..5)
            .max_by_key(|&r| (goal[r].saturating_sub(have[r]), rng.below(3)))
            .unwrap_or(0);
        return single(ui, "market", RES_KEYS[r]).or_else(|| choose_in(ui, "market", rng));
    }

    // Trades.
    if find(ui, "trade_pick").is_some() {
        if rng.below(100) < 85 {
            if let Some(i) = choose_in(ui, "trade_pick", rng) {
                return Some(i);
            }
        }
        return press(ui, "trade_cancel");
    }
    if ui.has_input("trade_decline") {
        let roll = rng.below(100);
        if roll < 45 {
            if let Some(i) = press(ui, "trade_accept") {
                return Some(i);
            }
        }
        if roll < 65 {
            if let Some(values) = swap_values(have, target(ui), rng) {
                if let Some(i) = list(ui, "counter", values) {
                    return Some(i);
                }
            }
        }
        return press(ui, "trade_decline");
    }
    if ui.has_input("trade_cancel") {
        // Waiting on answers to our own offer.
        return None;
    }

    // Start of turn.
    if ui.has_input("roll") {
        if rng.below(100) < 30 {
            if let Some(i) = single(ui, "play_card", "ferryman") {
                return Some(i);
            }
        }
        return press(ui, "roll");
    }

    // Main phase.
    if !ui.has_input("end_turn") {
        return None;
    }
    if let Some(w) = ui.find_input("build_lodge") {
        let o = best_spot(&options(w), rng)?;
        return single(ui, "build_lodge", &o.value);
    }
    if let Some(w) = ui.find_input("build_hut") {
        let o = best_spot(&options(w), rng)?;
        return single(ui, "build_hut", &o.value);
    }
    if ui.has_input("play_card") && rng.below(100) < 60 {
        if let Some(i) = choose_in(ui, "play_card", rng) {
            return Some(i);
        }
    }
    let goal = target(ui);
    let hut_spots = all_options(find(ui, "build_hut")) > 0;
    if let Some(w) = ui.find_input("build_causeway") {
        let opts = options(w);
        let leads: Vec<&ChoiceOption> = opts
            .iter()
            .copied()
            .filter(|o| !o.detail.is_empty())
            .collect();
        if !hut_spots || rng.below(100) < 20 {
            let o = pick(&leads, rng).or_else(|| pick(&opts, rng))?;
            return single(ui, "build_causeway", &o.value);
        }
    }
    if ui.has_input("buy_almanac") && (goal == COST_CAUSEWAY || rng.below(100) < 25) {
        return press(ui, "buy_almanac");
    }
    if let Some(i) = bank_trade(ui, have, goal) {
        return Some(i);
    }
    if ui.has_input("offer") && rng.below(100) < 30 {
        if let Some(values) = swap_values(have, goal, rng) {
            if let Some(i) = list(ui, "offer", values) {
                return Some(i);
            }
        }
    }
    press(ui, "end_turn")
}
