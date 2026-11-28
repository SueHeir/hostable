//! The interface a Hostable game's rules implement.
//!
//! Rules are a pure state machine: the host calls them, stores whatever state
//! they return, and decides what to send where. Rules never touch the
//! network, files, the clock or system randomness: time and the random seed
//! come in as arguments. That keeps games replayable from their seed and
//! action log, and lets the same rules run natively or inside the WebAssembly
//! sandbox for third-party packs.
//!
//! Every call can fail with a [`RulesError`]. Native games rarely do; a
//! sandboxed game fails when it runs out of fuel or memory, traps, or returns
//! something malformed, and the host pauses the game instead of freezing.

use std::fmt;

use hostable_protocol::{PhoneInput, Widget};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;

pub mod abi;
pub mod rng;
pub use rng::Rng;

/// Version of this interface. A pack's manifest names the version it was
/// built against.
pub const RULES_API_VERSION: u32 = 1;

/// A seat at the table, numbered from 0 in join order.
pub type SeatId = u32;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Seat {
    pub id: SeatId,
    pub name: String,
    /// A CSS colour the player picked, or empty.
    #[serde(default)]
    pub color: String,
}

impl Seat {
    pub fn new(id: SeatId, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            color: String::new(),
        }
    }
}

/// Why a call into the rules failed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "message", rename_all = "snake_case")]
pub enum RulesError {
    /// The rules used up their fuel (CPU budget) for this call.
    OutOfFuel,
    /// The rules tried to use more memory than they are allowed.
    OutOfMemory,
    /// The rules crashed (a wasm trap or a panic).
    Crashed(String),
    /// The rules returned something that isn't a valid outcome.
    Malformed(String),
    /// The rules rejected the call themselves.
    Game(String),
}

impl fmt::Display for RulesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RulesError::OutOfFuel => write!(f, "the game's rules ran too long"),
            RulesError::OutOfMemory => write!(f, "the game's rules used too much memory"),
            RulesError::Crashed(m) => write!(f, "the game's rules crashed: {m}"),
            RulesError::Malformed(m) => write!(f, "the game's rules sent something invalid: {m}"),
            RulesError::Game(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for RulesError {}

pub type RulesResult<T> = Result<T, RulesError>;

/// The rules of one game.
pub trait Rules {
    /// Everything about a game in progress. The host saves it after every
    /// action, so it must hold all the game needs, including its own RNG.
    type State: Clone + Serialize + DeserializeOwned;

    /// Fewest and most seats the game supports.
    fn seats(&self) -> (u32, u32);

    /// Set up a new game. The outcome may already wait on seats (a turn
    /// timer, say).
    fn setup(&self, seats: &[Seat], seed: u64) -> RulesResult<Outcome<Self::State>>;

    /// A player did something on their phone. The host has already checked
    /// that the widget is on that player's current screen and the value has
    /// the right shape.
    fn on_action(
        &self,
        state: Self::State,
        seat: SeatId,
        input: &PhoneInput,
    ) -> RulesResult<Outcome<Self::State>>;

    /// A [`Wait`] with [`OnTimeout::Rules`] ran out. `now_ms` is game time.
    fn on_timeout(
        &self,
        state: Self::State,
        seat: SeatId,
        now_ms: u64,
    ) -> RulesResult<Outcome<Self::State>> {
        let _ = (seat, now_ms);
        Ok(Outcome::new(state))
    }

    /// What the shared screen shows: a serialized
    /// [`hostable_protocol::TableView`]. Must hold no private information.
    fn table_view(&self, state: &Self::State) -> RulesResult<Value>;

    /// What one seat's phone shows.
    fn phone_view(&self, state: &Self::State, seat: SeatId) -> RulesResult<Widget>;

    /// Whether `seat` has anything it could do in the current reaction
    /// window. When false, the host may pass for them at once.
    fn can_respond(&self, state: &Self::State, seat: SeatId) -> RulesResult<bool> {
        let _ = (state, seat);
        Ok(true)
    }
}

impl<T: Rules + ?Sized> Rules for Box<T> {
    type State = T::State;

    fn seats(&self) -> (u32, u32) {
        (**self).seats()
    }
    fn setup(&self, seats: &[Seat], seed: u64) -> RulesResult<Outcome<Self::State>> {
        (**self).setup(seats, seed)
    }
    fn on_action(
        &self,
        state: Self::State,
        seat: SeatId,
        input: &PhoneInput,
    ) -> RulesResult<Outcome<Self::State>> {
        (**self).on_action(state, seat, input)
    }
    fn on_timeout(
        &self,
        state: Self::State,
        seat: SeatId,
        now_ms: u64,
    ) -> RulesResult<Outcome<Self::State>> {
        (**self).on_timeout(state, seat, now_ms)
    }
    fn table_view(&self, state: &Self::State) -> RulesResult<Value> {
        (**self).table_view(state)
    }
    fn phone_view(&self, state: &Self::State, seat: SeatId) -> RulesResult<Widget> {
        (**self).phone_view(state, seat)
    }
    fn can_respond(&self, state: &Self::State, seat: SeatId) -> RulesResult<bool> {
        (**self).can_respond(state, seat)
    }
}

/// Rules whose state is plain JSON, so the host can hold any game behind one
/// type (`Box<dyn DynRules>`), native or sandboxed.
pub type DynRules = dyn Rules<State = Value> + Send;

/// Runs typed native rules with JSON state, the same way the sandbox does.
#[derive(Debug, Clone, Default)]
pub struct Json<R>(pub R);

fn to_json<S: Serialize>(state: &S) -> RulesResult<Value> {
    serde_json::to_value(state).map_err(|e| RulesError::Malformed(e.to_string()))
}

fn from_json<S: DeserializeOwned>(state: Value) -> RulesResult<S> {
    serde_json::from_value(state).map_err(|e| RulesError::Malformed(format!("bad state: {e}")))
}

fn outcome_to_json<S: Serialize>(o: Outcome<S>) -> RulesResult<Outcome<Value>> {
    Ok(Outcome {
        state: to_json(&o.state)?,
        events: o.events,
        waiting_on: o.waiting_on,
        finished: o.finished,
    })
}

impl<R: Rules> Rules for Json<R> {
    type State = Value;

    fn seats(&self) -> (u32, u32) {
        self.0.seats()
    }
    fn setup(&self, seats: &[Seat], seed: u64) -> RulesResult<Outcome<Value>> {
        outcome_to_json(self.0.setup(seats, seed)?)
    }
    fn on_action(
        &self,
        state: Value,
        seat: SeatId,
        input: &PhoneInput,
    ) -> RulesResult<Outcome<Value>> {
        outcome_to_json(self.0.on_action(from_json(state)?, seat, input)?)
    }
    fn on_timeout(&self, state: Value, seat: SeatId, now_ms: u64) -> RulesResult<Outcome<Value>> {
        outcome_to_json(self.0.on_timeout(from_json(state)?, seat, now_ms)?)
    }
    fn table_view(&self, state: &Value) -> RulesResult<Value> {
        self.0.table_view(&from_json(state.clone())?)
    }
    fn phone_view(&self, state: &Value, seat: SeatId) -> RulesResult<Widget> {
        self.0.phone_view(&from_json(state.clone())?, seat)
    }
    fn can_respond(&self, state: &Value, seat: SeatId) -> RulesResult<bool> {
        self.0.can_respond(&from_json(state.clone())?, seat)
    }
}

/// What the rules return after handling a call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Outcome<S> {
    pub state: S,
    /// Sounds, animations and toasts for the host to play.
    #[serde(default)]
    pub events: Vec<Event>,
    /// Seats the game is waiting on with a deadline, e.g. a reaction window
    /// or a turn timer. Replaces the previous list; a wait that is still
    /// listed keeps its original deadline.
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

    pub fn waiting(mut self, wait: Wait) -> Self {
        self.waiting_on.push(wait);
        self
    }

    pub fn finish(mut self, results: Results) -> Self {
        self.finished = Some(results);
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
    /// Something a native renderer animates, e.g. a wizard moving.
    Animate { what: Value },
}

/// What happens when a [`Wait`] runs out.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OnTimeout {
    /// Call [`Rules::on_timeout`] for the seat.
    #[default]
    Rules,
    /// Act as if the seat sent this input (usually a "pass" button).
    Input { input: PhoneInput },
}

/// The game is waiting on a seat, with a deadline. Used for reaction
/// windows ("respond to Fireball?") and turn timers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Wait {
    pub seat: SeatId,
    /// Shown above the seat's countdown, e.g. "Respond to Fireball?".
    #[serde(default)]
    pub prompt: String,
    /// Used when everyone is online, e.g. 8 seconds.
    pub live_timeout_ms: u64,
    /// Used in async games, e.g. 24 hours.
    pub async_timeout_ms: u64,
    #[serde(default)]
    pub on_timeout: OnTimeout,
    /// A reaction window (as opposed to a turn). Seats that can't respond
    /// to one may be passed automatically.
    #[serde(default)]
    pub reaction: bool,
}

impl Wait {
    /// A turn timer: on timeout the rules decide.
    pub fn turn(seat: SeatId, live_timeout_ms: u64, async_timeout_ms: u64) -> Self {
        Self {
            seat,
            prompt: String::new(),
            live_timeout_ms,
            async_timeout_ms,
            on_timeout: OnTimeout::Rules,
            reaction: false,
        }
    }

    /// A reaction window that passes with `pass` (an input) on timeout.
    pub fn reaction(
        seat: SeatId,
        prompt: impl Into<String>,
        live_timeout_ms: u64,
        async_timeout_ms: u64,
        pass: PhoneInput,
    ) -> Self {
        Self {
            seat,
            prompt: prompt.into(),
            live_timeout_ms,
            async_timeout_ms,
            on_timeout: OnTimeout::Input { input: pass },
            reaction: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Results {
    /// Seats from first place to last. Ties share an inner list.
    pub ranking: Vec<Vec<SeatId>>,
    pub summary: String,
}

/// Rank seats by score, highest first, ties grouped.
pub fn rank_by_score(scores: &[i64]) -> Vec<Vec<SeatId>> {
    let mut distinct: Vec<i64> = scores.to_vec();
    distinct.sort_unstable_by(|a, b| b.cmp(a));
    distinct.dedup();
    distinct
        .into_iter()
        .map(|s| {
            (0..scores.len() as SeatId)
                .filter(|&i| scores[i as usize] == s)
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[derive(Default)]
    struct Counter;

    impl Rules for Counter {
        type State = u32;
        fn seats(&self) -> (u32, u32) {
            (1, 2)
        }
        fn setup(&self, _: &[Seat], seed: u64) -> RulesResult<Outcome<u32>> {
            Ok(Outcome::new(seed as u32))
        }
        fn on_action(&self, s: u32, _: SeatId, _: &PhoneInput) -> RulesResult<Outcome<u32>> {
            Ok(Outcome::new(s + 1))
        }
        fn table_view(&self, s: &u32) -> RulesResult<Value> {
            Ok(json!(s))
        }
        fn phone_view(&self, s: &u32, _: SeatId) -> RulesResult<Widget> {
            Ok(Widget::text(s.to_string()))
        }
    }

    #[test]
    fn json_adapter_round_trips_state() {
        let rules: Box<DynRules> = Box::new(Json(Counter));
        let out = rules.setup(&[Seat::new(0, "Ana")], 5).unwrap();
        assert_eq!(out.state, json!(5));
        let out = rules
            .on_action(out.state, 0, &PhoneInput::press("x"))
            .unwrap();
        assert_eq!(out.state, json!(6));
        assert_eq!(
            rules.on_action(json!("nope"), 0, &PhoneInput::press("x")),
            Err(RulesError::Malformed(
                "bad state: invalid type: string \"nope\", expected u32".into()
            ))
        );
        assert!(rules.can_respond(&json!(1), 0).unwrap());
    }

    #[test]
    fn waits_default_to_calling_the_rules() {
        let w: Wait =
            serde_json::from_value(json!({"seat": 1, "live_timeout_ms": 5, "async_timeout_ms": 9}))
                .unwrap();
        assert_eq!(w, Wait::turn(1, 5, 9));
    }

    #[test]
    fn ranking_groups_ties() {
        assert_eq!(rank_by_score(&[3, 5, 3]), vec![vec![1], vec![0, 2]]);
    }
}
