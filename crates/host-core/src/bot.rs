//! A bot that plays any game through its phone screens, the way a person
//! would: it can only send what the screen offers. The pack checker uses it
//! to play seeded games; games use it in tests; the end-to-end tests drive
//! simulated phones with it.

use hostable_protocol::{Direction, PhoneInput, Widget};
use hostable_rules_api::Rng;
use serde_json::{json, Value};

/// A random input that `ui` would accept, or `None` if the screen has no
/// controls. `prefer` lists widget ids to pick first when present (for
/// example a game's "end turn"), each with the given chance in percent.
pub fn random_input(ui: &Widget, rng: &mut Rng, prefer: &[(&str, u32)]) -> Option<PhoneInput> {
    let inputs = ui.inputs();
    if inputs.is_empty() {
        return None;
    }
    for (id, chance) in prefer {
        if let Some(w) = inputs.iter().find(|w| w.input_id() == Some(*id)) {
            if rng.below(100) < *chance {
                return input_for(w, rng);
            }
        }
    }
    let pick = inputs[rng.below(inputs.len() as u32) as usize];
    input_for(pick, rng)
}

/// A valid input for one widget.
pub fn input_for(widget: &Widget, rng: &mut Rng) -> Option<PhoneInput> {
    let id = widget.input_id()?.to_string();
    let value = match widget {
        Widget::Button { .. } => Value::Null,
        Widget::Choice {
            options, min, max, ..
        } => picks(
            options
                .iter()
                .filter(|o| !o.disabled)
                .map(|o| o.value.clone())
                .collect(),
            *min,
            *max,
            rng,
        )?,
        Widget::Hand {
            cards, min, max, ..
        } => picks(
            cards
                .iter()
                .filter(|c| !c.disabled)
                .map(|c| c.value.clone())
                .collect(),
            *min,
            *max,
            rng,
        )?,
        Widget::TextInput { max_len, .. } => {
            let n = rng.range(1, (*max_len).clamp(1, 8));
            Value::String(
                (0..n)
                    .map(|_| (b'a' + rng.below(26) as u8) as char)
                    .collect(),
            )
        }
        Widget::Dpad { .. } => {
            let d = [
                Direction::Up,
                Direction::Down,
                Direction::Left,
                Direction::Right,
            ][rng.below(4) as usize];
            serde_json::to_value(d).expect("direction serializes")
        }
        Widget::Touchpad { .. } => {
            if rng.below(4) == 0 {
                json!({ "tap": true })
            } else {
                let f = |r: &mut Rng| (r.below(201) as f64 - 100.0) / 100.0;
                json!({ "dx": f(rng), "dy": f(rng) })
            }
        }
        Widget::Draw { .. } => json!([[[0.1, 0.1], [0.5, 0.5], [0.9, 0.2]]]),
        _ => return None,
    };
    let input = PhoneInput { widget: id, value };
    widget_accepts(widget, &input).then_some(input)
}

fn widget_accepts(widget: &Widget, input: &PhoneInput) -> bool {
    widget.accepts(input)
}

fn picks(mut values: Vec<String>, min: u32, max: u32, rng: &mut Rng) -> Option<Value> {
    if values.is_empty() {
        return None;
    }
    if max <= 1 {
        let v = values.swap_remove(rng.below(values.len() as u32) as usize);
        return Some(Value::String(v));
    }
    let lo = min.max(1).min(values.len() as u32);
    let hi = max.min(values.len() as u32);
    if lo > hi {
        return None;
    }
    let n = rng.range(lo, hi);
    rng.shuffle(&mut values);
    values.truncate(n as usize);
    Some(Value::Array(
        values.into_iter().map(Value::String).collect(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use hostable_protocol::{ButtonStyle, ChoiceOption, ChoiceStyle};

    #[test]
    fn bot_inputs_are_always_accepted() {
        let ui = Widget::column(vec![
            Widget::button("a", "A", ButtonStyle::Normal),
            Widget::Choice {
                id: "c".into(),
                style: ChoiceStyle::Grid,
                options: (0..5)
                    .map(|i| ChoiceOption::new(i.to_string(), "x"))
                    .collect(),
                max: 3,
                min: 2,
                label: String::new(),
            },
            Widget::TextInput {
                id: "t".into(),
                placeholder: String::new(),
                max_len: 4,
                submit_label: String::new(),
            },
            Widget::Dpad { id: "d".into() },
            Widget::Touchpad { id: "p".into() },
            Widget::Draw {
                id: "w".into(),
                prompt: String::new(),
            },
        ]);
        let mut rng = Rng::new(1);
        for _ in 0..500 {
            let input = random_input(&ui, &mut rng, &[]).unwrap();
            assert!(ui.accepts(&input), "{input:?}");
        }
        assert!(random_input(&Widget::text("x"), &mut rng, &[]).is_none());
    }

    #[test]
    fn preferred_widgets_are_picked_first() {
        let ui = Widget::column(vec![
            Widget::button("a", "A", ButtonStyle::Normal),
            Widget::button("end", "End", ButtonStyle::Normal),
        ]);
        let mut rng = Rng::new(2);
        let ends = (0..100)
            .filter(|_| random_input(&ui, &mut rng, &[("end", 100)]).unwrap().widget == "end")
            .count();
        assert_eq!(ends, 100);
    }
}
