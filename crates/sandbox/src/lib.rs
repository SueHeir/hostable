//! Runs untrusted game rules in the `wasmi` interpreter.
//!
//! A pack's `rules.wasm` gets:
//!
//! - no imports except `env.log`, so it can't touch files, the network, the
//!   clock or system randomness;
//! - a fuel budget per call (about 50 ms of work), so an infinite loop ends
//!   with [`RulesError::OutOfFuel`] instead of freezing the host;
//! - a 64 MB memory cap, so a memory bomb ends with
//!   [`RulesError::OutOfMemory`];
//! - schema checks on everything it returns, so garbage ends with
//!   [`RulesError::Malformed`].
//!
//! After any failure the instance is thrown away and a fresh one is made for
//! the next call, so a crash can't leave half-updated memory behind. Rules
//! are pure, so nothing is lost: the state lives with the host.
//!
//! [`WasmRules`] implements [`Rules`] with JSON state, so a [`Room`] can run a
//! sandboxed game exactly like a native one.
//!
//! [`Room`]: https://docs.rs/hostable-host-core

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hostable_protocol::{PhoneInput, Widget};
use hostable_rules_api::abi::{Info, Request, Response, ABI_VERSION};
use hostable_rules_api::{Outcome, Rules, RulesError, RulesResult, Seat, SeatId};
use serde_json::Value;
use wasmi::{
    Caller, Config, Engine, Extern, ExternType, Instance, Linker, Memory, Module, Store, TrapCode,
    TypedFunc, ValType,
};

/// The QuickJS interpreter built as a rules module (see `runtimes/quickjs`).
/// It loads a JavaScript game with `hostable_load` and then speaks the same
/// ABI as a Rust one.
pub const QUICKJS_RULES_HOST: &[u8] = include_bytes!("../runtimes/quickjs-rules-host.wasm");

/// Largest response a call may return.
pub const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
/// Largest request the host will send (mostly the state).
pub const MAX_REQUEST_BYTES: usize = 8 * 1024 * 1024;
/// Most log lines kept per call, and their length.
const MAX_LOG_LINES: usize = 64;
const MAX_LOG_CHARS: usize = 500;

/// How much a rules module may use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Fuel (roughly, wasm instructions) per call.
    pub fuel_per_call: u64,
    /// Fuel for loading a JavaScript game (parsing the script).
    pub fuel_to_load: u64,
    /// Linear memory cap in bytes.
    pub memory_bytes: usize,
}

impl Limits {
    /// About 50 ms of work per call on a laptop (wasmi runs on the order of
    /// 10^9 simple instructions a second; see the `fuel_is_about_fifty_ms`
    /// test) and 64 MB of memory.
    pub const DEFAULT: Limits = Limits {
        fuel_per_call: 60_000_000,
        fuel_to_load: 400_000_000,
        memory_bytes: 64 * 1024 * 1024,
    };
}

impl Default for Limits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// What kind of module this is.
#[derive(Debug, Clone)]
enum Kind {
    Wasm,
    JavaScript(Arc<Vec<u8>>),
}

struct HostState {
    limits: Tracker,
    logs: Vec<String>,
}

/// Wraps wasmi's limits to remember that memory growth was refused, so a
/// trap that follows can be reported as out of memory.
struct Tracker {
    memory_bytes: usize,
    denied: bool,
}

impl wasmi::ResourceLimiter for Tracker {
    fn memory_growing(
        &mut self,
        _current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> Result<bool, wasmi::errors::LimiterError> {
        let ok = desired <= self.memory_bytes && maximum.is_none_or(|m| desired <= m);
        if !ok {
            self.denied = true;
        }
        Ok(ok)
    }

    fn table_growing(
        &mut self,
        _current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> Result<bool, wasmi::errors::LimiterError> {
        Ok(desired <= 100_000 && maximum.is_none_or(|m| desired <= m))
    }

    fn instances(&self) -> usize {
        1
    }

    fn tables(&self) -> usize {
        4
    }

    fn memories(&self) -> usize {
        1
    }
}

struct Live {
    store: Store<HostState>,
    memory: Memory,
    alloc: TypedFunc<i32, i32>,
    free: TypedFunc<(i32, i32), ()>,
    call: TypedFunc<(i32, i32), i64>,
}

/// A sandboxed game.
pub struct WasmRules {
    engine: Engine,
    module: Module,
    kind: Kind,
    limits: Limits,
    info: Info,
    live: Mutex<Option<Live>>,
    stats: Mutex<Stats>,
}

/// What the sandbox has used, for quotas and the pack checker.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Stats {
    pub calls: u64,
    pub fuel: u64,
    pub busy: Duration,
    pub last_fuel: u64,
    pub logs: Vec<String>,
}

impl WasmRules {
    /// Load a `rules.wasm` compiled from Rust (or anything else that speaks
    /// the ABI).
    pub fn new(wasm: &[u8], limits: Limits) -> RulesResult<Self> {
        Self::build(wasm, Kind::Wasm, limits)
    }

    /// Load a JavaScript game into the QuickJS rules host.
    pub fn javascript(source: &[u8], limits: Limits) -> RulesResult<Self> {
        Self::build(
            QUICKJS_RULES_HOST,
            Kind::JavaScript(Arc::new(source.to_vec())),
            limits,
        )
    }

    fn build(wasm: &[u8], kind: Kind, limits: Limits) -> RulesResult<Self> {
        let mut config = Config::default();
        config.consume_fuel(true);
        let engine = Engine::new(&config);
        let module = Module::new(&engine, wasm)
            .map_err(|e| RulesError::Malformed(format!("not a valid wasm module: {e}")))?;
        check_imports(&module)?;
        let mut rules = Self {
            engine,
            module,
            kind,
            limits,
            info: Info {
                min_seats: 0,
                max_seats: 0,
                api: 0,
            },
            live: Mutex::new(None),
            stats: Mutex::new(Stats::default()),
        };
        let info: Info = serde_json::from_value(rules.request(&Request::Info)?)
            .map_err(|e| RulesError::Malformed(format!("info: {e}")))?;
        if info.min_seats == 0 || info.min_seats > info.max_seats || info.max_seats > 64 {
            return Err(RulesError::Malformed(format!(
                "seats {}..{} make no sense",
                info.min_seats, info.max_seats
            )));
        }
        if info.api != hostable_rules_api::RULES_API_VERSION {
            return Err(RulesError::Malformed(format!(
                "built for rules API {}, this host speaks {}",
                info.api,
                hostable_rules_api::RULES_API_VERSION
            )));
        }
        rules.info = info;
        Ok(rules)
    }

    pub fn limits(&self) -> Limits {
        self.limits
    }

    /// Usage so far. Logs are drained.
    pub fn take_stats(&self) -> Stats {
        let mut stats = self.stats.lock().unwrap();
        let out = stats.clone();
        stats.logs.clear();
        out
    }

    fn instantiate(&self) -> RulesResult<Live> {
        let mut store = Store::new(
            &self.engine,
            HostState {
                limits: Tracker {
                    memory_bytes: self.limits.memory_bytes,
                    denied: false,
                },
                logs: Vec::new(),
            },
        );
        store.limiter(|s| &mut s.limits);
        store
            .set_fuel(self.limits.fuel_to_load)
            .map_err(|e| RulesError::Crashed(e.to_string()))?;
        let mut linker = Linker::<HostState>::new(&self.engine);
        linker
            .func_wrap(
                "env",
                "log",
                |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| {
                    let Some(Extern::Memory(mem)) = caller.get_export("memory") else {
                        return;
                    };
                    let len = (len as u32 as usize).min(MAX_LOG_CHARS * 4);
                    let mut buf = vec![0u8; len];
                    if mem.read(&caller, ptr as u32 as usize, &mut buf).is_err() {
                        return;
                    }
                    let logs = &mut caller.data_mut().logs;
                    if logs.len() < MAX_LOG_LINES {
                        let text: String = String::from_utf8_lossy(&buf)
                            .chars()
                            .take(MAX_LOG_CHARS)
                            .collect();
                        logs.push(text);
                    }
                },
            )
            .map_err(|e| RulesError::Crashed(e.to_string()))?;
        let instance = linker
            .instantiate_and_start(&mut store, &self.module)
            .map_err(|e| classify(e, &store))?;
        let memory = instance
            .get_memory(&store, "memory")
            .ok_or_else(|| RulesError::Malformed("no exported memory".into()))?;
        let version: TypedFunc<(), i32> = typed(&instance, &store, "hostable_abi_version")?;
        let v = version
            .call(&mut store, ())
            .map_err(|e| classify(e, &store))?;
        if v != ABI_VERSION {
            return Err(RulesError::Malformed(format!(
                "module speaks ABI {v}, this host speaks {ABI_VERSION}"
            )));
        }
        let mut live = Live {
            alloc: typed(&instance, &store, "hostable_alloc")?,
            free: typed(&instance, &store, "hostable_free")?,
            call: typed(&instance, &store, "hostable_call")?,
            memory,
            store,
        };
        if let Kind::JavaScript(source) = &self.kind {
            let load: TypedFunc<(i32, i32), i64> = typed(&instance, &live.store, "hostable_load")?;
            let response = exchange(&mut live, &load, source)?;
            match parse_response(&response)? {
                Value::Null => {}
                other => {
                    return Err(RulesError::Malformed(format!(
                        "loading the script returned {other}"
                    )))
                }
            }
        }
        Ok(live)
    }

    /// Send one request and get the `ok` value back.
    fn request(&self, request: &Request) -> RulesResult<Value> {
        let bytes = serde_json::to_vec(request).map_err(|e| RulesError::Crashed(e.to_string()))?;
        if bytes.len() > MAX_REQUEST_BYTES {
            return Err(RulesError::Malformed(format!(
                "the game state grew to {} bytes",
                bytes.len()
            )));
        }
        let mut guard = self.live.lock().unwrap();
        if guard.is_none() {
            *guard = Some(self.instantiate()?);
        }
        let live = guard.as_mut().expect("just made");
        live.store
            .set_fuel(self.limits.fuel_per_call)
            .map_err(|e| RulesError::Crashed(e.to_string()))?;
        live.store.data_mut().limits.denied = false;
        let started = Instant::now();
        let call = live.call;
        let result = exchange(live, &call, &bytes);
        let left = live.store.get_fuel().unwrap_or(0);
        let used = self.limits.fuel_per_call.saturating_sub(left);
        let logs = std::mem::take(&mut live.store.data_mut().logs);
        {
            let mut stats = self.stats.lock().unwrap();
            stats.calls += 1;
            stats.fuel += used;
            stats.last_fuel = used;
            stats.busy += started.elapsed();
            stats.logs.extend(logs);
        }
        match result.and_then(|r| parse_response(&r)) {
            Ok(v) => Ok(v),
            Err(e) => {
                // Throw the instance away; the next call gets a fresh one.
                *guard = None;
                Err(e)
            }
        }
    }
}

fn typed<P: wasmi::WasmParams, R: wasmi::WasmResults>(
    instance: &Instance,
    store: &Store<HostState>,
    name: &str,
) -> RulesResult<TypedFunc<P, R>> {
    instance
        .get_typed_func::<P, R>(store, name)
        .map_err(|e| RulesError::Malformed(format!("export {name}: {e}")))
}

/// Write `input` into the module, call `func` with it, read the answer.
fn exchange(
    live: &mut Live,
    func: &TypedFunc<(i32, i32), i64>,
    input: &[u8],
) -> RulesResult<Vec<u8>> {
    let len = i32::try_from(input.len()).map_err(|_| RulesError::Malformed("too big".into()))?;
    let ptr = live
        .alloc
        .call(&mut live.store, len)
        .map_err(|e| classify(e, &live.store))?;
    live.memory
        .write(&mut live.store, ptr as u32 as usize, input)
        .map_err(|_| RulesError::Malformed("hostable_alloc returned a bad pointer".into()))?;
    let packed = func
        .call(&mut live.store, (ptr, len))
        .map_err(|e| classify(e, &live.store))?;
    live.free
        .call(&mut live.store, (ptr, len))
        .map_err(|e| classify(e, &live.store))?;
    let out_ptr = (packed as u64 >> 32) as usize;
    let out_len = (packed as u64 & 0xffff_ffff) as usize;
    if out_len > MAX_RESPONSE_BYTES {
        return Err(RulesError::Malformed(format!(
            "response of {out_len} bytes is too big"
        )));
    }
    let mut out = vec![0u8; out_len];
    live.memory
        .read(&live.store, out_ptr, &mut out)
        .map_err(|_| RulesError::Malformed("response points outside memory".into()))?;
    Ok(out)
}

fn parse_response(bytes: &[u8]) -> RulesResult<Value> {
    match serde_json::from_slice::<Response>(bytes) {
        Ok(Response::Ok(v)) => Ok(v),
        Ok(Response::Err(e)) => Err(match e {
            // A module can't claim the host's own failure kinds.
            RulesError::OutOfFuel | RulesError::OutOfMemory => {
                RulesError::Game("the rules reported a resource error".into())
            }
            other => other,
        }),
        Err(e) => {
            let preview: String = String::from_utf8_lossy(bytes).chars().take(80).collect();
            Err(RulesError::Malformed(format!(
                "response isn't JSON ({e}): {preview}"
            )))
        }
    }
}

fn classify(e: wasmi::Error, store: &Store<HostState>) -> RulesError {
    if store.data().limits.denied {
        return RulesError::OutOfMemory;
    }
    match e.as_trap_code() {
        Some(TrapCode::OutOfFuel) => RulesError::OutOfFuel,
        Some(TrapCode::StackOverflow) => RulesError::Crashed("stack overflow".into()),
        Some(code) => RulesError::Crashed(format!("{code:?}")),
        None => {
            let text = e.to_string();
            if text.to_ascii_lowercase().contains("fuel") {
                RulesError::OutOfFuel
            } else if text.to_ascii_lowercase().contains("memory")
                && text.to_ascii_lowercase().contains("limit")
            {
                RulesError::OutOfMemory
            } else {
                RulesError::Crashed(text)
            }
        }
    }
}

/// The only import a rules module may have is `env.log(i32, i32)`.
fn check_imports(module: &Module) -> RulesResult<()> {
    for import in module.imports() {
        let ok = import.module() == "env"
            && import.name() == "log"
            && match import.ty() {
                ExternType::Func(f) => {
                    f.params() == [ValType::I32, ValType::I32] && f.results().is_empty()
                }
                _ => false,
            };
        if !ok {
            return Err(RulesError::Malformed(format!(
                "rules may not import {}.{}",
                import.module(),
                import.name()
            )));
        }
    }
    Ok(())
}

fn outcome(v: Value) -> RulesResult<Outcome<Value>> {
    serde_json::from_value(v).map_err(|e| RulesError::Malformed(format!("outcome: {e}")))
}

impl Rules for WasmRules {
    type State = Value;

    fn seats(&self) -> (u32, u32) {
        (self.info.min_seats, self.info.max_seats)
    }

    fn setup(&self, seats: &[Seat], seed: u64) -> RulesResult<Outcome<Value>> {
        outcome(self.request(&Request::Setup {
            seats: seats.to_vec(),
            seed,
        })?)
    }

    fn on_action(
        &self,
        state: Value,
        seat: SeatId,
        input: &PhoneInput,
    ) -> RulesResult<Outcome<Value>> {
        outcome(self.request(&Request::OnAction {
            state,
            seat,
            input: input.clone(),
        })?)
    }

    fn on_timeout(&self, state: Value, seat: SeatId, now_ms: u64) -> RulesResult<Outcome<Value>> {
        outcome(self.request(&Request::OnTimeout {
            state,
            seat,
            now_ms,
        })?)
    }

    fn table_view(&self, state: &Value) -> RulesResult<Value> {
        self.request(&Request::TableView {
            state: state.clone(),
        })
    }

    fn phone_view(&self, state: &Value, seat: SeatId) -> RulesResult<Widget> {
        let v = self.request(&Request::PhoneView {
            state: state.clone(),
            seat,
        })?;
        serde_json::from_value(v).map_err(|e| RulesError::Malformed(format!("phone view: {e}")))
    }

    fn can_respond(&self, state: &Value, seat: SeatId) -> RulesResult<bool> {
        let v = self.request(&Request::CanRespond {
            state: state.clone(),
            seat,
        })?;
        v.as_bool()
            .ok_or_else(|| RulesError::Malformed(format!("can_respond returned {v}")))
    }
}

#[cfg(test)]
mod tests;
