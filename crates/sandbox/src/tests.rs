use super::*;
use serde_json::json;

/// A tiny rules module in WAT. It answers `info`, `setup` and `phone_view`
/// properly and does `on_action` (the 9th byte of the request is the first
/// letter of the call name) the way `misbehave` says.
fn fixture(misbehave: &str) -> Vec<u8> {
    let info = r#"{"ok":{"min_seats":1,"max_seats":4,"api":1}}"#;
    let setup = r#"{"ok":{"state":{"n":0}}}"#;
    let view = r#"{"ok":{"type":"text","value":"hi"}}"#;
    let action = r#"{"ok":{"state":{"n":1},"events":[{"type":"toast","text":"one"}]}}"#;
    let malformed = r#"{"ok":{"nope":true}}"#;
    let garbage = "this is not json";
    let wat = format!(
        r#"(module
  (import "env" "log" (func $log (param i32 i32)))
  (memory (export "memory") 1)
  (data (i32.const 2048) "{info}")
  (data (i32.const 4096) "{setup}")
  (data (i32.const 6144) "{view}")
  (data (i32.const 8192) "{action}")
  (data (i32.const 10240) "{malformed}")
  (data (i32.const 12288) "{garbage}")
  (func (export "hostable_abi_version") (result i32) (i32.const 1))
  (func (export "hostable_alloc") (param i32) (result i32) (i32.const 16384))
  (func (export "hostable_free") (param i32 i32))
  (func $ret (param $ptr i32) (param $len i32) (result i64)
    (i64.or (i64.shl (i64.extend_i32_u (local.get $ptr)) (i64.const 32))
            (i64.extend_i32_u (local.get $len))))
  (func (export "hostable_call") (param $p i32) (param $n i32) (result i64)
    (local $c i32)
    (local.set $c (i32.load8_u (i32.add (local.get $p) (i32.const 9))))
    (if (i32.eq (local.get $c) (i32.const 105)) (then (return (call $ret (i32.const 2048) (i32.const {info_len})))))
    (if (i32.eq (local.get $c) (i32.const 115)) (then (return (call $ret (i32.const 4096) (i32.const {setup_len})))))
    (if (i32.eq (local.get $c) (i32.const 112)) (then (return (call $ret (i32.const 6144) (i32.const {view_len})))))
    (call $log (i32.const 2048) (i32.const 5))
    {body})
)"#,
        info = info.replace('"', "\\\""),
        setup = setup.replace('"', "\\\""),
        view = view.replace('"', "\\\""),
        action = action.replace('"', "\\\""),
        malformed = malformed.replace('"', "\\\""),
        garbage = garbage,
        info_len = info.len(),
        setup_len = setup.len(),
        view_len = view.len(),
        body = match misbehave {
            "fine" => format!("(call $ret (i32.const 8192) (i32.const {}))", action.len()),
            "loop" => "(loop $l (br $l)) (unreachable)".to_string(),
            "bomb" => "(loop $l (br_if $l (i32.ne (memory.grow (i32.const 16)) (i32.const -1)))) (unreachable)".to_string(),
            "malformed" => format!("(call $ret (i32.const 10240) (i32.const {}))", malformed.len()),
            "garbage" => format!("(call $ret (i32.const 12288) (i32.const {}))", garbage.len()),
            "trap" => "(unreachable)".to_string(),
            "wild" => "(i64.const -1)".to_string(),
            "recurse" => "(call $deep) (unreachable)".to_string(),
            other => panic!("unknown fixture {other}"),
        },
    )
    .replace("(func (export \"hostable_call\")", "(func $deep (call $deep))\n  (func (export \"hostable_call\")");
    wat::parse_str(&wat).unwrap_or_else(|e| panic!("{e}\n{wat}"))
}

fn seats() -> Vec<Seat> {
    vec![Seat::new(0, "Ana"), Seat::new(1, "Bo")]
}

fn act(rules: &WasmRules) -> RulesResult<Outcome<Value>> {
    let state = rules.setup(&seats(), 1)?.state;
    rules.on_action(state, 0, &PhoneInput::press("go"))
}

#[test]
fn a_well_behaved_module_runs_and_logs() {
    let rules = WasmRules::new(&fixture("fine"), Limits::DEFAULT).unwrap();
    assert_eq!(rules.seats(), (1, 4));
    let out = act(&rules).unwrap();
    assert_eq!(out.state, json!({"n": 1}));
    assert_eq!(out.events.len(), 1);
    assert_eq!(rules.phone_view(&out.state, 0).unwrap(), Widget::text("hi"));
    let stats = rules.take_stats();
    assert_eq!(stats.logs, vec!["{\"ok\"".to_string()]);
    assert!(stats.fuel > 0);
}

#[test]
fn an_infinite_loop_runs_out_of_fuel_and_the_host_carries_on() {
    let rules = WasmRules::new(&fixture("loop"), Limits::DEFAULT).unwrap();
    let started = Instant::now();
    assert_eq!(act(&rules), Err(RulesError::OutOfFuel));
    assert!(started.elapsed() < Duration::from_secs(10));
    // The broken instance was replaced; other calls still work.
    assert_eq!(rules.phone_view(&json!({}), 0).unwrap(), Widget::text("hi"));
}

#[test]
fn a_memory_bomb_hits_the_cap() {
    let rules = WasmRules::new(&fixture("bomb"), Limits::DEFAULT).unwrap();
    assert_eq!(act(&rules), Err(RulesError::OutOfMemory));
    assert!(rules.phone_view(&json!({}), 0).is_ok());
}

#[test]
fn a_module_asking_for_too_much_memory_up_front_is_refused() {
    let wat = r#"(module (memory (export "memory") 2000)
        (func (export "hostable_abi_version") (result i32) (i32.const 1)))"#;
    let err = WasmRules::new(&wat::parse_str(wat).unwrap(), Limits::DEFAULT).err();
    assert_eq!(err, Some(RulesError::OutOfMemory));
}

#[test]
fn malformed_outcomes_are_rejected() {
    let rules = WasmRules::new(&fixture("malformed"), Limits::DEFAULT).unwrap();
    assert!(matches!(act(&rules), Err(RulesError::Malformed(m)) if m.starts_with("outcome")));
    let rules = WasmRules::new(&fixture("garbage"), Limits::DEFAULT).unwrap();
    assert!(matches!(act(&rules), Err(RulesError::Malformed(m)) if m.contains("isn't JSON")));
    let rules = WasmRules::new(&fixture("wild"), Limits::DEFAULT).unwrap();
    assert!(matches!(act(&rules), Err(RulesError::Malformed(_))));
}

#[test]
fn traps_and_stack_overflows_are_crashes() {
    let rules = WasmRules::new(&fixture("trap"), Limits::DEFAULT).unwrap();
    assert!(matches!(act(&rules), Err(RulesError::Crashed(_))));
    let rules = WasmRules::new(&fixture("recurse"), Limits::DEFAULT).unwrap();
    assert!(matches!(act(&rules), Err(RulesError::Crashed(_))));
}

#[test]
fn modules_may_only_import_log() {
    let wat = r#"(module
        (import "wasi_snapshot_preview1" "fd_write" (func (param i32 i32 i32 i32) (result i32)))
        (memory (export "memory") 1))"#;
    let err = WasmRules::new(&wat::parse_str(wat).unwrap(), Limits::DEFAULT).err();
    assert!(matches!(err, Some(RulesError::Malformed(m)) if m.contains("fd_write")));
    let wat = r#"(module (import "env" "log" (func (param i64))) (memory (export "memory") 1))"#;
    assert!(WasmRules::new(&wat::parse_str(wat).unwrap(), Limits::DEFAULT).is_err());
    assert!(WasmRules::new(b"not wasm", Limits::DEFAULT).is_err());
}

#[test]
fn the_quickjs_host_imports_nothing_but_log() {
    let engine = Engine::default();
    let module = Module::new(&engine, QUICKJS_RULES_HOST).unwrap();
    check_imports(&module).unwrap();
    let imports: Vec<String> = module
        .imports()
        .map(|i| format!("{}.{}", i.module(), i.name()))
        .collect();
    assert_eq!(imports, vec!["env.log"]);
}

/// A complete (tiny) game written in plain JavaScript against the ABI.
const JS_COUNTER: &str = r#"
function handle(req) {
  switch (req.call) {
    case "info": return { min_seats: 1, max_seats: 2, api: 1 };
    case "setup": return { state: { n: 0, names: req.seats.map(s => s.name) } };
    case "on_action": {
      if (req.input.widget === "spin") { for (;;) {} }
      if (req.input.widget === "hog") { const a = []; for (;;) a.push(new Array(100000).fill(1)); }
      if (req.input.widget === "throw") throw new Error("bad \"move\"");
      const n = req.state.n + 1;
      __hostable_log("count " + n);
      const out = { state: { n, names: req.state.names }, events: [], waiting_on: [] };
      if (n >= 3) out.finished = { ranking: [[0]], summary: "done" };
      return out;
    }
    case "on_timeout": return { state: req.state };
    case "table_view": return { title: "Counter", status: "at " + req.state.n, players: [], log: [] };
    case "phone_view": return { type: "button", id: "add", label: "Add (" + req.state.n + ")" };
    case "can_respond": return Date.now() === 0;
  }
  throw new Error("unknown call " + req.call);
}
globalThis.__hostable_dispatch = (text) => {
  try { return JSON.stringify({ ok: handle(JSON.parse(text)) }); }
  catch (e) { return JSON.stringify({ err: { kind: "game", message: String(e && e.message || e) } }); }
};
"#;

#[test]
fn javascript_games_run_in_quickjs() {
    let rules = WasmRules::javascript(JS_COUNTER.as_bytes(), Limits::DEFAULT).unwrap();
    assert_eq!(rules.seats(), (1, 2));
    let mut state = rules.setup(&seats(), 9).unwrap().state;
    assert_eq!(state["names"], json!(["Ana", "Bo"]));
    for _ in 0..3 {
        let out = rules
            .on_action(state, 0, &PhoneInput::press("add"))
            .unwrap();
        state = out.state;
        if out.finished.is_some() {
            break;
        }
    }
    assert_eq!(state["n"], json!(3));
    assert!(rules.take_stats().logs.contains(&"count 3".to_string()));
    let view = rules.phone_view(&state, 0).unwrap();
    assert!(view.has_input("add"));
    // The clock is frozen, so scripts stay deterministic.
    assert!(rules.can_respond(&state, 0).unwrap());
    let table = rules.table_view(&state).unwrap();
    assert_eq!(table["status"], json!("at 3"));
}

#[test]
fn javascript_errors_loops_and_memory_hogs_are_contained() {
    let rules = WasmRules::javascript(JS_COUNTER.as_bytes(), Limits::DEFAULT).unwrap();
    let state = rules.setup(&seats(), 1).unwrap().state;
    assert_eq!(
        rules.on_action(state.clone(), 0, &PhoneInput::press("throw")),
        Err(RulesError::Game("bad \"move\"".into()))
    );
    assert_eq!(
        rules.on_action(state.clone(), 0, &PhoneInput::press("spin")),
        Err(RulesError::OutOfFuel)
    );
    let hog = rules.on_action(state.clone(), 0, &PhoneInput::press("hog"));
    assert!(
        matches!(
            hog,
            Err(RulesError::OutOfMemory) | Err(RulesError::Crashed(_)) | Err(RulesError::OutOfFuel)
        ),
        "{hog:?}"
    );
    // And it still plays afterwards.
    let out = rules
        .on_action(state, 0, &PhoneInput::press("add"))
        .unwrap();
    assert_eq!(out.state["n"], json!(1));
}

#[test]
fn a_broken_script_fails_to_load() {
    let err = WasmRules::javascript(b"this is ( not javascript", Limits::DEFAULT).err();
    assert!(matches!(err, Some(RulesError::Crashed(m)) if m.contains("SyntaxError")));
    let err = WasmRules::javascript(b"// no dispatch", Limits::DEFAULT).err();
    assert!(err.is_some());
}

#[test]
fn fuel_is_about_fifty_ms() {
    // Measure how long the default budget lasts on this machine. It should
    // be tens of milliseconds in an optimised build: enough for any
    // reasonable move, short enough that a stuck game never freezes a host.
    let rules = WasmRules::new(&fixture("loop"), Limits::DEFAULT).unwrap();
    let state = rules.setup(&seats(), 1).unwrap().state;
    let started = Instant::now();
    let _ = rules.on_action(state, 0, &PhoneInput::press("go"));
    let took = started.elapsed();
    eprintln!("default fuel lasted {took:?}");
    assert!(took < Duration::from_secs(3), "fuel budget took {took:?}");
}
