# Deploying the test site to dinkydelvers.com/play

Nothing here runs automatically. These are the files and steps to put the
relay, the cloud host and the web client on the existing DigitalOcean
droplet behind Caddy, next to `/wizards`.

| Piece | Where on the droplet | Listens on |
| --- | --- | --- |
| `hostable-relay` | `/opt/hostable/bin/` | `127.0.0.1:8787` |
| `hostable-cloud` | `/opt/hostable/bin/` | `127.0.0.1:8790` |
| web client (`client/dist`) | `/opt/hostable/client/` | served by the cloud host |
| packs (`target/packs/*.gamepack`) | `/opt/hostable/packs/` | loaded at start |
| state (relay rooms, SQLite) | `/var/lib/hostable/` | — |
| secrets | `/etc/hostable/cloud.env` (mode 600) | — |

Caddy terminates TLS and routes `/play/ws/*` to the relay and everything
else under `/play/` to the cloud host (`deploy/Caddyfile.snippet`).

## Build

On a Linux x86_64 machine (or in CI):

```sh
cargo build --release -p hostable-relay -p hostable-cloud
scripts/packs.sh                                  # target/packs/*.gamepack
(cd client && npm ci && HOSTABLE_BASE=/play/ npm run build)
```

Sign packs with a key you keep off the server: `scripts/packs.sh` uses a
throwaway key in `target/verify/keys`; for the real site run
`hostable-pack keygen --out ~/.hostable/pack.key` once and build with
`hostable-pack build packs/<name> --out <name>.gamepack --key ~/.hostable/pack.key`.

## One-time setup on the droplet

```sh
sudo useradd --system --home /var/lib/hostable --shell /usr/sbin/nologin hostable
sudo mkdir -p /opt/hostable/bin /opt/hostable/client /opt/hostable/packs /etc/hostable
sudo cp deploy/hostable-relay.service deploy/hostable-cloud.service /etc/systemd/system/
sudo cp deploy/cloud.env.example /etc/hostable/cloud.env
sudo chmod 600 /etc/hostable/cloud.env && sudoedit /etc/hostable/cloud.env
```

Make the VAPID key for Web Push (any machine with Node):

```sh
node -e 'const c=require("crypto");const k=c.generateKeyPairSync("ec",{namedCurve:"P-256"});console.log(k.privateKey.export({format:"jwk"}).d)'
```

Put the printed value in `HOSTABLE_VAPID_PRIVATE_KEY`, and a long random
string in `HOSTABLE_ADMIN_TOKEN`. Never commit either.

Add the snippet from `deploy/Caddyfile.snippet` inside the
`dinkydelvers.com { ... }` block of `/etc/caddy/Caddyfile`, then
`sudo caddy validate --config /etc/caddy/Caddyfile && sudo systemctl reload caddy`.

## Each deploy

```sh
rsync -av target/release/hostable-relay target/release/hostable-cloud dinky:/tmp/
rsync -av --delete client/dist/ dinky:/tmp/hostable-client/
rsync -av target/packs/ dinky:/tmp/hostable-packs/
ssh dinky 'sudo install -m 755 /tmp/hostable-relay /tmp/hostable-cloud /opt/hostable/bin/ \
  && sudo rsync -a --delete /tmp/hostable-client/ /opt/hostable/client/ \
  && sudo rsync -a /tmp/hostable-packs/ /opt/hostable/packs/ \
  && sudo systemctl daemon-reload \
  && sudo systemctl enable --now hostable-relay hostable-cloud \
  && sudo systemctl restart hostable-relay hostable-cloud'
```

Restarts are safe: the relay keeps rooms and tokens in its state file, and
the cloud host brings every game back from SQLite and reclaims its room, so
phones keep their seats.

## Check it

```sh
curl -fsS https://dinkydelvers.com/play/healthz           # cloud: ok
curl -fsS https://dinkydelvers.com/play/config.json       # relay URL and VAPID key
curl -fsS https://dinkydelvers.com/play/api/packs         # the loaded packs
```

Then open `https://dinkydelvers.com/play/` on a phone. To host from a
laptop: `hostable-desktop --relay wss://dinkydelvers.com/play --cloud https://dinkydelvers.com/play`.

## Taking a pack down

```sh
curl -X POST https://dinkydelvers.com/play/api/admin/delist \
  -H "Authorization: Bearer $HOSTABLE_ADMIN_TOKEN" -H 'content-type: application/json' \
  -d '{"pack":"some-pack","reason":"reported for abuse"}'
```

Its running games pause with the reason and it can't start new ones.
Reports from players are at `GET /play/api/admin/reports` with the same token.
