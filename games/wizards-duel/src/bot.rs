//! A bot that plays the duel from its phone screen alone, like a person
//! would. It reads only what the screen says (option labels and details),
//! so it works the same against a real host over sockets.
//!
//! It plays to win quickly: cast a spell that can hit a rival (damage
//! first), walk toward the nearest rival, sometimes wander toward new
//! mana, and end the turn claiming mana when it can.

use hostable_sdk::prelude::*;

fn find<'a>(ui: &'a Widget, id: &str) -> Option<Vec<&'a ChoiceOption>> {
    match ui.find_input(id)? {
        Widget::Choice { options, .. } => Some(options.iter().filter(|o| !o.disabled).collect()),
        _ => None,
    }
}

fn pick<'a>(options: &[&'a ChoiceOption], rng: &mut Rng) -> Option<&'a ChoiceOption> {
    if options.is_empty() {
        None
    } else {
        Some(options[rng.below(options.len() as u32) as usize])
    }
}

fn send(id: &str, o: &ChoiceOption) -> PhoneInput {
    PhoneInput::with(id, o.value.clone())
}

/// The bot's next input for this screen, or `None` when it has nothing
/// to do (not its turn). Every input is one the screen accepts.
pub fn choose(ui: &Widget, rng: &mut Rng) -> Option<PhoneInput> {
    let input = decide(ui, rng)?;
    ui.accepts(&input).then_some(input)
}

fn decide(ui: &Widget, rng: &mut Rng) -> Option<PhoneInput> {
    // Aiming: hit as many rivals as possible.
    if let Some(targets) = find(ui, "aim") {
        let most = targets
            .iter()
            .map(|o| hit_count(&o.detail))
            .max()
            .unwrap_or(0);
        let best: Vec<&ChoiceOption> = targets
            .iter()
            .copied()
            .filter(|o| hit_count(&o.detail) == most)
            .collect();
        if let Some(o) = pick(&best, rng) {
            return Some(send("aim", o));
        }
    }
    if ui.has_input("cancel") && !ui.has_input("aim") {
        return Some(PhoneInput::press("cancel"));
    }
    if let Some(spells) = find(ui, "cast") {
        let hitting: Vec<&ChoiceOption> = spells
            .iter()
            .copied()
            .filter(|o| o.detail.contains("can hit"))
            .collect();
        let hurting: Vec<&ChoiceOption> = hitting
            .iter()
            .copied()
            .filter(|o| o.detail.contains("damage") || o.detail.contains("burn"))
            .collect();
        // Damage wins duels; roots and stuns only help now and then (and
        // casting them every turn can lock two bots in a stalemate).
        let (choice, chance) = if hurting.is_empty() {
            (hitting, 30)
        } else {
            (hurting, 90)
        };
        if rng.below(100) < chance {
            if let Some(o) = pick(&choice, rng) {
                return Some(send("cast", o));
            }
        }
    }
    if let Some(steps) = find(ui, "move") {
        let closer: Vec<&ChoiceOption> = steps
            .iter()
            .copied()
            .filter(|o| o.detail.contains("closer to"))
            .collect();
        if rng.below(100) < 85 {
            if let Some(o) = pick(&closer, rng) {
                return Some(send("move", o));
            }
        }
        let fresh: Vec<&ChoiceOption> = steps
            .iter()
            .copied()
            .filter(|o| o.detail.contains("new mana"))
            .collect();
        if rng.below(100) < 40 {
            if let Some(o) = pick(&fresh, rng) {
                return Some(send("move", o));
            }
        }
        // Wander now and then, so a bot with the wrong mana finds more.
        if rng.below(100) < 25 {
            if let Some(o) = pick(&steps, rng) {
                return Some(send("move", o));
            }
        }
    }
    if let Some(ends) = find(ui, "end") {
        let claims: Vec<&ChoiceOption> = ends
            .iter()
            .copied()
            .filter(|o| o.value.starts_with("claim:"))
            .collect();
        if let Some(o) = pick(&claims, rng).or_else(|| ends.first().copied()) {
            return Some(send("end", o));
        }
    }
    None
}

/// Rivals named after "hits " in a target's detail.
fn hit_count(detail: &str) -> usize {
    detail
        .strip_prefix("hits ")
        .map_or(0, |names| names.split(", ").count())
}
