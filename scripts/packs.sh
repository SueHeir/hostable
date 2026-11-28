#!/usr/bin/env bash
# Build every pack in packs/ and examples/ts-tictactoe, check each one, and
# check that every fixture in packs/bad/ is refused for the reason it names.
#
# Output: target/packs/<id>.gamepack for the good packs (the cloud host and
# end-to-end tests load these). Signing keys are generated under
# target/verify/keys and never committed.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
if [ -x "$HOME/.cargo/bin/cargo" ]; then export PATH="$HOME/.cargo/bin:$PATH"; fi

cargo build --release -q -p hostable-pack
pack="$root/target/release/hostable-pack"
out="$root/target/packs"
keys="$root/target/verify/keys"
work="$root/target/verify/packs-work"
mkdir -p "$out" "$keys"
rm -rf "$work" && mkdir -p "$work"
rm -f "$out"/*.gamepack

for k in dev other; do
  if [ ! -f "$keys/$k.key" ]; then "$pack" keygen --out "$keys/$k.key" > "$keys/$k.pub"; fi
done
dev_pub="$(cat "$keys/dev.pub")"

# ---- good packs ----
(cd examples/ts-tictactoe && npm ci --no-audit --no-fund --silent)
good=(packs/buzzer packs/wizards-duel packs/tideholm packs/riposte packs/standard-deck examples/ts-tictactoe)
pins="$work/pins.json"
echo '{"pins":{}}' > "$pins"
for dir in "${good[@]}"; do
  id="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["id"])' "$dir/manifest.json")"
  file="$out/$id.gamepack"
  "$pack" build "$dir" --out "$file" --key "$keys/dev.key"
  "$pack" check "$file" --pins "$pins" --pin --seeds 3
  # A second check against the pin must still pass (same key).
  "$pack" check "$file" --pins "$pins" --seeds 1 > /dev/null
done
echo "good packs: ${#good[@]} built, signed and checked"

# ---- packs that must be refused ----
bad=0
for dir in packs/bad/*/; do
  name="$(basename "$dir")"
  fixture="$dir/fixture.json"
  expect="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["expect"])' "$fixture")"
  sign="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["sign"])' "$fixture")"
  pinned="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1])).get("pinned_to",""))' "$fixture")"
  pad="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1])).get("pad_mb",0))' "$fixture")"
  src="$work/src-$name"
  cp -R "$dir" "$src"
  if [ "$pad" -gt 0 ]; then
    mkdir -p "$src/assets"
    head -c "$((pad * 1024 * 1024))" /dev/zero > "$src/assets/big.bin"
  fi
  file="$work/$name.gamepack"
  case "$sign" in
    none) "$pack" build "$src" --out "$file" ;;
    dev | other) "$pack" build "$src" --out "$file" --key "$keys/$sign.key" ;;
    *) echo "bad fixture $name: unknown sign $sign"; exit 1 ;;
  esac
  args=()
  if [ -n "$pinned" ]; then
    id="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["id"])' "$dir/manifest.json")"
    printf '{"pins":{"%s":"%s"}}\n' "$id" "$(cat "$keys/$pinned.pub")" > "$work/pins-$name.json"
    args=(--pins "$work/pins-$name.json")
  fi
  set +e
  result="$("$pack" check "$file" "${args[@]+"${args[@]}"}" 2>&1)"
  code=$?
  set -e
  echo "$name: $result"
  if [ "$code" -ne 1 ]; then echo "FAIL: $name should have been refused (exit $code)"; exit 1; fi
  if ! grep -q "check: FAIL $expect:" <<<"$result"; then
    echo "FAIL: $name was refused, but not with $expect"; exit 1
  fi
  rm -rf "$src"
  bad=$((bad + 1))
done
[ "$bad" -ge 6 ] || { echo "expected at least 6 bad fixtures, found $bad"; exit 1; }
echo "bad packs: $bad refused for the expected reasons"
echo "dev key: $dev_pub"
