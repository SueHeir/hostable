//! The pack checker: everything a registry runs before accepting a pack.
//!
//! It validates the files (types, paths, the 200 MB cap), the manifest, the
//! signature and the pinned author key, then plays a few seeded games with
//! bots on every seat through the real sandbox and host core. Any crash,
//! timeout (out of fuel), memory blow-up, malformed outcome or stall fails
//! the check.

use std::sync::{Arc, Mutex};

use hostable_host_core::{bot, Effect, Room};
use hostable_protocol::{PhoneInput, Widget};
use hostable_rules_api::{DynRules, Outcome, Rng, Rules, RulesError, RulesResult, Seat, SeatId};
use hostable_sandbox::Limits;
use serde::Serialize;
use serde_json::Value;

use crate::{load_rules, Pack, PackError, Pins};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum IssueKind {
    Unreadable,
    TooLarge,
    BadManifest,
    ForbiddenFile,
    Unsigned,
    BadSignature,
    KeyMismatch,
    RulesCrashed,
    RulesTimeout,
    RulesMemory,
    RulesMalformed,
    RulesStuck,
}

impl IssueKind {
    pub fn code(self) -> &'static str {
        match self {
            IssueKind::Unreadable => "unreadable",
            IssueKind::TooLarge => "too-large",
            IssueKind::BadManifest => "bad-manifest",
            IssueKind::ForbiddenFile => "forbidden-file",
            IssueKind::Unsigned => "unsigned",
            IssueKind::BadSignature => "bad-signature",
            IssueKind::KeyMismatch => "key-mismatch",
            IssueKind::RulesCrashed => "rules-crashed",
            IssueKind::RulesTimeout => "rules-timeout",
            IssueKind::RulesMemory => "rules-memory",
            IssueKind::RulesMalformed => "rules-malformed",
            IssueKind::RulesStuck => "rules-stuck",
        }
    }
}

impl From<&PackError> for IssueKind {
    fn from(e: &PackError) -> Self {
        match e {
            PackError::Unreadable(_) => IssueKind::Unreadable,
            PackError::TooLarge(_) => IssueKind::TooLarge,
            PackError::BadManifest(_) => IssueKind::BadManifest,
            PackError::ForbiddenFile(_) => IssueKind::ForbiddenFile,
            PackError::Unsigned => IssueKind::Unsigned,
            PackError::BadSignature(_) => IssueKind::BadSignature,
            PackError::KeyMismatch(_) => IssueKind::KeyMismatch,
        }
    }
}

impl From<&RulesError> for IssueKind {
    fn from(e: &RulesError) -> Self {
        match e {
            RulesError::OutOfFuel => IssueKind::RulesTimeout,
            RulesError::OutOfMemory => IssueKind::RulesMemory,
            RulesError::Malformed(_) => IssueKind::RulesMalformed,
            RulesError::Crashed(_) | RulesError::Game(_) => IssueKind::RulesCrashed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Issue {
    pub kind: IssueKind,
    pub message: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Report {
    pub id: Option<String>,
    pub version: Option<String>,
    pub issues: Vec<Issue>,
    pub games_played: u32,
    pub games_finished: u32,
    pub moves: u64,
}

impl Report {
    pub fn ok(&self) -> bool {
        self.issues.is_empty()
    }

    fn fail(&mut self, kind: IssueKind, message: impl Into<String>) {
        self.issues.push(Issue {
            kind,
            message: message.into(),
        });
    }
}

#[derive(Debug, Clone)]
pub struct CheckOptions {
    /// Pinned author keys; a pack whose id is pinned to another key fails.
    pub pins: Option<Pins>,
    /// Seeded games per seat count.
    pub seeds: u32,
    /// Inputs per game before giving up (a long game isn't an error).
    pub max_moves: u32,
    pub limits: Limits,
}

impl Default for CheckOptions {
    fn default() -> Self {
        Self {
            pins: None,
            seeds: 3,
            max_moves: 600,
            limits: Limits::DEFAULT,
        }
    }
}

/// Check a `.gamepack` file's bytes.
pub fn check_pack(bytes: &[u8], options: &CheckOptions) -> Report {
    let mut report = Report::default();
    if bytes.len() as u64 > crate::MAX_PACK_BYTES {
        report.fail(IssueKind::TooLarge, "the file itself is over 200 MB");
        return report;
    }
    let pack = match Pack::read(bytes) {
        Ok(p) => p,
        Err(e) => {
            report.fail((&e).into(), e.to_string());
            return report;
        }
    };
    if let Err(e) = pack.check_files() {
        report.fail((&e).into(), e.to_string());
        return report;
    }
    let manifest = match pack.manifest() {
        Ok(m) => m,
        Err(e) => {
            report.fail((&e).into(), e.to_string());
            return report;
        }
    };
    report.id = Some(manifest.id.clone());
    report.version = Some(manifest.version.clone());
    let key = match pack.verify(&manifest) {
        Ok(k) => k,
        Err(e) => {
            report.fail((&e).into(), e.to_string());
            return report;
        }
    };
    if let Some(pins) = &options.pins {
        if let Err(e) = pins.check(&manifest.id, &key) {
            report.fail((&e).into(), e.to_string());
            return report;
        }
    }
    let game = match load_rules(&pack, options.limits) {
        Ok(g) => g,
        Err(e) => {
            // Loading runs the module's `info`; classify sandbox failures.
            let message = e.to_string();
            let kind = if message.contains("ran too long") {
                IssueKind::RulesTimeout
            } else if message.contains("too much memory") {
                IssueKind::RulesMemory
            } else if message.contains("crashed") {
                IssueKind::RulesCrashed
            } else if message.contains("rules.wasm") || message.contains("rules.js") {
                IssueKind::RulesMalformed
            } else {
                (&e).into()
            };
            report.fail(kind, message);
            return report;
        }
    };
    let recorder = Recorder::new(game.rules);
    let (min, max) = recorder.seats();
    let mut counts = vec![min];
    if max > min {
        counts.push(max.min(min + 2));
    }
    for n in counts {
        for seed in 1..=options.seeds {
            if let Err(issue) = play(&recorder, n, seed as u64, options, &mut report) {
                report.issues.push(issue);
                return report;
            }
        }
    }
    report
}

/// Play one game with bots on every seat.
fn play(
    rules: &Recorder,
    seats: u32,
    seed: u64,
    options: &CheckOptions,
    report: &mut Report,
) -> Result<(), Issue> {
    let mut room = Room::new(rules.clone(), seed);
    for p in 0..seats {
        room.player_joined(p + 1, format!("Bot {}", p + 1), String::new(), 0);
    }
    let mut now = 0u64;
    let started = room.start(now);
    if let Err(e) = started {
        return Err(rules.issue().unwrap_or(Issue {
            kind: IssueKind::RulesCrashed,
            message: format!("setup: {e}"),
        }));
    }
    report.games_played += 1;
    let mut rng = Rng::new(seed ^ 0xC0FFEE);
    for _ in 0..options.max_moves {
        if room.finished().is_some() {
            report.games_finished += 1;
            return Ok(());
        }
        if let Some(issue) = rules.issue() {
            return Err(issue);
        }
        if let Some(msg) = room.paused() {
            return Err(Issue {
                kind: IssueKind::RulesMalformed,
                message: msg.to_string(),
            });
        }
        if let Err(e) = room.table_view().unwrap_or(Ok(Value::Null)) {
            return Err(Issue {
                kind: (&e).into(),
                message: format!("table view: {e}"),
            });
        }
        if let Some(Ok(table)) = room.table_view() {
            if let Err(e) = hostable_host_core::check_table(&table) {
                return Err(Issue {
                    kind: IssueKind::RulesMalformed,
                    message: e.to_string(),
                });
            }
        }
        // Players with something to press, in random order.
        let mut options_now: Vec<(u32, PhoneInput)> = Vec::new();
        for p in 1..=seats {
            if let Some(ui) = room.current_ui(p) {
                if let Some(input) = bot::random_input(ui, &mut rng, &[]) {
                    options_now.push((p, input));
                }
            }
        }
        now += 250;
        if options_now.is_empty() {
            match room.next_deadline() {
                Some(deadline) => {
                    now = now.max(deadline);
                    let effects = room.tick(now);
                    check_effects(&effects, rules)?;
                }
                None => {
                    return Err(Issue {
                        kind: IssueKind::RulesStuck,
                        message: "nobody can do anything and nothing is pending".into(),
                    })
                }
            }
            continue;
        }
        let (player, input) = options_now.swap_remove(rng.below(options_now.len() as u32) as usize);
        let effects = room.handle_input(player, input, now).map_err(|e| Issue {
            kind: IssueKind::RulesCrashed,
            message: e.to_string(),
        })?;
        report.moves += 1;
        check_effects(&effects, rules)?;
    }
    if room.finished().is_some() {
        report.games_finished += 1;
    }
    Ok(())
}

fn check_effects(effects: &[Effect], rules: &Recorder) -> Result<(), Issue> {
    for e in effects {
        if let Effect::Paused(msg) = e {
            return Err(rules.issue().unwrap_or(Issue {
                kind: IssueKind::RulesMalformed,
                message: msg.clone(),
            }));
        }
    }
    Ok(())
}

/// Wraps the rules to remember the first error and the fuel used.
#[derive(Clone)]
struct Recorder {
    inner: Arc<Mutex<Box<DynRules>>>,
    error: Arc<Mutex<Option<RulesError>>>,
}

impl Recorder {
    fn new(rules: Box<DynRules>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(rules)),
            error: Arc::new(Mutex::new(None)),
        }
    }

    fn issue(&self) -> Option<Issue> {
        self.error.lock().unwrap().as_ref().map(|e| Issue {
            kind: e.into(),
            message: e.to_string(),
        })
    }

    fn record<T>(&self, r: RulesResult<T>) -> RulesResult<T> {
        if let Err(e) = &r {
            let mut slot = self.error.lock().unwrap();
            if slot.is_none() {
                *slot = Some(e.clone());
            }
        }
        r
    }
}

impl Rules for Recorder {
    type State = Value;

    fn seats(&self) -> (u32, u32) {
        self.inner.lock().unwrap().seats()
    }
    fn setup(&self, seats: &[Seat], seed: u64) -> RulesResult<Outcome<Value>> {
        let r = self.inner.lock().unwrap().setup(seats, seed);
        self.record(r)
    }
    fn on_action(&self, s: Value, seat: SeatId, input: &PhoneInput) -> RulesResult<Outcome<Value>> {
        let r = self.inner.lock().unwrap().on_action(s, seat, input);
        self.record(r)
    }
    fn on_timeout(&self, s: Value, seat: SeatId, now: u64) -> RulesResult<Outcome<Value>> {
        let r = self.inner.lock().unwrap().on_timeout(s, seat, now);
        self.record(r)
    }
    fn table_view(&self, s: &Value) -> RulesResult<Value> {
        let r = self.inner.lock().unwrap().table_view(s);
        self.record(r)
    }
    fn phone_view(&self, s: &Value, seat: SeatId) -> RulesResult<Widget> {
        let r = self.inner.lock().unwrap().phone_view(s, seat);
        self.record(r)
    }
    fn can_respond(&self, s: &Value, seat: SeatId) -> RulesResult<bool> {
        let r = self.inner.lock().unwrap().can_respond(s, seat);
        self.record(r)
    }
}
