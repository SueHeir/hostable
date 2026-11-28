//! A bot that plays Riposte from its phone screen alone, the way a person
//! would: it reads the cards in its hand, its Spark line and the buttons on
//! screen, and only ever sends what the screen offers.
//!
//! It builds a deck (or takes a starter), plays what it can afford, keeps
//! Spark back for a Feint, strikes with everything, blocks now and then, and
//! answers the opponent with Feints whenever it can, so chains of responses
//! happen often.

use hostable_sdk::prelude::*;

/// Every widget in the tree, depth first.
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

struct HandCard {
    value: String,
    cost: u32,
    feint: bool,
    enabled: bool,
}

fn hand_cards(all: &[&Widget]) -> Vec<HandCard> {
    all.iter()
        .find_map(|w| match w {
            Widget::Hand { id, cards, .. } if id == "play" => Some(
                cards
                    .iter()
                    .map(|c| HandCard {
                        value: c.value.clone(),
                        cost: c.corner.parse().unwrap_or(0),
                        feint: c.text.starts_with("Feint"),
                        enabled: !c.disabled,
                    })
                    .collect(),
            ),
            _ => None,
        })
        .unwrap_or_default()
}

fn spark(all: &[&Widget]) -> u32 {
    all.iter()
        .find_map(|w| match w {
            Widget::Text {
                id: Some(id),
                value,
                ..
            } if id == "spark" => value
                .strip_prefix("Spark ")
                .and_then(|r| r.split('/').next())
                .and_then(|n| n.trim().parse().ok()),
            _ => None,
        })
        .unwrap_or(0)
}

fn pick<'a, T>(items: &'a [T], rng: &mut Rng) -> Option<&'a T> {
    if items.is_empty() {
        None
    } else {
        items.get(rng.below(items.len() as u32) as usize)
    }
}

fn enabled_options(w: &Widget) -> Vec<(String, String)> {
    match w {
        Widget::Choice { options, .. } => options
            .iter()
            .filter(|o| !o.disabled)
            .map(|o| (o.value.clone(), o.label.clone()))
            .collect(),
        _ => Vec::new(),
    }
}

/// Any input the screen accepts, as a last resort.
fn fallback(ui: &Widget, rng: &mut Rng) -> Option<PhoneInput> {
    for w in ui.inputs() {
        let id = w.input_id()?.to_string();
        let input = match w {
            Widget::Button { .. } => PhoneInput::press(id),
            Widget::Choice { min, max, .. } => {
                let opts = enabled_options(w);
                if *max <= 1 {
                    let (v, _) = pick(&opts, rng)?;
                    PhoneInput::with(id, v.clone())
                } else {
                    let n = (*min).max(1) as usize;
                    let vals: Vec<Value> = opts
                        .iter()
                        .take(n)
                        .map(|(v, _)| Value::String(v.clone()))
                        .collect();
                    PhoneInput::with(id, Value::Array(vals))
                }
            }
            Widget::Hand { cards, .. } => {
                let c = cards.iter().find(|c| !c.disabled)?;
                PhoneInput::with(id, c.value.clone())
            }
            _ => continue,
        };
        if ui.accepts(&input) {
            return Some(input);
        }
    }
    None
}

fn checked(ui: &Widget, input: PhoneInput, rng: &mut Rng) -> Option<PhoneInput> {
    if ui.accepts(&input) {
        Some(input)
    } else {
        fallback(ui, rng)
    }
}

/// The bot's move for this screen, or `None` if it has no controls.
pub fn choose(ui: &Widget, rng: &mut Rng) -> Option<PhoneInput> {
    let inputs = ui.inputs();
    if inputs.is_empty() {
        return None;
    }
    let mut all = Vec::new();
    walk(ui, &mut all);
    let find = |id: &str| inputs.iter().copied().find(|w| w.input_id() == Some(id));

    // Deck building: a starter half the time, else 20 random cards.
    if let Some(deck) = find("deck") {
        let starters: Vec<&str> = inputs
            .iter()
            .filter_map(|w| w.input_id())
            .filter(|id| id.starts_with("starter"))
            .collect();
        if !starters.is_empty() && rng.below(2) == 0 {
            let id = pick(&starters, rng)?;
            return checked(ui, PhoneInput::press(*id), rng);
        }
        let mut vals: Vec<String> = enabled_options(deck).into_iter().map(|(v, _)| v).collect();
        rng.shuffle(&mut vals);
        vals.truncate(20);
        let input = PhoneInput::with(
            "deck",
            Value::Array(vals.into_iter().map(Value::String).collect()),
        );
        return checked(ui, input, rng);
    }

    // Picking a target: hurt the opponent, help yourself.
    if let Some(t) = find("target") {
        let label = match t {
            Widget::Choice { label, .. } => label.clone(),
            _ => String::new(),
        };
        let harmful = ["Deal", "Destroy", "Return", "Spend"]
            .iter()
            .any(|w| label.starts_with(w));
        let opts = enabled_options(t);
        let good: Vec<&(String, String)> = opts
            .iter()
            .filter(|(v, l)| {
                if harmful {
                    v == "foe" || l.starts_with("Enemy")
                } else {
                    v == "me" || l.starts_with("Your")
                }
            })
            .collect();
        let choice = if good.is_empty() || rng.below(10) == 0 {
            pick(&opts, rng).cloned()
        } else {
            pick(&good, rng).map(|x| (*x).clone())
        };
        if let Some((v, _)) = choice {
            return checked(ui, PhoneInput::with("target", v), rng);
        }
        return checked(ui, PhoneInput::press("cancel"), rng);
    }

    // Blocking: add a block half the time, then finish.
    if find("blocks_done").is_some() {
        if let Some(b) = find("block") {
            if rng.below(2) == 0 {
                if let Some((v, _)) = pick(&enabled_options(b), rng) {
                    return checked(ui, PhoneInput::with("block", v.clone()), rng);
                }
            }
        }
        return checked(ui, PhoneInput::press("blocks_done"), rng);
    }

    let hand = hand_cards(&all);
    let playable: Vec<&HandCard> = hand.iter().filter(|c| c.enabled).collect();

    // A reaction window: answer with a Feint whenever possible.
    if find("pass").is_some() {
        if !playable.is_empty() && rng.below(100) < 85 {
            let c = pick(&playable, rng)?;
            return checked(ui, PhoneInput::with("play", c.value.clone()), rng);
        }
        if let Some(a) = find("ability") {
            if rng.below(2) == 0 {
                if let Some((v, _)) = pick(&enabled_options(a), rng) {
                    return checked(ui, PhoneInput::with("ability", v.clone()), rng);
                }
            }
        }
        return checked(ui, PhoneInput::press("pass"), rng);
    }

    // Own turn.
    if find("end_turn").is_some() {
        let sp = spark(&all);
        let reserve = hand
            .iter()
            .filter(|c| c.feint)
            .map(|c| c.cost)
            .min()
            .filter(|_| rng.below(100) < 75)
            .unwrap_or(0);
        let main: Vec<&&HandCard> = playable
            .iter()
            .filter(|c| !c.feint && c.cost + reserve <= sp)
            .collect();
        if let Some(c) = pick(&main, rng) {
            return checked(ui, PhoneInput::with("play", c.value.clone()), rng);
        }
        if let Some(Widget::Choice { options, max, .. }) = find("strike") {
            let vals: Vec<Value> = options
                .iter()
                .filter(|o| !o.disabled)
                .map(|o| Value::String(o.value.clone()))
                .collect();
            let value = if *max <= 1 {
                vals.into_iter().next().unwrap_or(Value::Null)
            } else {
                Value::Array(vals)
            };
            return checked(ui, PhoneInput::with("strike", value), rng);
        }
        if let Some(a) = find("ability") {
            if rng.below(2) == 0 {
                if let Some((v, _)) = pick(&enabled_options(a), rng) {
                    return checked(ui, PhoneInput::with("ability", v.clone()), rng);
                }
            }
        }
        let feints: Vec<&&HandCard> = playable.iter().filter(|c| c.feint).collect();
        if rng.below(100) < 10 {
            if let Some(c) = pick(&feints, rng) {
                return checked(ui, PhoneInput::with("play", c.value.clone()), rng);
            }
        }
        return checked(ui, PhoneInput::press("end_turn"), rng);
    }

    fallback(ui, rng)
}
