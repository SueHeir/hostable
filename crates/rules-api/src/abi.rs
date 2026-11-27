//! The JSON ABI between the host's sandbox and a `rules.wasm` module.
//!
//! A rules module exports:
//!
//! - `memory`
//! - `hostable_abi_version() -> i32`, which returns [`ABI_VERSION`]
//! - `hostable_alloc(len: i32) -> i32`, a buffer the host writes a request into
//! - `hostable_free(ptr: i32, len: i32)`, which the host calls on that buffer
//!   once the call returns
//! - `hostable_call(ptr: i32, len: i32) -> i64`: handle one JSON [`Request`]
//!   and return a JSON [`Response`] as `(ptr << 32) | len`. The response buffer
//!   belongs to the module and stays valid until the next call.
//!
//! The only import it may have is `env.log(ptr: i32, len: i32)`.
//!
//! A JavaScript rules module (the QuickJS runtime) additionally exports
//! `hostable_load(ptr, len) -> i64`, which the host calls once with the game's
//! script before anything else.

use hostable_protocol::{PhoneInput, Widget};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Outcome, Rules, RulesError, Seat, SeatId, RULES_API_VERSION};

pub const ABI_VERSION: i32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "call", rename_all = "snake_case")]
pub enum Request {
    Info,
    Setup {
        seats: Vec<Seat>,
        seed: u64,
    },
    OnAction {
        state: Value,
        seat: SeatId,
        input: PhoneInput,
    },
    OnTimeout {
        state: Value,
        seat: SeatId,
        now_ms: u64,
    },
    TableView {
        state: Value,
    },
    PhoneView {
        state: Value,
        seat: SeatId,
    },
    CanRespond {
        state: Value,
        seat: SeatId,
    },
}

/// `{"ok": ...}` or `{"err": {"kind": ..., "message": ...}}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Response {
    Ok(Value),
    Err(RulesError),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Info {
    pub min_seats: u32,
    pub max_seats: u32,
    pub api: u32,
}

/// Handle one request with native rules. The SDK's `export!` macro calls
/// this inside the wasm module; tests call it directly.
pub fn dispatch<R: Rules>(rules: &R, request: &[u8]) -> Vec<u8> {
    let response = match serde_json::from_slice::<Request>(request) {
        Ok(req) => match handle(rules, req) {
            Ok(v) => Response::Ok(v),
            Err(e) => Response::Err(e),
        },
        Err(e) => Response::Err(RulesError::Malformed(format!("bad request: {e}"))),
    };
    serde_json::to_vec(&response).unwrap_or_else(|e| {
        format!(r#"{{"err":{{"kind":"crashed","message":"{e}"}}}}"#).into_bytes()
    })
}

fn state<S: serde::de::DeserializeOwned>(v: Value) -> Result<S, RulesError> {
    serde_json::from_value(v).map_err(|e| RulesError::Malformed(format!("bad state: {e}")))
}

fn value<T: Serialize>(v: &T) -> Result<Value, RulesError> {
    serde_json::to_value(v).map_err(|e| RulesError::Crashed(e.to_string()))
}

fn outcome<S: Serialize>(o: Outcome<S>) -> Result<Value, RulesError> {
    value(&Outcome {
        state: value(&o.state)?,
        events: o.events,
        waiting_on: o.waiting_on,
        finished: o.finished,
    })
}

fn handle<R: Rules>(rules: &R, req: Request) -> Result<Value, RulesError> {
    match req {
        Request::Info => {
            let (min_seats, max_seats) = rules.seats();
            value(&Info {
                min_seats,
                max_seats,
                api: RULES_API_VERSION,
            })
        }
        Request::Setup { seats, seed } => outcome(rules.setup(&seats, seed)?),
        Request::OnAction {
            state: s,
            seat,
            input,
        } => outcome(rules.on_action(state(s)?, seat, &input)?),
        Request::OnTimeout {
            state: s,
            seat,
            now_ms,
        } => outcome(rules.on_timeout(state(s)?, seat, now_ms)?),
        Request::TableView { state: s } => rules.table_view(&state(s)?),
        Request::PhoneView { state: s, seat } => {
            let w: Widget = rules.phone_view(&state(s)?, seat)?;
            value(&w)
        }
        Request::CanRespond { state: s, seat } => value(&rules.can_respond(&state(s)?, seat)?),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct Echo;
    impl Rules for Echo {
        type State = Vec<String>;
        fn seats(&self) -> (u32, u32) {
            (2, 4)
        }
        fn setup(&self, seats: &[Seat], _: u64) -> crate::RulesResult<Outcome<Self::State>> {
            Ok(Outcome::new(seats.iter().map(|s| s.name.clone()).collect()))
        }
        fn on_action(
            &self,
            mut s: Self::State,
            _: SeatId,
            input: &PhoneInput,
        ) -> crate::RulesResult<Outcome<Self::State>> {
            if input.widget == "boom" {
                return Err(RulesError::Game("no".into()));
            }
            s.push(input.widget.clone());
            Ok(Outcome::new(s))
        }
        fn table_view(&self, s: &Self::State) -> crate::RulesResult<Value> {
            Ok(json!(s.len()))
        }
        fn phone_view(&self, _: &Self::State, _: SeatId) -> crate::RulesResult<Widget> {
            Ok(Widget::text("hi"))
        }
    }

    fn call(req: Value) -> Value {
        serde_json::from_slice(&dispatch(&Echo, req.to_string().as_bytes())).unwrap()
    }

    #[test]
    fn requests_and_responses_use_the_documented_shape() {
        assert_eq!(
            call(json!({"call": "info"})),
            json!({"ok": {"min_seats": 2, "max_seats": 4, "api": 1}})
        );
        let out = call(json!({"call": "setup", "seats": [{"id": 0, "name": "Ana"}], "seed": 1}));
        assert_eq!(out["ok"]["state"], json!(["Ana"]));
        let out = call(json!({"call": "on_action", "state": ["Ana"], "seat": 0,
            "input": {"widget": "go", "value": null}}));
        assert_eq!(out["ok"]["state"], json!(["Ana", "go"]));
        assert_eq!(
            call(json!({"call": "phone_view", "state": [], "seat": 0})),
            json!({"ok": {"type": "text", "value": "hi"}})
        );
        assert_eq!(
            call(json!({"call": "can_respond", "state": [], "seat": 0})),
            json!({"ok": true})
        );
    }

    #[test]
    fn errors_come_back_as_err() {
        assert_eq!(
            call(json!({"call": "on_action", "state": [], "seat": 0,
                "input": {"widget": "boom"}})),
            json!({"err": {"kind": "game", "message": "no"}})
        );
        assert_eq!(call(json!({"call": "nope"}))["err"]["kind"], "malformed");
        assert_eq!(
            call(json!({"call": "table_view", "state": 5}))["err"]["kind"],
            "malformed"
        );
    }
}
