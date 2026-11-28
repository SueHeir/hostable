# Hostable

An open-source host for any game: board games, card games, party games. One screen shows the table, and everyone plays from their phone by scanning a QR code. No app install, no account.

Games are packages of assets plus sandboxed rules, so anyone can make one. The same host runs as a desktop app (Windows, macOS, Linux) for game night, or headless on a server for remote and async play.

Status: MVP. Everything below builds and is tested end to end; nothing is deployed yet.

## What's here

| Path | What |
| --- | --- |
| `crates/protocol` | wire messages and the phone widget language (TypeScript types generated with ts-rs into `client/src/protocol`) |
| `crates/rules-api`, `crates/sdk` | the `Rules` trait, the wasm JSON ABI, and the Rust SDK (`export!`) |
| `crates/host-core` | seats, reaction windows, timers, auto-pass, spectators, saves, outcome checks |
| `crates/sandbox` | `wasmi` with fuel and a 64 MB cap; also runs JavaScript rules in QuickJS compiled to wasm |
| `crates/relay` | rooms, codes, tokens, rate limits (`hostable-relay`) |
| `crates/host-runtime` | the host loop shared by desktop and cloud: relay link, reconnect and reclaim, saves, CPU quotas |
| `crates/cloud` | the cloud host (`hostable-cloud`): HTTP API, SQLite, share links, Web Push, handoff, takedown |
| `crates/desktop` | the Bevy desktop host (`hostable-desktop`) |
| `crates/pack` | `hostable-pack build / sign / check`, key pinning |
| `crates/tabletop` | sandbox-level packs: cards, decks, dice, tokens, counters, zones, hands |
| `client/` | the phone and web client (Preact + Vite), PWA |
| `games/` | buzzer, Wizards duel, Tideholm (hex trading), Riposte (card duel) |
| `packs/`, `examples/ts-tictactoe` | pack sources; `packs/bad/` holds packs the checker must refuse |
| `sdk/typescript` | `@hostable/sdk` for TypeScript rules |

Docs: [design](docs/design.md), [HTTP and socket API](docs/api.md), [making a game](docs/creators.md), [deploying](docs/deploy.md).

## Try it

```sh
cargo run -p hostable-relay                       # ws://127.0.0.1:8787
cargo run -p hostable-desktop -- --relay ws://127.0.0.1:8787 --join-url http://localhost:5173
cd client && npm install && npm run dev           # phones open http://<your-ip>:5173
```

## Check everything

```sh
scripts/verify.sh     # fmt, clippy, tests, client, packs, e2e, browser, desktop, IP check
```

It ends with `VERIFY: ALL GREEN`. It needs rustup's toolchain with the `wasm32-unknown-unknown` target and Node 22. The Wizards duel depends on the private `SueHeir/wizards` repo, so cargo needs git access to it.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option. Unless you explicitly state otherwise, any contribution you submit for inclusion in this project shall be dual licensed as above, without any additional terms or conditions.
