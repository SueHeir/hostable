//! CPU quotas for sandboxed games on a shared server.
//!
//! Each call already has a fuel budget (about 50 ms). A quota also caps how
//! much time one game may spend in its rules over a rolling window, so one
//! busy or hostile game can't starve the others. Memory is capped per game
//! by the sandbox's [`hostable_sandbox::Limits`].

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use hostable_protocol::{PhoneInput, Widget};
use hostable_rules_api::{DynRules, Outcome, Rules, RulesError, RulesResult, Seat, SeatId};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Quota {
    /// Most time in rules calls within `window`.
    pub busy: Duration,
    pub window: Duration,
}

impl Default for Quota {
    fn default() -> Self {
        // Ten seconds of rules per minute is far more than any turn-based
        // game needs.
        Self {
            busy: Duration::from_secs(10),
            window: Duration::from_secs(60),
        }
    }
}

/// Rules that count their own running time and refuse calls once a game
/// is over its quota.
pub struct Metered {
    inner: Box<DynRules>,
    quota: Quota,
    calls: Mutex<VecDeque<(Instant, Duration)>>,
}

impl Metered {
    pub fn new(inner: Box<DynRules>, quota: Quota) -> Self {
        Self {
            inner,
            quota,
            calls: Mutex::new(VecDeque::new()),
        }
    }

    /// Time spent in the current window.
    pub fn busy(&self) -> Duration {
        let mut calls = self.calls.lock().unwrap();
        prune(&mut calls, self.quota.window);
        calls.iter().map(|(_, d)| *d).sum()
    }

    fn call<T>(&self, f: impl FnOnce(&DynRules) -> RulesResult<T>) -> RulesResult<T> {
        if self.busy() >= self.quota.busy {
            return Err(RulesError::Crashed(format!(
                "this game used its CPU quota ({:?} per {:?})",
                self.quota.busy, self.quota.window
            )));
        }
        let started = Instant::now();
        let out = f(&*self.inner);
        self.calls
            .lock()
            .unwrap()
            .push_back((started, started.elapsed()));
        out
    }
}

fn prune(calls: &mut VecDeque<(Instant, Duration)>, window: Duration) {
    while calls.front().is_some_and(|(at, _)| at.elapsed() > window) {
        calls.pop_front();
    }
}

impl Rules for Metered {
    type State = Value;

    fn seats(&self) -> (u32, u32) {
        self.inner.seats()
    }
    fn setup(&self, seats: &[Seat], seed: u64) -> RulesResult<Outcome<Value>> {
        self.call(|r| r.setup(seats, seed))
    }
    fn on_action(&self, s: Value, seat: SeatId, input: &PhoneInput) -> RulesResult<Outcome<Value>> {
        self.call(|r| r.on_action(s, seat, input))
    }
    fn on_timeout(&self, s: Value, seat: SeatId, now: u64) -> RulesResult<Outcome<Value>> {
        self.call(|r| r.on_timeout(s, seat, now))
    }
    fn table_view(&self, s: &Value) -> RulesResult<Value> {
        self.call(|r| r.table_view(s))
    }
    fn phone_view(&self, s: &Value, seat: SeatId) -> RulesResult<Widget> {
        self.call(|r| r.phone_view(s, seat))
    }
    fn can_respond(&self, s: &Value, seat: SeatId) -> RulesResult<bool> {
        self.call(|r| r.can_respond(s, seat))
    }
}
