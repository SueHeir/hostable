//! Wire messages shared by the Hostable host, the relay and the phone client.
//!
//! Everything travels as JSON over WebSockets. The relay only reads the outer
//! envelopes ([`HostToRelay`], [`RelayToHost`], [`PhoneToRelay`],
//! [`RelayToPhone`]); what the host and a phone say to each other travels
//! inside them as [`HostToPhone`] and [`PhoneInput`], which the relay
//! forwards without looking.
//!
//! TypeScript types for the web client are generated from these with
//! `ts-rs` (`cargo test -p hostable-protocol export_bindings`), so the two
//! sides can't drift apart.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

mod table;
mod widget;

pub use table::{PlayerPanel, Scene, Shape, TableView};
pub use widget::{
    ButtonStyle, CardFace, ChoiceOption, ChoiceStyle, Direction, MeterStyle, Tab, TextStyle,
    Widget, MAX_TEXT_CHARS,
};

/// Version of this protocol. Bumped on any breaking change.
pub const PROTOCOL_VERSION: u32 = 2;

/// A room's join code, e.g. `KQWX`.
pub type RoomCode = String;

/// The relay's id for one player connection in a room. It survives
/// reconnects: a phone that rejoins with its token gets the same id.
pub type PlayerId = u32;

/// Inputs whose widget id starts with this are meant for the host itself
/// (settings, subscriptions), never for the game's rules.
pub const HOST_INPUT_PREFIX: char = '$';
/// `value: true|false`: also send me the table view (laptops, remote play).
pub const INPUT_WANT_TABLE: &str = "$table";
/// `value: true|false`: "only ask me when I can respond".
pub const INPUT_AUTO_PASS: &str = "$auto_pass";
/// `value`: a Web Push subscription (`{endpoint, keys: {p256dh, auth}}`).
pub const INPUT_PUSH: &str = "$push";

// ---------------------------------------------------------------------------
// Host <-> relay
// ---------------------------------------------------------------------------

/// What the host sends to the relay.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum HostToRelay {
    /// Open a new room. Must be the first message on a fresh host socket.
    CreateRoom,
    /// Take back a room after the host dropped, or hand a room to another
    /// host (desktop to cloud) that was given the code and host token.
    /// Must be the first message.
    ReclaimRoom { code: RoomCode, host_token: String },
    /// Send a message to one player's phone.
    ToPlayer {
        player: PlayerId,
        message: HostToPhone,
    },
    /// Make a seat for a player who will arrive later with a link (remote
    /// and async games). The relay answers with [`RelayToHost::PlayerAdded`].
    AddPlayer { name: String },
    /// Remove a player from the room. Their token stops working.
    Kick { player: PlayerId },
    /// Stop new players from joining (rejoins still work).
    Lock { locked: bool },
    /// Close the room for good.
    CloseRoom,
}

/// What the relay sends to the host.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
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
        #[serde(default)]
        color: String,
    },
    /// Answer to [`HostToRelay::AddPlayer`]: the token goes into a share link.
    PlayerAdded {
        player: PlayerId,
        name: String,
        player_token: String,
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum PhoneToRelay {
    /// Join a room as a new player. First message on a fresh phone socket.
    Join {
        code: RoomCode,
        name: String,
        /// A CSS colour like `#e4572e`; anything else is dropped.
        #[serde(default)]
        color: String,
    },
    /// Come back to a seat with the token from an earlier `Joined` (or from
    /// a share link).
    Rejoin {
        code: RoomCode,
        player_token: String,
    },
    /// Something the player did, forwarded to the host.
    Input(PhoneInput),
}

/// What the relay sends to a phone.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
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
    /// The host is back.
    HostBack,
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum HostToPhone {
    /// Replace the whole screen. `version` only ever goes up, so a phone can
    /// ignore a stale tree that arrives late.
    Ui {
        #[ts(type = "number")]
        version: u64,
        ui: Widget,
    },
    /// The shared table, for phones that asked for it with `$table`.
    Table {
        #[ts(type = "number")]
        version: u64,
        table: TableView,
    },
    /// A short buzz or sound on the phone.
    Feedback { kind: Feedback },
    /// Who this phone is in the game.
    Seat {
        /// `null` for a spectator.
        seat: Option<u32>,
        name: String,
        game: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Feedback {
    Tap,
    Success,
    Warning,
    YourTurn,
}

/// Something a player did on their phone.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PhoneInput {
    /// The `id` of the widget that was used.
    pub widget: String,
    /// What it sent: `null` for a button, the picked value (or a list of
    /// values) for a choice or hand, the text for a text input, a
    /// [`Direction`] for a dpad, `{dx, dy}` or `{tap: true}` for a touchpad,
    /// a list of strokes for a drawing.
    #[serde(default)]
    #[ts(type = "unknown")]
    pub value: Value,
}

impl PhoneInput {
    pub fn press(widget: impl Into<String>) -> Self {
        Self {
            widget: widget.into(),
            value: Value::Null,
        }
    }

    pub fn with(widget: impl Into<String>, value: impl Into<Value>) -> Self {
        Self {
            widget: widget.into(),
            value: value.into(),
        }
    }

    /// Meant for the host (settings), not for the rules.
    pub fn is_host_input(&self) -> bool {
        self.widget.starts_with(HOST_INPUT_PREFIX)
    }
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
    fn join_without_colour_still_parses() {
        let msg: PhoneToRelay =
            serde_json::from_str(r#"{"type":"join","code":"BCDF","name":"Ana"}"#).unwrap();
        assert_eq!(
            msg,
            PhoneToRelay::Join {
                code: "BCDF".into(),
                name: "Ana".into(),
                color: String::new()
            }
        );
    }

    #[test]
    fn host_inputs_are_recognised() {
        assert!(PhoneInput::with(INPUT_WANT_TABLE, true).is_host_input());
        assert!(!PhoneInput::press("buzz").is_host_input());
    }
}
