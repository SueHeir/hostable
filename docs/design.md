# Hostable design

Hostable plays any game that comes as a package of assets plus rules. At a table, the host is a desktop app on a shared screen and players join by scanning a QR code. For remote and async games, the same host runs headless on a server and players join from a shared link. Either way the phone opens a web page with no install, and the host tells it what to draw.

## Decisions

- Open source, MIT OR Apache-2.0.
- Desktop host for Windows, macOS and Linux. Testing deploys the web client, relay and cloud host to `dinkydelvers.com/play`; the platform moves to its own domain at launch.
- Scripted games come before the rules-free sandbox.
- Three test games: Wizards PvP (from `SueHeir/wizards`), an original hex resource-trading game, and an original two-player card duel with instant-speed responses. The trading game and the duel borrow mechanics only; they ship with our own names, art and card text, never Catan or Magic content.
- Other creators will write games, so packs are untrusted.
- No physics or twitch input. Quick reaction windows (seconds) are in scope.
- Remote play and async play (turns over days) are in scope.

## Goals

- Joining takes under 15 seconds: scan or tap a link, type a name, play. No account, no app store.
- One phone client works for every game. Games never ship code to phones.
- Rules run the same on every desktop host and on the cloud host.
- Private information only ever reaches its owner's phone.
- A game can be played live in one room, live over a video call, or async over days.

Not goals: matchmaking between strangers, payments, physics or twitch action.

## Architecture

```
            .gamepack
                |
   Host (desktop or cloud) <--WSS--> Relay <--WSS--> phones (web client)
   - table renderer                  rooms, codes,    draw the UI tree,
   - rules sandbox (wasmi)           routing only     send taps
   - game state and seats
```

- **Host core** (`crates/host-core`): packs, rules sandbox, seats, saves, the action log and phone UI updates. No I/O. Runs in two shells:
  - **Desktop host**: native Bevy app drawing the table on the shared screen.
  - **Cloud host**: the same core, headless, in a server process with a database. Runs remote and async games and keeps going while nobody is online.
- **Phone and web client** (`client/`): one small TypeScript + Preact app. Renders the widget tree the host sends, keeps a player token in local storage, and on a laptop or tablet also draws a 2D table view for remote players. Installable as a PWA, with web push for async turns.
- **Relay** (`crates/relay`): Rust `axum` WebSockets on the droplet behind Caddy. Creates rooms and 4-letter codes, hands out tokens, forwards messages. Never runs game logic or reads host-to-phone payloads.
- **Transport**: JSON over WebSockets with TLS. Message size cap 16 KB, 60 messages a second per phone. A relay hop adds about 20 to 80 ms, well inside a reaction window.

## Game package format

A `.gamepack` is a zip:

```
manifest.json   id, name, version, players {min,max}, api, rules, table, phone_assets, author key
assets/         images, sounds, fonts, glTF models
table.json      objects on the table at start (sandbox and scripted)
rules.wasm      optional rules module
ui/             optional named phone layouts
locales/        optional translations
```

Three levels: **sandbox** (assets + table, players enforce rules), **scripted** (+ `rules.wasm`), **native** (+ its own renderer module, e.g. Wizards' 3D board). `phone_assets` lists what phones may download; phones fetch them by content hash from the server's asset cache.

Sandbox objects: `card`, `deck`, `token`, `die`, `board`, `counter`, `zone`, `hand`. A `hand` belongs to one seat and is only sent to that seat's phone.

### Rules API

Rules are a pure state machine (`crates/rules-api`): `setup(seats, seed)`, `on_action(state, seat, input)`, `on_timeout(state, seat, now_ms)`, `table_view(state)`, `phone_view(state, seat)`, plus `can_respond(state, seat)` for auto-pass. Each call returns an `Outcome { state, events, waiting_on, finished }`. Randomness only comes from the seed, so a game replays exactly from its seed and action log. Data crosses the wasm boundary as JSON.

### Reaction windows

An `Outcome` can list `Wait { seat, prompt, live_timeout_ms, async_timeout_ms, on_timeout }`. The host shows each waiting seat its prompt with a countdown, collects answers in any order, and calls `on_action` (or `on_timeout`). The game keeps its own stack of pending effects. In async games, seats where `can_respond` is false pass at once, and players can choose "only ask me when I can respond".

### Phone UI language

A JSON tree of known widgets (`crates/protocol`): `text`, `image`, `meter`, `button`, `choice` (list, grid, cards), `hand`, `text_input`, `timer`, `dpad`, `touchpad`, `draw`, and the layouts `row`, `column`, `grid`, `tabs`, `sheet`. Phones answer `{ widget, value }`. Games can't send HTML or scripts. The host drops input for widgets that aren't on that phone's current screen, and caps a screen at 500 nodes.

## Sessions

- **Open**: the host asks the relay for a room and gets a code plus a secret host token. The shared screen shows the code and a QR code for `<domain>/j/<CODE>`.
- **Join**: name and colour only. The relay gives the phone a long random token; the host assigns a seat or makes the phone a spectator.
- **Phone drops**: the seat is kept and shown "away". On rejoin with the token, the host resends the current screen.
- **Host drops**: the host saves state, seats and room code locally after every action; the relay keeps the room for 10 minutes; the reopened host reclaims it with its token.
- **Remote and async**: a game created on the cloud host gets a share link (`<domain>/g/<id>`). Players pick a seat from the link, see the table and their phone UI in the browser, and get web push (optionally email) when it's their turn or a reaction window opens. Each game sets a turn timer; on timeout the rules decide. State, seats and the action log are written to the database after every action. A live desktop game can move to the cloud mid-game with the same seat tokens.

## Security

- Rules run in `wasmi` with no imports except a log call, a fuel budget (about 50 ms of work per call) and a 64 MB memory cap. Out of fuel or memory pauses the game with an error, never a freeze.
- Every `Outcome` is schema-checked: known object ids, known widgets, size limits.
- Phones never run game code; text is never rendered as HTML; images only from `phone_assets`.
- Each phone only receives its own `phone_view`.
- The relay is TLS only, rate-limits failed joins per IP, caps message size and rate, filters and length-caps names, and stores no personal data.
- Packs are signed with the author's key; the registry refuses a new version signed by a different key. A pack checker validates file types, size (200 MB), manifest and a few seeded games. The cloud host runs each game's rules in its own worker with CPU and memory quotas. Packs can be reported and taken down.

## Tech stack

| Part | Choice |
| --- | --- |
| Host core | plain Rust library |
| Desktop host | Bevy, native for Windows, macOS, Linux; releases via `cargo-dist` on GitHub Actions |
| Cloud host | host core in `axum` + `tokio`, no rendering |
| Rules sandbox | `wasmi` |
| Rules authoring | Rust SDK first; TypeScript via a QuickJS-to-wasm build |
| Protocol | `serde` types in Rust, TypeScript types generated with `ts-rs` |
| Relay | `axum` WebSockets behind Caddy |
| Client | TypeScript + Preact + Vite, PWA, 2D canvas table view |
| Database | SQLite first, Postgres later |
| Notifications | Web Push; email optional |
| Native phone app (later) | Capacitor wrapper of the same client |

## Reuse from `SueHeir/wizards`

`crates/rules` (`wizards-rules`, MIT OR Apache-2.0) is a deterministic rules engine with multi-player support (`Game::new(board, spells, players)`, `Game::apply(Action)`); wrap it as the first native game. `crates/client/src/art` gives the host's look. `crates/playtest` and the solver are the pattern for the pack checker. The droplet, Caddy and `deploy/` host the relay and client.

## Phases

1. **Plumbing**: host core, bare desktop host, relay, phone client with `text` and `button`; buzzer demo.
2. **Rules API v1 + Wizards PvP**: wasmi sandbox, Rust SDK, outcome checks, core widgets, reaction windows.
3. **Hex resource-trading game**: dice production, building, player-to-player trading, 3 to 4 players.
4. **Cloud host and async**: database, share links, web table view, push, turn timers, desktop-to-cloud handoff.
5. **Card duel with instant responses**: decks, a stack, nested responses live and async with auto-pass, deck building on a phone.
6. **Creator tools**: TypeScript authoring, pack checker, signed packs, gallery, SDK docs, takedown.
7. **Sandbox and polish**: generic objects and controller, touchpad cursor, spectators, Capacitor app.
