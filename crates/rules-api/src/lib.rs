//! The interface a Hostable game's rules implement.
//!
//! Rules are a pure state machine: the host calls them, stores whatever state
//! they return, and decides what to send where. Rules never touch the
//! network, files, the clock or system randomness: time and the random seed
//! come in as arguments. That keeps games replayable from their seed and
//! action log, and lets the same rules run in the desktop host, the cloud
//! host, and (later) inside a WebAssembly sandbox for third-party packs.

use hostable_protocol::{PhoneInput, Widget};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;

/// Version of this interface. A pack's manifest names the version it was
/// built against.
pub const RULES_API_VERSION: u32 = 1;

/// A seat at the table, numbered from 0 in join order.
pub type SeatId = u32;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Seat {
    pub id: SeatId,
    pub name: String,
}

/// The rules of one game.
pub trait Rules {
    /// Everything about a game in progress. The host saves it after every
    /// action, so it must hold all the game needs, including its own RNG.
    type State: Clone + Serialize + DeserializeOwned;

    /// Fewest and most seats the game supports.
    fn seats(&self) -> (u32, u32);

    /// Set up a new game.
    fn setup(&self, seats: &[Seat], seed: u64) -> Self::State;

    /// A player did something on their phone. The host has already checked
    /// that the widget exists on that player's current screen.
    fn on_action(
        &self,
        state: Self::State,
        seat: SeatId,
        input: &PhoneInput,
    ) -> Outcome<Self::State>;

    /// A wait from an earlier [`Outcome`] ran out. `now_ms` is game time.
    fn on_timeout(&self, state: Self::State, seat: SeatId, now_ms: u64) -> Outcome<Self::State> {
        let _ = (seat, now_ms);
        Outcome::new(state)
    }

    /// What the shared screen shows. Free-form JSON until the table view
    /// format is settled.
    fn table_view(&self, state: &Self::State) -> Value;

    /// What one seat's phone shows.
    fn phone_view(&self, state: &Self::State, seat: SeatId) -> Widget;
}

/// What the rules return after handling an action or timeout.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Outcome<S> {
    pub state: S,
    /// Sounds, animations and toasts for the host to play.
    #[serde(default)]
    pub events: Vec<Event>,
    /// Seats the game is waiting on with a deadline, e.g. a reaction window.
    #[serde(default)]
    pub waiting_on: Vec<Wait>,
    /// Set when the game is over.
    #[serde(default)]
    pub finished: Option<Results>,
}

impl<S> Outcome<S> {
    pub fn new(state: S) -> Self {
        Self {
            state,
            events: Vec::new(),
            waiting_on: Vec::new(),
            finished: None,
        }
    }

    pub fn with_event(mut self, event: Event) -> Self {
        self.events.push(event);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    /// Play a sound from the pack (or a built-in one) on the shared screen.
    Sound { name: String },
    /// A short message on the shared screen.
    Toast { text: String },
    /// Buzz one seat's phone.
    Feedback {
        seat: SeatId,
        kind: hostable_protocol::Feedback,
    },
}

/// The game is waiting on a seat, with a deadline. Used for reaction
/// windows ("respond to Fireball?") and turn timers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Wait {
    pub seat: SeatId,
    /// Used when everyone is online, e.g. 8 seconds.
    pub live_timeout_ms: u64,
    /// Used in async games, e.g. 24 hours.
    pub async_timeout_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Results {
    /// Seats from first place to last. Ties share an inner list.
    pub ranking: Vec<Vec<SeatId>>,
    pub summary: String,
}
