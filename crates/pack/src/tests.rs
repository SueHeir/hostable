use super::*;
use serde_json::json;

const JS_GAME: &str = r#"
function handle(req) {
  switch (req.call) {
    case "info": return { min_seats: 2, max_seats: 2, api: 1 };
    case "setup": return { state: { n: 0 } };
    case "on_action": {
      const n = req.state.n + 1;
      const out = { state: { n } };
      if (n >= 6) out.finished = { ranking: [[0, 1]], summary: "done" };
      return out;
    }
    case "on_timeout": return { state: req.state };
    case "table_view": return { title: "Count", status: "n=" + req.state.n, players: [], log: [] };
    case "phone_view": return { type: "button", id: "go", label: "Go" };
    case "can_respond": return true;
  }
}
globalThis.__hostable_dispatch = (t) => JSON.stringify({ ok: handle(JSON.parse(t)) });
"#;

fn manifest(id: &str, rules: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "id": id, "name": "Test", "version": "1.0.0",
        "players": {"min": 2, "max": 2}, "api": 1, "rules": rules
    }))
    .unwrap()
}

fn js_pack(id: &str) -> Pack {
    let mut pack = Pack::default();
    pack.files
        .insert("manifest.json".into(), manifest(id, "rules.js"));
    pack.files
        .insert("rules.js".into(), JS_GAME.as_bytes().to_vec());
    pack
}

fn wat_pack(body: &str) -> Pack {
    let info = r#"{"ok":{"min_seats":2,"max_seats":2,"api":1}}"#;
    let wat = format!(
        r#"(module (memory (export "memory") 1)
  (data (i32.const 64) "{}")
  (func (export "hostable_abi_version") (result i32) (i32.const 1))
  (func (export "hostable_alloc") (param i32) (result i32) (i32.const 1024))
  (func (export "hostable_free") (param i32 i32))
  (func (export "hostable_call") (param $p i32) (param $n i32) (result i64)
    (if (i32.eq (i32.load8_u (i32.add (local.get $p) (i32.const 9))) (i32.const 105))
      (then (return (i64.or (i64.shl (i64.const 64) (i64.const 32)) (i64.const {})))))
    {body}))"#,
        info.replace('"', "\\\""),
        info.len()
    );
    let mut pack = Pack::default();
    pack.files
        .insert("manifest.json".into(), manifest("broken", "rules.wasm"));
    pack.files
        .insert("rules.wasm".into(), wat::parse_str(&wat).unwrap());
    pack
}

fn signed(mut pack: Pack, key: &SigningKey) -> Vec<u8> {
    pack.sign(key).unwrap();
    pack.write().unwrap()
}

fn kinds(bytes: &[u8], options: &CheckOptions) -> Vec<IssueKind> {
    check_pack(bytes, options)
        .issues
        .iter()
        .map(|i| i.kind)
        .collect()
}

#[test]
fn a_signed_javascript_pack_checks_out_and_plays() {
    let key = generate_key();
    let bytes = signed(js_pack("counter"), &key);
    let report = check_pack(&bytes, &CheckOptions::default());
    assert!(report.ok(), "{:?}", report.issues);
    assert_eq!(report.games_played, 3);
    assert_eq!(report.games_finished, 3);
    let pack = Pack::read(&bytes).unwrap();
    let game = load_rules(&pack, hostable_sandbox::Limits::DEFAULT).unwrap();
    assert_eq!(game.manifest.id, "counter");
    assert_eq!(game.author_key, public_key_b64(&key));
}

#[test]
fn unsigned_and_tampered_packs_fail() {
    let bytes = js_pack("counter").write().unwrap();
    assert_eq!(
        kinds(&bytes, &CheckOptions::default()),
        vec![IssueKind::Unsigned]
    );

    let key = generate_key();
    let mut pack = Pack::read(&signed(js_pack("counter"), &key)).unwrap();
    pack.files
        .insert("rules.js".into(), b"/* swapped */".to_vec());
    let bytes = pack.write().unwrap();
    assert_eq!(
        kinds(&bytes, &CheckOptions::default()),
        vec![IssueKind::BadSignature]
    );
}

#[test]
fn a_new_version_signed_by_another_key_is_refused() {
    let first = generate_key();
    let other = generate_key();
    let mut pins = Pins::default();
    assert!(pins.pin("counter", &public_key_b64(&first)).unwrap());
    let options = CheckOptions {
        pins: Some(pins.clone()),
        ..CheckOptions::default()
    };
    assert!(check_pack(&signed(js_pack("counter"), &first), &options).ok());
    assert_eq!(
        kinds(&signed(js_pack("counter"), &other), &options),
        vec![IssueKind::KeyMismatch]
    );
    // Swapping the key in the manifest after signing breaks the signature.
    let mut pack = Pack::read(&signed(js_pack("counter"), &other)).unwrap();
    let mut m: serde_json::Value = serde_json::from_slice(&pack.files["manifest.json"]).unwrap();
    m["author_key"] = json!(public_key_b64(&first));
    pack.files
        .insert("manifest.json".into(), serde_json::to_vec(&m).unwrap());
    let report = check_pack(&pack.write().unwrap(), &options);
    assert!(matches!(
        report.issues[0].kind,
        IssueKind::KeyMismatch | IssueKind::BadSignature
    ));
    assert!(pins.pin("counter", &public_key_b64(&other)).is_err());
}

#[test]
fn only_known_file_types_are_allowed() {
    for bad in [
        "assets/run.exe",
        "assets/page.html",
        "index.html",
        "assets/x.svg",
        "evil.js",
        "ui/a.png",
    ] {
        let mut pack = js_pack("counter");
        pack.files.insert(bad.into(), b"x".to_vec());
        let bytes = signed(pack, &generate_key());
        assert_eq!(
            kinds(&bytes, &CheckOptions::default()),
            vec![IssueKind::ForbiddenFile],
            "{bad}"
        );
    }
    for good in [
        "assets/card.png",
        "assets/a/b/c.ogg",
        "ui/main.json",
        "locales/fr.json",
        "README.md",
    ] {
        assert!(allowed_file(good), "{good}");
    }
}

#[test]
fn paths_cannot_escape() {
    for path in ["../x", "/etc/passwd", "a/../b", "a//b", "a\\b"] {
        assert!(!path_ok(path), "{path}");
    }
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file("../evil.png", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(b"x").unwrap();
    let bytes = zip.finish().unwrap().into_inner();
    assert!(matches!(Pack::read(&bytes), Err(PackError::Unreadable(_))));
}

#[test]
fn packs_over_two_hundred_megabytes_are_refused_while_reading() {
    let mut pack = js_pack("counter");
    pack.files.insert(
        "assets/big.bin".into(),
        vec![0u8; (MAX_PACK_BYTES + 1) as usize],
    );
    let bytes = signed(pack, &generate_key());
    // Zeros compress to almost nothing; the cap is on what they unpack to.
    assert!(bytes.len() < 2 * 1024 * 1024);
    assert_eq!(
        kinds(&bytes, &CheckOptions::default()),
        vec![IssueKind::TooLarge]
    );
}

#[test]
fn manifests_are_validated() {
    let key = generate_key();
    let mut pack = js_pack("Bad Id!");
    pack.files
        .insert("manifest.json".into(), manifest("Bad Id!", "rules.js"));
    assert_eq!(
        kinds(&signed(pack, &key), &CheckOptions::default()),
        vec![IssueKind::BadManifest]
    );
    let mut pack = js_pack("counter");
    pack.files.remove("rules.js");
    assert_eq!(
        kinds(&signed(pack, &key), &CheckOptions::default()),
        vec![IssueKind::BadManifest]
    );
    let mut pack = js_pack("counter");
    let mut m: serde_json::Value = serde_json::from_slice(&pack.files["manifest.json"]).unwrap();
    m["players"] = json!({"min": 3, "max": 4});
    pack.files
        .insert("manifest.json".into(), serde_json::to_vec(&m).unwrap());
    // The rules say 2-2, the manifest 3-4.
    assert_eq!(
        kinds(&signed(pack, &key), &CheckOptions::default()),
        vec![IssueKind::BadManifest]
    );
    let mut pack = js_pack("counter");
    let mut m: serde_json::Value = serde_json::from_slice(&pack.files["manifest.json"]).unwrap();
    m["surprise"] = json!(true);
    pack.files
        .insert("manifest.json".into(), serde_json::to_vec(&m).unwrap());
    assert_eq!(
        kinds(&signed(pack, &key), &CheckOptions::default()),
        vec![IssueKind::BadManifest]
    );
}

#[test]
fn crashing_and_looping_rules_fail_the_seeded_games() {
    let key = generate_key();
    let crash = signed(wat_pack("(unreachable)"), &key);
    assert_eq!(
        kinds(&crash, &CheckOptions::default()),
        vec![IssueKind::RulesCrashed]
    );
    let spin = signed(wat_pack("(loop $l (br $l)) (unreachable)"), &key);
    assert_eq!(
        kinds(&spin, &CheckOptions::default()),
        vec![IssueKind::RulesTimeout]
    );
    let bomb = signed(
        wat_pack("(loop $l (br_if $l (i32.ne (memory.grow (i32.const 16)) (i32.const -1)))) (unreachable)"),
        &key,
    );
    assert_eq!(
        kinds(&bomb, &CheckOptions::default()),
        vec![IssueKind::RulesMemory]
    );
}

#[test]
fn sandbox_packs_load_the_tabletop_engine() {
    let mut pack = Pack::default();
    pack.files.insert(
        "manifest.json".into(),
        serde_json::to_vec(&json!({
            "id": "dice-table", "name": "Dice", "version": "0.1.0",
            "players": {"min": 1, "max": 6}, "api": 1, "level": "sandbox", "table": "table.json"
        }))
        .unwrap(),
    );
    pack.files.insert(
        "table.json".into(),
        serde_json::to_vec(&json!({"title": "Dice", "width": 400, "height": 300,
            "objects": [{"kind": "die", "id": "d", "x": 10, "y": 10, "sides": 6}]}))
        .unwrap(),
    );
    let report = check_pack(&signed(pack, &generate_key()), &CheckOptions::default());
    assert!(report.ok(), "{:?}", report.issues);
}

#[test]
fn key_files_round_trip() {
    let key = generate_key();
    let text = key_to_file(&key);
    assert!(text.contains("never commit"));
    assert_eq!(key_from_file(&text).unwrap().to_bytes(), key.to_bytes());
    assert!(key_from_file("# nothing").is_err());
}
