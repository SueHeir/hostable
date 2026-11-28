# Hostable HTTP and socket API

Everything a phone, browser or host says, in one place. Message types are
defined in Rust in `crates/protocol` and generated into TypeScript in
`client/src/protocol` (never edit those by hand).

## Sockets (relay)

| Path | Who | First message |
| --- | --- | --- |
| `/ws/host` | a host (desktop or cloud) | `create_room` or `reclaim_room` |
| `/ws/phone` | a phone or browser | `join` (`code`, `name`, `color`) or `rejoin` (`code`, `player_token`) |
| `/healthz` | monitoring | — |

A phone keeps `{code, player, player_token}` in local storage after
`joined`, and sends `rejoin` with it whenever it reconnects. After that it
sends `{"type":"input","widget":...,"value":...}` and receives
`from_host` messages carrying `ui`, `table`, `feedback` and `seat`.

Limits: 16 KB per phone message, 60 messages a second per phone (extra ones
are dropped with an `error`), 10 wrong codes or tokens per IP per minute.

Host-only inputs a phone may send (handled by the host, never by the game):

| Widget | Value | Meaning |
| --- | --- | --- |
| `$table` | `true`/`false` | also send me the table view (laptops, remote players) |
| `$auto_pass` | `true`/`false` | only ask me when I can respond |
| `$push` | a `PushSubscription` JSON | send Web Push when it's my turn |

## Cloud host HTTP API

All JSON. Paths are relative to the cloud host's root (`/play/` in the test
deployment, where Caddy strips the prefix).

### `GET /config.json`

```json
{ "relay": "wss://dinkydelvers.com/play/ws/phone", "api": "/play/api", "vapid_public_key": "B..." }
```

The web client reads this at start. `relay` is the phone socket URL;
`vapid_public_key` is empty when push isn't set up.

### `GET /api/packs`

`[{ "id": "buzzer", "name": "Buzzer", "min_seats": 2, "max_seats": 12 }]`. Delisted packs are left out.

### `POST /api/games`

Request: `{ "pack": "buzzer", "seats": ["Ana", "Bo"], "pace": "async", "turn_timer_ms": 86400000 }`.
`seats` names the seats to create (2 to the game's maximum). `pace` is `live` or `async`.

Response: `{ "id": "g7Kx...", "code": "BCDF", "url": "/g/g7Kx..." }`.

### `GET /api/games/{id}`

```json
{
  "id": "g7Kx...", "pack": "buzzer", "name": "Buzzer", "code": "BCDF", "pace": "async",
  "started": true, "finished": null, "paused": null,
  "seats": [{ "seat": 0, "name": "Ana", "claimed": true, "connected": false }],
  "table": { "title": "Buzzer", "status": "...", "players": [], "log": [] }
}
```

`table` is the public table view (`null` before the game starts). Nothing
private is ever in it.

### `POST /api/games/{id}/claim`

Request: `{ "seat": 0 }`. Response: `{ "code": "BCDF", "player": 3, "player_token": "..." }`.
The browser stores it like a `joined` and connects with `rejoin`. A seat can
be claimed once; claiming again returns `409`.

### `POST /api/games/{id}/start`

Starts the game once enough seats exist. `409` if already started.

### `POST /api/handoff`

Moves a live desktop game to the cloud: `{ "pack": "...", "code": "BCDF", "host_token": "...", "snapshot": {...}, "pace": "async" }`.
The cloud reclaims the room on the relay with the host token, so every phone
keeps its seat and token. Response: `{ "id": "...", "url": "/g/..." }`.

### `POST /api/admin/delist`

`Authorization: Bearer <HOSTABLE_ADMIN_TOKEN>`. Request: `{ "pack": "id", "reason": "..." }`.
Delisted packs can't start new games and disappear from `/api/packs`;
running games are paused with the reason. `401` without the token.

### `POST /api/reports`

Anyone can report a pack: `{ "pack": "id", "reason": "..." }` → `202`.
Reports are listed at `GET /api/admin/reports` (admin token).

## Web client routes

| Route | What |
| --- | --- |
| `/` | join screen: code, name, colour |
| `/j/{CODE}` | join a room by link (QR code target); rejoins if a token is stored |
| `/g/{id}` | a cloud game: the table view, seats to claim, then the player's own screen |
