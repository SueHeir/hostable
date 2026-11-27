//! Wire messages shared by the Hostable host, the relay and the phone client.
//!
//! Everything travels as JSON over WebSockets. The relay only reads the outer
//! envelopes ([`HostToRelay`], [`RelayToHost`], [`PhoneToRelay`],
//! [`RelayToPhone`]); what the host and a phone say to each other travels
//! inside them as [`HostToPhone`] and [`PhoneInput`], which the relay
//! forwards without looking.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Version of this protocol. Bumped on any breaking change.
pub const PROTOCOL_VERSION: u32 = 1;

/// A room's join code, e.g. `KQWX`.
pub type RoomCode = String;

/// The relay's id for one player connection in a room. It survives
/// reconnects: a phone that rejoins with its token gets the same id.
pub type PlayerId = u32;

// ---------------------------------------------------------------------------
// Host <-> relay
// ---------------------------------------------------------------------------

/// What the host sends to the relay.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HostToRelay {
    /// Open a new room. Must be the first message on a fresh host socket.
    CreateRoom,
    /// Take back a room after the host dropped. Must be the first message.
    ReclaimRoom { code: RoomCode, host_token: String },
    /// Send a message to one player's phone.
    ToPlayer {
        player: PlayerId,
        message: HostToPhone,
    },
    /// Remove a player from the room. Their token stops working.
    Kick { player: PlayerId },
    /// Stop new players from joining (rejoins still work).
    Lock { locked: bool },
    /// Close the room for good.
    CloseRoom,
}

/// What the relay sends to the host.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RelayToHost {
    RoomCreated {
        code: RoomCode,
        host_token: String,
    },
    RoomReclaimed {
        code: RoomCode,
    },
    PlayerJoined {
        player: PlayerId,
        name: String,
    },
    /// A known player came back after a drop.
    PlayerReturned {
        player: PlayerId,
    },
    /// A player's connection dropped. Their seat should be kept.
    PlayerAway {
        player: PlayerId,
    },
    FromPlayer {
        player: PlayerId,
        input: PhoneInput,
    },
    Error {
        message: String,
    },
}

// ---------------------------------------------------------------------------
// Phone <-> relay
// ---------------------------------------------------------------------------

/// What a phone sends to the relay.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PhoneToRelay {
    /// Join a room as a new player. First message on a fresh phone socket.
    Join { code: RoomCode, name: String },
    /// Come back to a seat with the token from an earlier `Joined`.
    Rejoin {
        code: RoomCode,
        player_token: String,
    },
    /// Something the player did, forwarded to the host.
    Input(PhoneInput),
}

/// What the relay sends to a phone.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RelayToPhone {
    /// Store `player_token`; it is the only way back into this seat.
    Joined {
        player: PlayerId,
        player_token: String,
    },
    Rejoined {
        player: PlayerId,
    },
    /// The host is reconnecting; keep the last UI on screen.
    HostAway,
    FromHost {
        message: HostToPhone,
    },
    Error {
        message: String,
    },
}

// ---------------------------------------------------------------------------
// Host <-> phone (carried through the relay untouched)
// ---------------------------------------------------------------------------

/// What the host tells a phone.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HostToPhone {
    /// Replace the whole screen. `version` only ever goes up, so a phone can
    /// ignore a stale tree that arrives late.
    Ui { version: u64, ui: Widget },
    /// A short buzz or sound on the phone.
    Feedback { kind: Feedback },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Feedback {
    Tap,
    Success,
    Warning,
    YourTurn,
}

/// Something a player did on their phone.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhoneInput {
    /// The `id` of the widget that was used.
    pub widget: String,
    /// What it sent: `null` for a button, the picked value(s) for a choice,
    /// the text for a text input.
    #[serde(default)]
    pub value: Value,
}

impl PhoneInput {
    pub fn press(widget: impl Into<String>) -> Self {
        Self {
            widget: widget.into(),
            value: Value::Null,
        }
    }
}

// ---------------------------------------------------------------------------
// Phone UI description language
// ---------------------------------------------------------------------------

/// A node of the phone UI tree. Games can only describe screens with these
/// widgets; they can never send HTML or scripts to a phone.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Widget {
    Column {
        children: Vec<Widget>,
    },
    Row {
        children: Vec<Widget>,
    },
    Text {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        value: String,
        #[serde(default, skip_serializing_if = "is_default")]
        style: TextStyle,
    },
    Button {
        id: String,
        label: String,
        #[serde(default, skip_serializing_if = "is_default")]
        style: ButtonStyle,
        #[serde(default, skip_serializing_if = "is_false")]
        disabled: bool,
    },
    Choice {
        id: String,
        #[serde(default, skip_serializing_if = "is_default")]
        style: ChoiceStyle,
        options: Vec<ChoiceOption>,
        /// How many options may be picked at once. 1 sends a single value,
        /// more sends a list.
        #[serde(default = "one")]
        max: u32,
    },
    TextInput {
        id: String,
        #[serde(default)]
        placeholder: String,
        max_len: u32,
    },
    /// A countdown, e.g. for a reaction window. Purely visual; the host
    /// enforces the deadline.
    Timer {
        ends_in_ms: u64,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextStyle {
    #[default]
    Body,
    Title,
    Muted,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ButtonStyle {
    #[default]
    Normal,
    Primary,
    /// Fills most of the screen; for buzzers and big single actions.
    Huge,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChoiceStyle {
    #[default]
    List,
    Grid,
    Cards,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChoiceOption {
    pub value: String,
    pub label: String,
    /// Path of an image in the pack's `phone_assets`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
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

    pub fn title(value: impl Into<String>) -> Self {
        Widget::Text {
            id: None,
            value: value.into(),
            style: TextStyle::Title,
        }
    }

    pub fn button(id: impl Into<String>, label: impl Into<String>, style: ButtonStyle) -> Self {
        Widget::Button {
            id: id.into(),
            label: label.into(),
            style,
            disabled: false,
        }
    }

    /// Number of nodes in this tree, used to cap the size of what a game
    /// may send to a phone.
    pub fn node_count(&self) -> usize {
        match self {
            Widget::Column { children } | Widget::Row { children } => {
                1 + children.iter().map(Widget::node_count).sum::<usize>()
            }
            Widget::Choice { options, .. } => 1 + options.len(),
            _ => 1,
        }
    }

    /// Whether a widget with this id exists in the tree, so the host can
    /// drop input for buttons the phone shouldn't have.
    pub fn has_input(&self, wanted: &str) -> bool {
        match self {
            Widget::Column { children } | Widget::Row { children } => {
                children.iter().any(|c| c.has_input(wanted))
            }
            Widget::Button { id, disabled, .. } => id == wanted && !disabled,
            Widget::Choice { id, .. } | Widget::TextInput { id, .. } => id == wanted,
            Widget::Text { .. } | Widget::Timer { .. } => false,
        }
    }
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

    #[test]
    fn widgets_use_the_documented_json_shape() {
        let ui = Widget::Column {
            children: vec![
                Widget::text("Your turn"),
                Widget::button("end_turn", "End turn", ButtonStyle::Primary),
            ],
        };
        assert_eq!(
            serde_json::to_value(&ui).unwrap(),
            json!({
                "type": "column",
                "children": [
                    { "type": "text", "value": "Your turn" },
                    { "type": "button", "id": "end_turn", "label": "End turn", "style": "primary" }
                ]
            })
        );
    }

    #[test]
    fn phone_input_round_trips() {
        let msg = PhoneToRelay::Input(PhoneInput::press("buzz"));
        let text = serde_json::to_string(&msg).unwrap();
        assert_eq!(text, r#"{"type":"input","widget":"buzz","value":null}"#);
        assert_eq!(serde_json::from_str::<PhoneToRelay>(&text).unwrap(), msg);
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
}
