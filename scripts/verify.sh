#!/usr/bin/env bash
# Hostable's definition of done. Runs every check in order, prints one
# `PASS <name>` line per check, and ends with `VERIFY: ALL GREEN`. Any
# failure stops the script with a non-zero exit and the failing log tail.
#
#   scripts/verify.sh            # everything
#   VERIFY_ONLY="fmt clippy" scripts/verify.sh   # a subset, for quick loops
#
# Needs: rustup's toolchain with the wasm32-unknown-unknown target, Node 20+
# with npm, and network access the first time (crates, npm packages,
# Playwright's Chromium).
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

# Prefer rustup's cargo: a system cargo (e.g. Homebrew's) has no wasm target.
if [ -x "$HOME/.cargo/bin/cargo" ]; then
  export PATH="$HOME/.cargo/bin:$PATH"
fi

logs="$root/target/verify"
mkdir -p "$logs"
only="${VERIFY_ONLY:-}"
ran=0

want() {
  [ -z "$only" ] && return 0
  case " $only " in *" $1 "*) return 0 ;; esac
  return 1
}

# run NAME CMD...: run a check with its output in target/verify/NAME.log.
run() {
  local name="$1"
  shift
  want "$name" || return 0
  local log="$logs/$name.log"
  echo "---- $name"
  if "$@" >"$log" 2>&1; then
    echo "PASS $name"
    ran=$((ran + 1))
  else
    echo "FAIL $name (log: $log)"
    tail -n 60 "$log"
    exit 1
  fi
}

# ---------------------------------------------------------------------------

check_fmt() { cargo fmt --all --check; }

check_clippy() { cargo clippy --workspace --all-targets -- -D warnings; }

check_test() {
  cargo test --workspace 2>&1 | tee "$logs/test-output.txt"
  # The tests the definition of done names must have run.
  for t in \
    an_infinite_loop_runs_out_of_fuel_and_the_host_carries_on \
    a_memory_bomb_hits_the_cap \
    malformed_outcomes_are_rejected \
    a_reaction_window_times_out_into_its_pass_input \
    nested_responses_resolve_as_one_chain \
    async_games_pass_at_once_for_seats_that_cannot_respond \
    a_phone_is_limited_to_sixty_messages_a_second \
    too_many_wrong_codes_from_one_ip_are_refused \
    oversized_phone_messages_close_the_socket; do
    grep -q "$t ... ok" "$logs/test-output.txt" || { echo "missing test: $t"; return 1; }
  done
  ! grep -q "test result: FAILED" "$logs/test-output.txt"
}

check_no_stubs() {
  if grep -rnE "todo!|unimplemented!" crates games; then
    echo "stubs found"
    return 1
  fi
  echo "no todo!/unimplemented! in crates or games"
  # Only the end-to-end suite may be #[ignore]d (it runs with --include-ignored).
  if grep -rn "#\[ignore" crates games | grep -v "^crates/e2e/"; then
    echo "ignored tests outside crates/e2e"
    return 1
  fi
}

check_client() {
  cd "$root/client"
  npm ci --no-audit --no-fund
  npm run typecheck
  npm test
  npm run build
  npm run size
  # The committed TypeScript protocol types must match what ts-rs generates.
  local gen
  gen="$(mktemp -d)"
  (cd "$root" && TS_RS_EXPORT_DIR="$gen" cargo test -p hostable-protocol export_bindings -- --quiet)
  diff -r "$gen" "$root/client/src/protocol"
  rm -rf "$gen"
  echo "protocol types match ts-rs output"
}

check_packs() { "$root/scripts/packs.sh"; }

check_e2e() {
  cargo test -p hostable-e2e -- --include-ignored --test-threads 2 2>&1 | tee "$logs/e2e-output.txt"
  for t in \
    buzzer_plays_to_a_winner_with_a_reconnect \
    wizards_duel_plays_to_the_end \
    tideholm_plays_to_the_end \
    riposte_plays_to_the_end_with_a_nested_chain \
    host_reclaims_its_room_after_a_drop \
    cloud_restart_mid_game_loses_nothing \
    async_game_turn_timeout \
    desktop_to_cloud_handoff_keeps_seats \
    no_phone_ever_sees_another_seats_private_view; do
    grep -q "$t ... ok" "$logs/e2e-output.txt" || { echo "missing e2e test: $t"; return 1; }
  done
  ! grep -q "test result: FAILED" "$logs/e2e-output.txt"
}

check_browser() {
  cd "$root/client"
  npx playwright install chromium
  npm run e2e
}

check_desktop() {
  cargo build --release -p hostable-desktop -p hostable-relay
  local port_file="$logs/relay-port.txt"
  "$root/target/release/hostable-relay" --addr 127.0.0.1:0 >"$port_file" 2>&1 &
  local relay_pid=$!
  local addr=""
  for _ in $(seq 1 100); do
    addr="$(sed -n 's/.*listening on //p' "$port_file" | head -1)"
    [ -n "$addr" ] && break
    sleep 0.1
  done
  if [ -z "$addr" ]; then
    echo "relay didn't start"; cat "$port_file"; kill "$relay_pid" 2>/dev/null || true; return 1
  fi
  local code=0
  "$root/target/release/hostable-desktop" --smoke-test --relay "ws://$addr" --players 3 || code=$?
  kill "$relay_pid" 2>/dev/null || true
  wait "$relay_pid" 2>/dev/null || true
  return "$code"
}

check_ip() { "$root/scripts/ip_check.sh"; }

# ---------------------------------------------------------------------------

run fmt check_fmt
run clippy check_clippy
run test check_test
run no-stubs check_no_stubs
run client check_client
run packs check_packs
run e2e check_e2e
run browser check_browser
run desktop check_desktop
run ip-check check_ip

if [ -z "$only" ]; then
  [ "$ran" -eq 10 ] || { echo "only $ran of 10 checks ran"; exit 1; }
  echo "VERIFY: ALL GREEN"
else
  echo "VERIFY: SUBSET OK ($only)"
fi
