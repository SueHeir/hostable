# Packs the checker must refuse

Each directory is a pack source that `scripts/packs.sh` builds and then runs
`hostable-pack check` on, expecting it to FAIL with the code in its
`fixture.json` (`expect`):

| Fixture | Built how | Expected failure |
| --- | --- | --- |
| `unsigned` | not signed | `unsigned` |
| `wrong-key` | signed with a second key while the id is pinned to the first | `key-mismatch` |
| `forbidden-file` | contains `assets/page.html` | `forbidden-file` |
| `oversized` | a 201 MB asset is generated at build time (never committed) | `too-large` |
| `crashing-rules` | rules trap on every call after `info` | `rules-crashed` |
| `infinite-loop` | rules spin forever after `info` | `rules-timeout` |

Keys are generated at build time under `target/`; none are committed.
