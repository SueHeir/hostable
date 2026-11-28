# Making a game for Hostable

A Hostable game is a `.gamepack`: a zip with a `manifest.json`, the game's
rules, and any images or sounds. Phones never run your code. Your rules run
on the host inside a sandbox and describe each phone's screen as a tree of
widgets; the host sends each player only their own screen.

This walkthrough builds a pack, checks it, and plays it.

## 1. Pick a level

| Level | What you write | Who enforces the rules |
| --- | --- | --- |
| **sandbox** | `table.json`: cards, decks, dice, tokens, counters, zones | the players, like at a real table |
| **scripted** | rules in Rust or TypeScript | your rules |
| **native** | scripted, plus a renderer built into the host | your rules |

Sandbox is the quickest: see `packs/standard-deck/` for a 54-card table
with dice and score counters. Every phone gets the same controller: a
touchpad that moves the player's cursor on the shared screen, a tap that
picks up what's under it, buttons for what can be done with it, and the
player's own private hand.

## 2. Write rules

Rules are a pure state machine. The host calls:

| Call | Returns |
| --- | --- |
| `setup(seats, seed)` | the first state (and maybe a turn timer) |
| `on_action(state, seat, input)` | the next state |
| `on_timeout(state, seat, now_ms)` | the next state when a wait runs out |
| `table_view(state)` | what the shared screen shows (public only) |
| `phone_view(state, seat)` | one seat's screen |
| `can_respond(state, seat)` | whether the seat could answer the current reaction window |

Everything except the two views returns an *outcome*: the new state, plus
optional `events` (sounds, toasts, phone buzzes), `waiting_on` (seats with a
deadline) and `finished` (the ranking). Randomness comes only from the seed:
keep a seeded `Rng` in your state, so a game replays exactly from its seed
and its action log.

### In TypeScript

`examples/ts-tictactoe` is a complete game in about 120 lines:

```ts
import { defineGame, ui } from "@hostable/sdk";

defineGame<State>({
  seats: { min: 2, max: 2 },
  setup(seats) { return { state: { /* ... */ }, waiting_on: [/* first turn */] }; },
  onAction(state, seat, input) { /* ... */ },
  tableView(state) { return { title: "...", status: "...", scene: { /* shapes */ }, players: [], log: [] }; },
  phoneView(state, seat) { return ui.column(ui.title("Your move"), ui.button("go", "Go", "primary")); },
});
```

The manifest's `"build": {"npm": "."}` tells `hostable-pack build` to run
`npm run build` (esbuild bundles `src/rules.ts` into `dist/rules.js`). The
pack runs it in QuickJS compiled to WebAssembly: no network, no files,
`Date.now()` is always 0, and each call gets about 50 ms.

### In Rust

Implement `hostable_sdk::Rules` and call `hostable_sdk::export!(MyGame)`
(see `games/buzzer/src/lib.rs`). Make the crate `crate-type = ["cdylib",
"rlib"]`; the manifest's `"build": {"rust": "my-crate"}` builds it for
`wasm32-unknown-unknown`. The same crate runs natively in your tests, for
example driving `hostable_host_core::Room` with bots.

### Phone screens

Screens are trees of these widgets (`crates/protocol/src/widget.rs`):
layouts `column`, `row`, `grid`, `tabs`, `sheet`; display `text`, `image`,
`meter`, `timer`; input `button`, `choice` (list, grid, cards; one pick or
several), `hand`, `text_input`, `dpad`, `touchpad`, `draw`. The host drops
any input for a widget that isn't on that phone's current screen, or with a
value that screen couldn't send, so your rules only ever see inputs a
player could really make. A screen may have at most 500 nodes. Text is
always shown as text, never HTML; images only come from the files you list
in `phone_assets`.

### Reaction windows

To let players answer something ("respond to the fireball?"), return a
`Wait` for each seat that may respond, with a prompt, a live timeout (a few
seconds) and an async timeout (hours), and an `on_timeout` input such as
your pass button. The host shows each waiting seat the prompt and a
countdown and collects answers in any order. In async games, and for
players who ask for it, seats where `can_respond` is false pass at once.
Keep your own stack of pending effects in the state.

## 3. Build, sign, check

```sh
cargo run -p hostable-pack -- keygen --out ~/.hostable/pack.key   # once; keep it secret
cargo run -p hostable-pack -- build examples/ts-tictactoe --out tictactoe.gamepack --key ~/.hostable/pack.key
cargo run -p hostable-pack -- check tictactoe.gamepack
```

`check` is what a host or registry runs before accepting a pack. It fails
with a code if:

| Code | Why |
| --- | --- |
| `unsigned`, `bad-signature` | no signature, or files changed after signing |
| `key-mismatch` | signed by a different key than the one this pack id is pinned to |
| `forbidden-file` | a file type that isn't allowed (only images, sounds, fonts, models, JSON and the rules) |
| `too-large` | over 200 MB unpacked |
| `bad-manifest` | a missing or invalid field, or seat counts that don't match the rules |
| `rules-crashed`, `rules-timeout`, `rules-memory`, `rules-malformed` | a seeded bot game crashed, ran out of time (fuel), hit the 64 MB memory cap, or returned something invalid |
| `rules-stuck` | nobody could do anything and nothing was pending |

Every new version of a pack must be signed with the same key as the first,
so keep the key safe: nobody can publish an update to your pack without it.

## 4. Play it

```sh
cargo run -p hostable-relay &
cargo run -p hostable-desktop -- --relay ws://127.0.0.1:8787 --pack tictactoe.gamepack --join-url http://localhost:5173
```

The desktop shows a room code and QR code; open the web client
(`cd client && npm run dev`) on phones and join. For remote or async play,
upload the pack to a cloud host's packs directory and create a game with
`POST /api/games` (see `docs/api.md`); share the `/g/<id>` link.

## Rules of the road

- Your game must be your own. Borrowing a mechanic is fine; copying another
  game's name, art, card names or rules text isn't.
- Packs can be reported; an admin can take a pack down, which pauses its
  games.
