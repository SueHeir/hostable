//! `hostable-pack`: build, sign and check Hostable game packs.
//!
//! ```text
//! hostable-pack keygen --out KEYFILE
//! hostable-pack build DIR --out FILE.gamepack [--key KEYFILE]
//! hostable-pack sign FILE.gamepack --key KEYFILE
//! hostable-pack check FILE.gamepack [--pins PINS.json [--pin]] [--seeds N] [--json]
//! hostable-pack inspect FILE.gamepack
//! ```
//!
//! A pack source directory holds `manifest.json` and the pack's files. The
//! manifest may say how to build the rules, which is removed from the built
//! pack:
//!
//! - `"build": {"rust": "<crate name>"}` builds that workspace crate for
//!   `wasm32-unknown-unknown` and packs it as `rules.wasm`;
//! - `"build": {"npm": "<dir>"}` runs `npm run build` there and packs its
//!   `dist/rules.js`;
//! - `"build": {"wat": "rules.wat"}` assembles a WebAssembly text file.

use std::path::{Path, PathBuf};
use std::process::{exit, Command};

use hostable_pack::{
    check_pack, generate_key, key_from_file, key_to_file, public_key_b64, CheckOptions, Pack, Pins,
    MANIFEST_FILE,
};
use serde_json::Value;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("keygen") => keygen(&args[1..]),
        Some("build") => build(&args[1..]),
        Some("sign") => sign(&args[1..]),
        Some("check") => check(&args[1..]),
        Some("inspect") => inspect(&args[1..]),
        _ => Err(USAGE.to_string()),
    };
    match result {
        Ok(code) => exit(code),
        Err(message) => {
            eprintln!("hostable-pack: {message}");
            exit(2);
        }
    }
}

const USAGE: &str = "usage:
  hostable-pack keygen --out KEYFILE
  hostable-pack build DIR --out FILE.gamepack [--key KEYFILE]
  hostable-pack sign FILE.gamepack --key KEYFILE
  hostable-pack check FILE.gamepack [--pins PINS.json [--pin]] [--seeds N] [--json]
  hostable-pack inspect FILE.gamepack";

struct Args {
    positional: Vec<String>,
    flags: Vec<(String, Option<String>)>,
}

impl Args {
    fn parse(args: &[String], with_value: &[&str]) -> Result<Self, String> {
        let mut positional = Vec::new();
        let mut flags = Vec::new();
        let mut it = args.iter();
        while let Some(a) = it.next() {
            if let Some(name) = a.strip_prefix("--") {
                if with_value.contains(&name) {
                    let v = it.next().ok_or(format!("--{name} needs a value"))?;
                    flags.push((name.to_string(), Some(v.clone())));
                } else {
                    flags.push((name.to_string(), None));
                }
            } else {
                positional.push(a.clone());
            }
        }
        Ok(Self { positional, flags })
    }

    fn value(&self, name: &str) -> Option<&str> {
        self.flags
            .iter()
            .find(|(n, _)| n == name)
            .and_then(|(_, v)| v.as_deref())
    }

    fn has(&self, name: &str) -> bool {
        self.flags.iter().any(|(n, _)| n == name)
    }

    fn one(&self, what: &str) -> Result<&str, String> {
        match self.positional.as_slice() {
            [one] => Ok(one),
            _ => Err(format!("expected one {what}\n{USAGE}")),
        }
    }
}

fn read(path: &str) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|e| format!("{path}: {e}"))
}

fn keygen(args: &[String]) -> Result<i32, String> {
    let args = Args::parse(args, &["out"])?;
    let out = args.value("out").ok_or("keygen needs --out")?;
    if Path::new(out).exists() {
        return Err(format!("{out} already exists; not overwriting a key"));
    }
    let key = generate_key();
    if let Some(dir) = Path::new(out).parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(out, key_to_file(&key)).map_err(|e| format!("{out}: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(out, std::fs::Permissions::from_mode(0o600));
    }
    println!("{}", public_key_b64(&key));
    Ok(0)
}

fn load_key(path: &str) -> Result<ed25519_dalek::SigningKey, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    key_from_file(&text).map_err(|e| e.to_string())
}

fn build(args: &[String]) -> Result<i32, String> {
    let args = Args::parse(args, &["out", "key"])?;
    let dir = PathBuf::from(args.one("pack directory")?);
    let out = args.value("out").ok_or("build needs --out")?;
    let manifest_path = dir.join(MANIFEST_FILE);
    let mut manifest: Value = serde_json::from_slice(&read(&manifest_path.to_string_lossy())?)
        .map_err(|e| format!("{}: {e}", manifest_path.display()))?;
    let recipe = manifest
        .as_object_mut()
        .and_then(|m| m.remove("build"))
        .unwrap_or(Value::Null);
    let mut pack = Pack::default();
    collect_files(&dir, &dir, &mut pack)?;
    pack.files.remove("rules.wat");
    if let Some(krate) = recipe.get("rust").and_then(Value::as_str) {
        pack.files
            .insert("rules.wasm".into(), build_rust(&dir, krate)?);
    } else if let Some(npm_dir) = recipe.get("npm").and_then(Value::as_str) {
        pack.files
            .insert("rules.js".into(), build_npm(&dir.join(npm_dir))?);
    } else if let Some(wat_file) = recipe.get("wat").and_then(Value::as_str) {
        let wasm = wat::parse_file(dir.join(wat_file)).map_err(|e| e.to_string())?;
        pack.files.insert("rules.wasm".into(), wasm);
    } else if !recipe.is_null() {
        return Err(format!("unknown build recipe {recipe}"));
    }
    pack.files.insert(
        MANIFEST_FILE.into(),
        serde_json::to_vec_pretty(&manifest).expect("json"),
    );
    if let Some(key) = args.value("key") {
        pack.sign(&load_key(key)?).map_err(|e| e.to_string())?;
    }
    let bytes = pack.write().map_err(|e| e.to_string())?;
    if let Some(parent) = Path::new(out).parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(out, &bytes).map_err(|e| format!("{out}: {e}"))?;
    println!(
        "built {out}: {} files, {} bytes{}",
        pack.files.len(),
        bytes.len(),
        if args.has("key") || args.value("key").is_some() {
            ", signed"
        } else {
            ", unsigned"
        }
    );
    Ok(0)
}

/// Every file under `dir` except build inputs, by relative path.
fn collect_files(root: &Path, dir: &Path, pack: &mut Pack) -> Result<(), String> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(Result::ok)
        .collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        let rel = path
            .strip_prefix(root)
            .expect("under root")
            .to_string_lossy()
            .replace('\\', "/");
        let name = entry.file_name().to_string_lossy().to_string();
        // Build inputs and tooling never go into a pack.
        let skip = [
            "src",
            "node_modules",
            "dist",
            "package.json",
            "package-lock.json",
            "tsconfig.json",
            "fixture.json",
            ".gitignore",
            MANIFEST_FILE,
        ];
        if (dir == root && skip.contains(&name.as_str())) || name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            collect_files(root, &path, pack)?;
        } else {
            pack.files.insert(rel, read(&path.to_string_lossy())?);
        }
    }
    Ok(())
}

fn cargo() -> String {
    std::env::var("CARGO").unwrap_or_else(|_| "cargo".into())
}

fn build_rust(dir: &Path, krate: &str) -> Result<Vec<u8>, String> {
    let status = Command::new(cargo())
        .current_dir(dir)
        .args([
            "build",
            "--quiet",
            "-p",
            krate,
            "--target",
            "wasm32-unknown-unknown",
            "--release",
        ])
        .status()
        .map_err(|e| format!("running cargo: {e}"))?;
    if !status.success() {
        return Err(format!("cargo build of {krate} failed"));
    }
    let meta = Command::new(cargo())
        .current_dir(dir)
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .output()
        .map_err(|e| format!("cargo metadata: {e}"))?;
    let meta: Value = serde_json::from_slice(&meta.stdout).map_err(|e| e.to_string())?;
    let target = meta["target_directory"]
        .as_str()
        .ok_or("no target directory")?;
    let file = Path::new(target)
        .join("wasm32-unknown-unknown/release")
        .join(format!("{}.wasm", krate.replace('-', "_")));
    read(&file.to_string_lossy())
}

fn build_npm(dir: &Path) -> Result<Vec<u8>, String> {
    let npm = if cfg!(windows) { "npm.cmd" } else { "npm" };
    let status = Command::new(npm)
        .current_dir(dir)
        .args(["run", "build", "--silent"])
        .status()
        .map_err(|e| format!("running npm: {e}"))?;
    if !status.success() {
        return Err(format!("npm run build in {} failed", dir.display()));
    }
    read(&dir.join("dist/rules.js").to_string_lossy())
}

fn sign(args: &[String]) -> Result<i32, String> {
    let args = Args::parse(args, &["key", "out"])?;
    let file = args.one("pack file")?;
    let key = load_key(args.value("key").ok_or("sign needs --key")?)?;
    let mut pack = Pack::read(&read(file)?).map_err(|e| e.to_string())?;
    pack.sign(&key).map_err(|e| e.to_string())?;
    let out = args.value("out").unwrap_or(file);
    std::fs::write(out, pack.write().map_err(|e| e.to_string())?)
        .map_err(|e| format!("{out}: {e}"))?;
    println!("signed {out} with {}", public_key_b64(&key));
    Ok(0)
}

fn check(args: &[String]) -> Result<i32, String> {
    let args = Args::parse(args, &["pins", "seeds"])?;
    let file = args.one("pack file")?;
    let mut options = CheckOptions::default();
    let pins_path = args.value("pins");
    if let Some(path) = pins_path {
        options.pins = Some(match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| format!("{path}: {e}"))?,
            Err(_) => Pins::default(),
        });
    }
    if let Some(n) = args.value("seeds") {
        options.seeds = n.parse().map_err(|_| "--seeds takes a number")?;
    }
    let report = check_pack(&read(file)?, &options);
    if args.has("json") {
        println!("{}", serde_json::to_string_pretty(&report).expect("json"));
    }
    if !report.ok() {
        for issue in &report.issues {
            println!("check: FAIL {}: {}", issue.kind.code(), issue.message);
        }
        return Ok(1);
    }
    if args.has("pin") {
        let path = pins_path.ok_or("--pin needs --pins")?;
        let pack = Pack::read(&read(file)?).map_err(|e| e.to_string())?;
        let manifest = pack.manifest().map_err(|e| e.to_string())?;
        let mut pins = options.pins.unwrap_or_default();
        pins.pin(&manifest.id, &manifest.author_key)
            .map_err(|e| e.to_string())?;
        std::fs::write(path, serde_json::to_vec_pretty(&pins).expect("json"))
            .map_err(|e| format!("{path}: {e}"))?;
    }
    println!(
        "check: OK {} {} ({} games, {} finished, {} moves)",
        report.id.as_deref().unwrap_or("?"),
        report.version.as_deref().unwrap_or("?"),
        report.games_played,
        report.games_finished,
        report.moves
    );
    Ok(0)
}

fn inspect(args: &[String]) -> Result<i32, String> {
    let args = Args::parse(args, &[])?;
    let file = args.one("pack file")?;
    let pack = Pack::read(&read(file)?).map_err(|e| e.to_string())?;
    if let Some(m) = pack.files.get(MANIFEST_FILE) {
        println!("{}", String::from_utf8_lossy(m));
    }
    for (name, data) in &pack.files {
        println!("{:>12}  {name}", data.len());
    }
    Ok(0)
}
