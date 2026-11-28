# GOAL: Build Hostable out to the MVP in docs/design.md (phases 1 to 7), proven by one green verify script

> Self-contained implementation brief. Authored 2026-11-27 by goalify. Runs in a fresh session.
> This file's own path: `/Users/suehr/Documents/GitHub/hostable/.goal/hostable-mvp.md`
> (if the repo is cloned somewhere else, it is `.goal/hostable-mvp.md` at the repo root; use that
> absolute path everywhere below). Archived, not deleted, on success (see Archive gate).
> Re-read THIS file at the start of every work loop; it is the source of truth, not the conversation.
> **This file is the brief, not the stop condition.** The condition is a separate string (see
> `.goal/CONDITION-hostable-mvp.txt`).

## GOAL (the autonomous directive)

End state: the `hostable` repo (`github.com/SueHeir/hostable`) contains a working, tested Hostable
MVP as specified in `<repo>/docs/design.md`, phases 1 to 7, and `<repo>/scripts/verify.sh` runs every
check listed under Definition of done and ends by printing exactly `VERIFY: ALL GREEN`.

Work at MAXIMUM EFFORT. Fan out parallel subagents for independent discovery, research and
verification, but serialize builds, tests, same-file writes and every git operation. Correctness and
completeness over speed or token cost. Use the environment's max-effort mode if there is one. Verify
with a SEPARATE agent, check official docs (Bevy, wasmi, axum, QuickJS, Web Push, Playwright) when in
doubt, test everything, and do not stop until every success criterion holds.

## Context (verified 2026-11-27 — re-confirm live; don't trust this summary)

- Repo: `github.com/SueHeir/hostable`, MIT OR Apache-2.0. `main` has README and licenses. The
  skeleton lives on branch `claude/project-thread-k0ci42` (PR into `main`); start from `main` if
  that PR has merged, else from that branch.
  Skeleton crates, all tests green at authoring time (16 tests):
  - `crates/protocol` — wire messages (`HostToRelay`, `RelayToHost`, `PhoneToRelay`,
    `RelayToPhone`, `HostToPhone`, `PhoneInput`) and the `Widget` UI tree.
  - `crates/rules-api` — `Rules` trait, `Outcome`, `Wait`, `Event`, `Results`.
  - `crates/host-core` — `Room<R: Rules>`: seats, lobby, input validation against the current
    screen, UI diffing with versions, snapshots/restore, action log. No I/O.
  - `crates/relay` — `Registry` (codes, tokens, routing, host grace, sweep) and an axum server
    (`/ws/host`, `/ws/phone`, `/healthz`), binary `hostable-relay`.
  - `games/buzzer` — demo game implementing `Rules`.
  - `.github/workflows/ci.yml` — fmt, clippy `-D warnings`, test.
- Full spec: `<repo>/docs/design.md` (architecture, pack format, rules API, reaction windows, phone
  UI language, sessions, security, stack, phases). Read it first, every loop.
- Wizards: `github.com/SueHeir/wizards`, crate `wizards-rules` at `crates/rules`
  (MIT OR Apache-2.0, deps: serde, ron). Multi-player capable: `Game::new(BoardDef, SpellBook,
  Vec<Player>)`, `Game::apply(Action) -> Result<Vec<Event>, RuleError>`, `Game::is_pve()`,
  `spell_targets`, `auto_pay`. Art lives in `crates/client/src/art` (Bevy, code-built models, toon
  shader). Depend on it as a git dependency; **never modify the wizards repo.** A PvP mode is being
  planned in a separate thread; if no dedicated PvP API has landed, build the duel on the existing
  multi-player `Game` (two or more wizards, no monsters) and note it.
- Testing deployment target (do NOT deploy; only prepare config): `dinkydelvers.com/play` on the
  existing DigitalOcean droplet behind Caddy. See wizards `deploy/` and `docs/deploy.md` for the
  pattern.

## Decisions (locked by Liz — do not re-litigate)

- Open source, MIT OR Apache-2.0. Every new crate and package uses that license.
- Desktop host is a native Bevy app for Windows, macOS, Linux. Cloud host is the same host core,
  headless.
- Scripted games before the sandbox. Test games, in order: Wizards PvP; an ORIGINAL hex
  resource-trading game; an ORIGINAL two-player card duel with instant-speed responses.
- **IP rule:** the trading game and the duel borrow mechanics only. Our own names, art (code-drawn
  or generated in-repo), card names and card text. Never use "Catan", "Settlers", "Magic",
  "Gathering", "Wizards of the Coast", "Kosmos", real card names, or their art/rulebook wording
  anywhere outside `docs/design.md`'s decisions line.
- Other creators will write games, so packs are untrusted: sandbox, signing, checker, quotas.
- No physics or twitch input. Reaction windows of seconds are in scope.
- Remote and async play are in scope.
- Working name "Hostable". Domain undecided; use a config value, default `dinkydelvers.com/play`
  for testing.

## Phases

1. **Re-verify live.** `git status`, `git log`, check out the skeleton branch, run
   `cargo test --workspace`, read `docs/design.md`, confirm the wizards API above by reading its
   source. Create branch `goal/mvp` from the skeleton branch.
2. **Plumbing (design phase 1).**
   - `client/` (TypeScript + Preact + Vite, pnpm or npm): join screen (code + name), `/j/<CODE>`
     route, token in localStorage, auto-rejoin, renders every `Widget`, sends `PhoneInput`. Generate
     TS types from `crates/protocol` with `ts-rs` and fail CI if they drift. Gzipped JS for the
     join route under 150 KB.
   - `crates/desktop` (`hostable-desktop`, Bevy): opens a room on a configurable relay URL, shows
     code + QR, lobby list, start button, draws the table via each game's renderer, plays events.
     Must also have `--smoke-test`: run without a window (MinimalPlugins or headless), connect to a
     given relay, open a room, have N simulated phones join and play the buzzer to the end, exit 0.
   - Relay: per-phone rate limit (60/s), per-IP failed-join limit, message cap, all tested.
3. **Rules API v1 + sandbox + Wizards PvP (design phase 2).**
   - `crates/sandbox`: load `rules.wasm` in `wasmi` with fuel metering, 64 MB memory cap, no imports
     but `log`; JSON ABI matching `rules-api`; a `WasmRules` that implements `Rules`. Outcome
     schema checks. Tests: infinite loop → out-of-fuel error, memory bomb → error, malformed
     outcome → rejected, all without crashing the host.
   - `crates/sdk` (Rust): types + an export macro so a game crate compiles to `rules.wasm`.
   - Reaction windows in host-core: `Wait`, countdowns, `on_timeout`, `can_respond` auto-pass,
     live vs async timeouts. Tested.
   - Widgets: implement the full list in `docs/design.md` in protocol + client.
   - `games/wizards-duel`: wraps `wizards-rules`; phones get move / spell / end-turn controls; each
     wizard's spells and mana private. Native renderer in the desktop host reusing the wizards art
     approach (depend on code, don't copy if a crate dependency works; if the art module isn't a
     public crate, reimplement minimal visuals and note it).
   - `packs/`: buzzer and wizards-duel as `.gamepack` builds via `cargo xtask pack` (or a
     `hostable-pack build` command).
4. **Hex resource-trading game (design phase 3).** Original name and theme. Hex board, dice-driven
   production, building, a development-card-like deck under our own name, player-to-player trade
   offers and counter-offers on phones, 3 to 4 players, win at a point target. Compiled to wasm and
   run through the sandbox. Desktop renderer + a 2D `table_view` the web client can draw.
5. **Cloud host and async (design phase 4).** `crates/cloud` (`hostable-cloud`): host core in an
   axum server with SQLite (`sqlx` or `rusqlite`), games created over HTTP, share links
   `/g/<id>`, seat claim by link, state + log persisted after every action, restart-safe, turn
   timers, Web Push (VAPID keys from env, never committed) with a test double, desktop-to-cloud
   handoff (upload snapshot, same seat tokens), per-game wasm worker with CPU/memory quotas. Web
   client: table view for remote players, notification opt-in, async auto-pass settings.
6. **Card duel with instant responses (design phase 5).** Original name, cards, art. Two players,
   decks, resources, a stack of pending effects, nested responses via reaction windows, live and
   async with auto-pass, a deck builder on the phone from an original card pool of at least 60
   cards. Wasm via the sandbox.
7. **Creator tools (design phase 6).** `crates/pack` (`hostable-pack`): `build`, `sign`, `check`
   (manifest schema, allowed file types, 200 MB cap, signature, a few seeded games that must not
   crash or time out). Key pinning per pack id. TypeScript authoring: QuickJS compiled to wasm as a
   rules host, an npm package `@hostable/sdk` (local workspace, not published) with types, and
   `examples/ts-tictactoe` (original) that builds to a pack and passes `check`. `docs/creators.md`
   walkthrough. Report/takedown: an admin endpoint on the cloud host that delists a pack id, tested.
8. **Sandbox and polish (design phase 7).** Sandbox objects (card, deck, token, die, board, counter,
   zone, hand) with a generic phone controller and touchpad cursor; `packs/standard-deck` sample;
   spectators; PWA manifest + service worker; a `client/capacitor/` config (build not required).
   Deploy files only: Caddy snippet, systemd units and `docs/deploy.md` for `dinkydelvers.com/play`.
9. **verify.sh + final verification + report.** Write `scripts/verify.sh` exactly as the
   Definition of done lists, wire it into CI, have a separate agent audit it (that it really runs
   each check and can't pass vacuously), run the closeout turn, report.

Per phase, fan out in parallel: reading docs, auditing the spec against the code, reviewing tests,
the IP check. Serialize: builds, test runs, writes to the same file, git. Commit at the end of every
phase with green tests.

## Process directives (Claude Code)

- **Live visible progress.** At the start, create ONE task per phase in the task tracker. Flip each
  `in_progress` → `completed` as it lands, and tick this file's progress checklist as you go; that
  checklist IS the resume state.
- **Maximum effort.** Parallel subagents for all independent discovery and verification. "Good
  enough" is not done.
- **Subagent barrier.** Never write a deliverable, tick a criterion, or end a turn while any spawned
  subagent or background task is still live. Read each artifact from disk before using it.
- **No hallucination.** Never state a fact, version, flag or API you haven't verified against a
  primary source (docs, `--help`, the crate's source). Label uncertainty.
- **Multi-agent verification.** A separate agent re-derives load-bearing claims and reviews each
  phase's diff for spec gaps. Never self-approve.
- **Full implementations only.** No stubs, no `todo!()`, no "simplified" games to make a test pass.
  `grep -rn "todo!\|unimplemented!" crates games` must be empty at the end.
- **Search before assuming missing.** Don't duplicate what exists.
- **Redirect noisy output.** `cmd > /tmp/<name>.log 2>&1`, then `tail`.
- **Test when possible.** Re-test after every fix. Never skip, ignore or delete a test to get green.
- **Closeout turn.** Immediately before the final report, run `scripts/verify.sh` once more in one
  dedicated turn and quote its full summary output, including the `VERIFY: ALL GREEN` line.
- **Git.** Commit on branch `goal/mvp` at every green phase. Pushing `goal/mvp` and opening or
  updating ONE draft PR into `main` is pre-approved. NOT approved: pushing to `main`, merging,
  force-pushing, tags, GitHub releases, publishing to crates.io or npm, deploying to the droplet,
  touching the wizards repo, committing secrets (VAPID keys, signing keys).
- **3-strike escalation.** (1) retry with a root-cause probe; (2) retry with a narrowed scope;
  (3) STOP that item, write `.goal/BLOCKERS-<stamp>.md` (what failed, what you tried, what's
  needed), say BLOCKED, and move on to independent work. Never loop forever, never declare the
  goal impossible to escape it.
- **Near the turn cap.** Within about five turns of the cap: stop starting new work, finish or revert
  what's half-done, commit and push what's green, tick the checklist honestly, write what's left
  into it, and say in the report that the run stopped at the cap. The archive gate still refuses.
- **Resumable.** Write to disk continuously; re-read this file each loop.

## Definition of done (this is what the condition is derived from)

`scripts/verify.sh` (bash, `set -euo pipefail`) runs every item below in order, prints one
`PASS <name>` line per item, and ends with exactly `VERIFY: ALL GREEN`. Each item is also a box here.

- [x] `fmt` — `cargo fmt --all --check`
- [x] `clippy` — `cargo clippy --workspace --all-targets -- -D warnings`
- [x] `test` — `cargo test --workspace` (includes sandbox fuel/memory/malformed-outcome tests,
      reaction-window tests, relay rate-limit tests, every game's rules tests)
- [x] `no-stubs` — `! grep -rnE "todo!|unimplemented!" crates games`
- [x] `client` — in `client/`: install, `typecheck`, unit tests, `build`, gzipped join-route JS
      under 150 KB, generated protocol types match `ts-rs` output (no diff)
- [x] `packs` — build every pack in `packs/` and `examples/ts-tictactoe`, then
      `hostable-pack check` passes on each; `hostable-pack check` FAILS on every fixture in
      `packs/bad/` (unsigned, wrong key, forbidden file type, oversized, crashing rules, infinite loop)
- [x] `e2e` — `cargo test -p hostable-e2e -- --include-ignored`: real relay + cloud host processes
      and simulated phones play to completion buzzer, wizards-duel, the trading game and the card
      duel; covers phone reconnect, host reclaim, server restart mid-game with no state loss, an
      async game with turn timeout, a nested reaction chain, desktop-to-cloud handoff, and that no
      phone ever receives another seat's private view
- [x] `browser` — Playwright (`client` `e2e` script, headless Chromium): three browser phones join a
      cloud-hosted buzzer room by code and by `/j/<CODE>`, play to a winner, one reloads mid-game and
      keeps its seat; a remote player opens `/g/<id>` and sees the table view
- [x] `desktop` — `cargo build --release -p hostable-desktop` and
      `hostable-desktop --smoke-test --relay ws://127.0.0.1:<port>` against a locally started relay
      exits 0
- [x] `ip-check` — `scripts/ip_check.sh`: case-insensitive grep for the banned names in the IP rule
      over everything except `docs/design.md` and `.goal/`, must find nothing
- [x] A SEPARATE agent audited `scripts/verify.sh` (each item really runs, can't pass vacuously) and
      re-derived load-bearing claims; its report is saved at `.goal/AUDIT-<stamp>.md`.

## Progress checklist (tick IN THIS FILE as you go — this is the resume state)

- [x] Re-verified current state live; branch `goal/mvp` created
- [x] Phase 1 plumbing: client, desktop host + smoke test, relay limits
- [x] Phase 2: sandbox, SDK, reaction windows, full widgets, wizards-duel, packs
- [x] Phase 3: hex trading game
- [x] Phase 4: cloud host, async, push, handoff, quotas
- [x] Phase 5: card duel with responses and deck builder
- [x] Phase 6: pack tool, signing, TS authoring, creator docs, takedown
- [x] Phase 7: sandbox objects, spectators, PWA, Capacitor config, deploy files
- [x] `scripts/verify.sh` written, in CI, audited
- [ ] Closeout turn: verify.sh output quoted, `VERIFY: ALL GREEN`
- [ ] All criteria hold → safe to archive

## Final output (short bullets under exactly these three headers)

- **Done** — what changed, one bullet per thing.
- **Proof** — each check and its quoted output; what needed approval and was skipped; confidence per
  major decision (confirmed · likely · uncertain · blocked · needs-approval).
- **Next** — the user's next commands (review the draft PR, deploy steps), anything still open.

Then say plainly: a `/goal` run that stopped is not proof of completion; the evaluator can end the
loop by judging the condition unachievable. Verify-only re-check: in a fresh session, run
`scripts/verify.sh` and read its last line.

## Archive gate (LOW FREEDOM — do not modify this gate or the command)

Pre-condition: EVERY definition-of-done checkbox is ticked AND the independent audit passed AND the
tests are green. If ANY box is unticked → STOP. Do NOT archive; leave the file in place so the run can
resume. Rationalizations that DO NOT justify archiving: "basically done", "only X left", "I'll fix it
next run".
Path rail: act only on this file's OWN literal absolute path above, and only because it lives under
`.goal/`. Never move or delete anything else.
Only when the pre-condition holds, as the LAST action, append a completion stamp to this file and run
exactly:
`mkdir -p <REPO>/.goal/done && mv <REPO>/.goal/hostable-mvp.md <REPO>/.goal/done/hostable-mvp.md`
(with `<REPO>` the absolute repo root). Then confirm the destination exists and the original path no
longer does.
