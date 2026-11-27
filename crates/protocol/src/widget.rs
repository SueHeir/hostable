//! The phone UI description language: a JSON tree of known widgets.
//!
//! Games can only describe screens with these widgets; they can never send
//! HTML or scripts to a phone, and text is always shown as text.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::PhoneInput;

/// Longest string a text input or any label may hold.
pub const MAX_TEXT_CHARS: usize = 2000;

/// A node of the phone UI tree.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum Widget {
    // ---- layouts ----
    Column {
        children: Vec<Widget>,
    },
    Row {
        children: Vec<Widget>,
    },
    Grid {
        columns: u32,
        children: Vec<Widget>,
    },
    /// Pages the player flips between. Only the open tab is drawn, but all
    /// of them count as on screen.
    Tabs {
        tabs: Vec<Tab>,
    },
    /// A panel that slides up from the bottom when its button is tapped.
    Sheet {
        label: String,
        children: Vec<Widget>,
    },

    // ---- display ----
    Text {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        id: Option<String>,
        value: String,
        #[serde(default, skip_serializing_if = "is_default")]
        #[ts(as = "Option<TextStyle>", optional)]
        style: TextStyle,
    },
    /// An image from the pack's `phone_assets`, or a built-in icon
    /// (`icon:<name>`).
    Image {
        src: String,
        #[serde(default)]
        alt: String,
        /// Height in CSS pixels; width keeps the aspect ratio.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        height: Option<u32>,
    },
    /// A bar such as health or progress.
    Meter {
        label: String,
        value: i32,
        max: i32,
        #[serde(default, skip_serializing_if = "is_default")]
        #[ts(as = "Option<MeterStyle>", optional)]
        style: MeterStyle,
    },
    /// A countdown, e.g. for a reaction window. Purely visual; the host
    /// enforces the deadline.
    Timer {
        #[ts(type = "number")]
        ends_in_ms: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional, type = "number")]
        total_ms: Option<u64>,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        #[ts(as = "Option<String>", optional)]
        label: String,
    },

    // ---- input ----
    Button {
        id: String,
        label: String,
        #[serde(default, skip_serializing_if = "is_default")]
        #[ts(as = "Option<ButtonStyle>", optional)]
        style: ButtonStyle,
        #[serde(default, skip_serializing_if = "is_false")]
        #[ts(as = "Option<bool>", optional)]
        disabled: bool,
    },
    /// Pick from options. `max` 1 sends a single value; more sends a list
    /// of between `min` and `max` values after a confirm tap.
    Choice {
        id: String,
        #[serde(default, skip_serializing_if = "is_default")]
        #[ts(as = "Option<ChoiceStyle>", optional)]
        style: ChoiceStyle,
        options: Vec<ChoiceOption>,
        #[serde(default = "one")]
        max: u32,
        #[serde(default = "one")]
        min: u32,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        #[ts(as = "Option<String>", optional)]
        label: String,
    },
    /// The player's own cards, fanned. Tapping picks cards like a choice.
    Hand {
        id: String,
        cards: Vec<CardFace>,
        #[serde(default = "one")]
        max: u32,
        #[serde(default = "one")]
        min: u32,
    },
    TextInput {
        id: String,
        #[serde(default)]
        placeholder: String,
        max_len: u32,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        #[ts(as = "Option<String>", optional)]
        submit_label: String,
    },
    /// Four arrows. Sends a [`Direction`].
    Dpad {
        id: String,
    },
    /// A pad that moves a cursor on the shared screen. Sends `{dx, dy}`
    /// (each -1..1, a fraction of the pad) or `{tap: true}`.
    Touchpad {
        id: String,
    },
    /// A canvas the player draws on. Sends a list of strokes, each a list
    /// of `[x, y]` points in 0..1.
    Draw {
        id: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        #[ts(as = "Option<String>", optional)]
        prompt: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Tab {
    pub label: String,
    pub children: Vec<Widget>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum TextStyle {
    #[default]
    Body,
    Title,
    Muted,
    Warning,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ButtonStyle {
    #[default]
    Normal,
    Primary,
    Danger,
    /// Fills most of the screen; for buzzers and big single actions.
    Huge,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ChoiceStyle {
    #[default]
    List,
    Grid,
    Cards,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum MeterStyle {
    #[default]
    Normal,
    Health,
    Mana,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ChoiceOption {
    pub value: String,
    pub label: String,
    /// Second line of smaller text.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    #[ts(as = "Option<String>", optional)]
    pub detail: String,
    /// Path of an image in the pack's `phone_assets`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub image: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    #[ts(as = "Option<bool>", optional)]
    pub disabled: bool,
}

impl ChoiceOption {
    pub fn new(value: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
            detail: String::new(),
            image: None,
            disabled: false,
        }
    }

    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = detail.into();
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

/// One card as a phone draws it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CardFace {
    /// Sent back when the card is picked.
    pub value: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    #[ts(as = "Option<String>", optional)]
    pub text: String,
    /// Shown in the corner, e.g. a cost.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    #[ts(as = "Option<String>", optional)]
    pub corner: String,
    /// A CSS colour for the frame.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    #[ts(as = "Option<String>", optional)]
    pub color: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub image: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    #[ts(as = "Option<bool>", optional)]
    pub disabled: bool,
}

impl Widget {
    pub fn text(value: impl Into<String>) -> Self {
        Widget::Text {
            id: None,
            value: value.into(),
            style: TextStyle::Body,
        }
    }

    pub fn styled(value: impl Into<String>, style: TextStyle) -> Self {
        Widget::Text {
            id: None,
            value: value.into(),
            style,
        }
    }

    pub fn title(value: impl Into<String>) -> Self {
        Self::styled(value, TextStyle::Title)
    }

    pub fn muted(value: impl Into<String>) -> Self {
        Self::styled(value, TextStyle::Muted)
    }

    pub fn button(id: impl Into<String>, label: impl Into<String>, style: ButtonStyle) -> Self {
        Widget::Button {
            id: id.into(),
            label: label.into(),
            style,
            disabled: false,
        }
    }

    pub fn column(children: Vec<Widget>) -> Self {
        Widget::Column { children }
    }

    pub fn row(children: Vec<Widget>) -> Self {
        Widget::Row { children }
    }

    /// A single-pick list.
    pub fn choice(id: impl Into<String>, style: ChoiceStyle, options: Vec<ChoiceOption>) -> Self {
        Widget::Choice {
            id: id.into(),
            style,
            options,
            max: 1,
            min: 1,
            label: String::new(),
        }
    }

    fn children(&self) -> Vec<&Widget> {
        match self {
            Widget::Column { children }
            | Widget::Row { children }
            | Widget::Grid { children, .. }
            | Widget::Sheet { children, .. } => children.iter().collect(),
            Widget::Tabs { tabs } => tabs.iter().flat_map(|t| t.children.iter()).collect(),
            _ => Vec::new(),
        }
    }

    /// Number of nodes in this tree, used to cap the size of what a game
    /// may send to a phone. Options and cards count as nodes.
    pub fn node_count(&self) -> usize {
        let own = match self {
            Widget::Choice { options, .. } => 1 + options.len(),
            Widget::Hand { cards, .. } => 1 + cards.len(),
            Widget::Tabs { tabs } => 1 + tabs.len(),
            _ => 1,
        };
        own + self
            .children()
            .into_iter()
            .map(Widget::node_count)
            .sum::<usize>()
    }

    /// Every widget in the tree that takes input and isn't disabled.
    pub fn inputs(&self) -> Vec<&Widget> {
        let mut out = Vec::new();
        self.collect_inputs(&mut out);
        out
    }

    fn collect_inputs<'a>(&'a self, out: &mut Vec<&'a Widget>) {
        match self {
            Widget::Button { disabled, .. } => {
                if !disabled {
                    out.push(self)
                }
            }
            Widget::Choice { options, .. } => {
                if options.iter().any(|o| !o.disabled) {
                    out.push(self)
                }
            }
            Widget::Hand { cards, .. } => {
                if cards.iter().any(|c| !c.disabled) {
                    out.push(self)
                }
            }
            Widget::TextInput { .. }
            | Widget::Dpad { .. }
            | Widget::Touchpad { .. }
            | Widget::Draw { .. } => out.push(self),
            _ => {}
        }
        for child in self.children() {
            child.collect_inputs(out);
        }
    }

    /// The input widget with this id, if it is on screen and enabled.
    pub fn find_input(&self, wanted: &str) -> Option<&Widget> {
        self.inputs()
            .into_iter()
            .find(|w| w.input_id() == Some(wanted))
    }

    /// Whether a widget with this id exists in the tree, so the host can
    /// drop input for buttons the phone shouldn't have.
    pub fn has_input(&self, wanted: &str) -> bool {
        self.find_input(wanted).is_some()
    }

    pub fn input_id(&self) -> Option<&str> {
        match self {
            Widget::Button { id, .. }
            | Widget::Choice { id, .. }
            | Widget::Hand { id, .. }
            | Widget::TextInput { id, .. }
            | Widget::Dpad { id }
            | Widget::Touchpad { id }
            | Widget::Draw { id, .. } => Some(id),
            Widget::Text { id, .. } => id.as_deref(),
            _ => None,
        }
    }

    /// Whether `input` is something this screen could have sent: the widget
    /// is on screen and enabled, and the value has the right shape (a real
    /// option, a short enough text, a known direction...).
    pub fn accepts(&self, input: &PhoneInput) -> bool {
        let Some(widget) = self.find_input(&input.widget) else {
            return false;
        };
        let v = &input.value;
        match widget {
            Widget::Button { .. } => v.is_null(),
            Widget::Choice {
                options, min, max, ..
            } => {
                let allowed: Vec<&str> = options
                    .iter()
                    .filter(|o| !o.disabled)
                    .map(|o| o.value.as_str())
                    .collect();
                picks_ok(v, &allowed, *min, *max)
            }
            Widget::Hand {
                cards, min, max, ..
            } => {
                let allowed: Vec<&str> = cards
                    .iter()
                    .filter(|c| !c.disabled)
                    .map(|c| c.value.as_str())
                    .collect();
                picks_ok(v, &allowed, *min, *max)
            }
            Widget::TextInput { max_len, .. } => v
                .as_str()
                .is_some_and(|s| s.chars().count() <= (*max_len as usize).min(MAX_TEXT_CHARS)),
            Widget::Dpad { .. } => serde_json::from_value::<Direction>(v.clone()).is_ok(),
            Widget::Touchpad { .. } => touch_ok(v),
            Widget::Draw { .. } => strokes_ok(v),
            _ => false,
        }
    }
}

/// One value for a single pick, or a list of distinct values in `min..=max`.
fn picks_ok(v: &Value, allowed: &[&str], min: u32, max: u32) -> bool {
    if max <= 1 {
        return v.as_str().is_some_and(|s| allowed.contains(&s));
    }
    let Some(list) = v.as_array() else {
        return false;
    };
    let n = list.len() as u32;
    if n < min || n > max {
        return false;
    }
    let mut seen = Vec::new();
    for item in list {
        match item.as_str() {
            Some(s) if allowed.contains(&s) && !seen.contains(&s) => seen.push(s),
            _ => return false,
        }
    }
    true
}

fn touch_ok(v: &Value) -> bool {
    let Some(obj) = v.as_object() else {
        return false;
    };
    if obj.get("tap") == Some(&Value::Bool(true)) && obj.len() == 1 {
        return true;
    }
    let unit = |k: &str| {
        obj.get(k)
            .and_then(Value::as_f64)
            .is_some_and(|x| (-1.0..=1.0).contains(&x))
    };
    obj.len() == 2 && unit("dx") && unit("dy")
}

/// At most 64 strokes of at most 512 points, each coordinate in 0..1.
fn strokes_ok(v: &Value) -> bool {
    let Some(strokes) = v.as_array() else {
        return false;
    };
    strokes.len() <= 64
        && strokes.iter().all(|s| {
            s.as_array().is_some_and(|pts| {
                pts.len() <= 512
                    && pts.iter().all(|p| {
                        p.as_array().is_some_and(|xy| {
                            xy.len() == 2
                                && xy
                                    .iter()
                                    .all(|c| c.as_f64().is_some_and(|c| (0.0..=1.0).contains(&c)))
                        })
                    })
            })
        })
}

fn is_default<T: Default + PartialEq>(v: &T) -> bool {
    *v == T::default()
}

fn is_false(b: &bool) -> bool {
    !*b
}

fn one() -> u32 {
    1
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn opts(values: &[&str]) -> Vec<ChoiceOption> {
        values.iter().map(|v| ChoiceOption::new(*v, *v)).collect()
    }

    #[test]
    fn disabled_buttons_take_no_input() {
        let ui = Widget::Row {
            children: vec![Widget::Button {
                id: "undo".into(),
                label: "Undo".into(),
                style: ButtonStyle::Normal,
                disabled: true,
            }],
        };
        assert!(!ui.has_input("undo"));
        assert!(!ui.has_input("missing"));
        assert_eq!(ui.node_count(), 2);
    }

    #[test]
    fn inputs_inside_tabs_and_sheets_count_as_on_screen() {
        let ui = Widget::Tabs {
            tabs: vec![
                Tab {
                    label: "Hand".into(),
                    children: vec![Widget::Sheet {
                        label: "More".into(),
                        children: vec![Widget::button("deep", "Deep", ButtonStyle::Normal)],
                    }],
                },
                Tab {
                    label: "Board".into(),
                    children: vec![Widget::Dpad { id: "pad".into() }],
                },
            ],
        };
        assert!(ui.has_input("deep"));
        assert!(ui.has_input("pad"));
        assert_eq!(ui.node_count(), 6);
    }

    #[test]
    fn choice_values_must_be_real_enabled_options() {
        let mut options = opts(&["a", "b", "c"]);
        options[2].disabled = true;
        let ui = Widget::Choice {
            id: "pick".into(),
            style: ChoiceStyle::List,
            options,
            max: 1,
            min: 1,
            label: String::new(),
        };
        assert!(ui.accepts(&PhoneInput::with("pick", "a")));
        assert!(!ui.accepts(&PhoneInput::with("pick", "c")));
        assert!(!ui.accepts(&PhoneInput::with("pick", "zzz")));
        assert!(!ui.accepts(&PhoneInput::with("pick", json!(["a"]))));
    }

    #[test]
    fn multi_picks_respect_min_max_and_uniqueness() {
        let ui = Widget::Hand {
            id: "hand".into(),
            cards: ["x", "y", "z"]
                .iter()
                .map(|v| CardFace {
                    value: v.to_string(),
                    title: v.to_string(),
                    text: String::new(),
                    corner: String::new(),
                    color: String::new(),
                    image: None,
                    disabled: false,
                })
                .collect(),
            max: 2,
            min: 1,
        };
        assert!(ui.accepts(&PhoneInput::with("hand", json!(["x", "z"]))));
        assert!(!ui.accepts(&PhoneInput::with("hand", json!([]))));
        assert!(!ui.accepts(&PhoneInput::with("hand", json!(["x", "x"]))));
        assert!(!ui.accepts(&PhoneInput::with("hand", json!(["x", "y", "z"]))));
    }

    #[test]
    fn other_inputs_are_shape_checked() {
        let ui = Widget::column(vec![
            Widget::TextInput {
                id: "name".into(),
                placeholder: String::new(),
                max_len: 5,
                submit_label: String::new(),
            },
            Widget::Dpad { id: "pad".into() },
            Widget::Touchpad { id: "touch".into() },
            Widget::Draw {
                id: "art".into(),
                prompt: String::new(),
            },
            Widget::button("ok", "OK", ButtonStyle::Primary),
        ]);
        assert!(ui.accepts(&PhoneInput::with("name", "hey")));
        assert!(!ui.accepts(&PhoneInput::with("name", "too long")));
        assert!(ui.accepts(&PhoneInput::with("pad", "left")));
        assert!(!ui.accepts(&PhoneInput::with("pad", "sideways")));
        assert!(ui.accepts(&PhoneInput::with("touch", json!({"dx": 0.5, "dy": -1.0}))));
        assert!(ui.accepts(&PhoneInput::with("touch", json!({"tap": true}))));
        assert!(!ui.accepts(&PhoneInput::with("touch", json!({"dx": 4, "dy": 0}))));
        assert!(ui.accepts(&PhoneInput::with("art", json!([[[0.1, 0.2], [0.3, 0.4]]]))));
        assert!(!ui.accepts(&PhoneInput::with("art", json!([[[2, 0]]]))));
        assert!(ui.accepts(&PhoneInput::press("ok")));
        assert!(!ui.accepts(&PhoneInput::with("ok", "x")));
    }

    #[test]
    fn every_widget_kind_round_trips_through_json() {
        let ui = Widget::Grid {
            columns: 2,
            children: vec![
                Widget::Image {
                    src: "assets/a.png".into(),
                    alt: "a".into(),
                    height: Some(40),
                },
                Widget::Meter {
                    label: "HP".into(),
                    value: 3,
                    max: 5,
                    style: MeterStyle::Health,
                },
                Widget::Timer {
                    ends_in_ms: 8000,
                    total_ms: Some(8000),
                    label: "Respond?".into(),
                },
                Widget::choice("c", ChoiceStyle::Cards, opts(&["1"])),
            ],
        };
        let text = serde_json::to_string(&ui).unwrap();
        assert_eq!(serde_json::from_str::<Widget>(&text).unwrap(), ui);
    }
}
