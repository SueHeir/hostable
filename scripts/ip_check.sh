#!/usr/bin/env bash
# Fail if any file in the repo (tracked, or new and not ignored) mentions the
# names our original games must never use. docs/design.md (whose decisions
# line names them on purpose) and .goal/ are skipped.
#
# The names are assembled from pieces so this script doesn't match itself.
set -euo pipefail
cd "$(dirname "$0")/.."
words=(
  "Cat""an"
  "Settl""ers"
  "Mag""ic"
  "Gath""ering"
  "Wizards of the Co""ast"
  "Kos""mos"
)
pattern="$(IFS='|'; echo "${words[*]}")"
files="$(git ls-files -co --exclude-standard | grep -v -e '^docs/design\.md$' -e '^\.goal/' || true)"
[ -n "$files" ] || { echo "ip-check: no files to check"; exit 1; }
count="$(printf '%s\n' "$files" | wc -l | tr -d ' ')"
hits="$(printf '%s\n' "$files" | while IFS= read -r f; do
  [ -f "$f" ] && grep -aniE "$pattern" -- "$f" 2>/dev/null | sed "s|^|$f:|" | cut -c1-200
done || true)"
if [ -n "$hits" ]; then
  echo "ip-check: banned names found:"
  echo "$hits"
  exit 1
fi
echo "ip-check: $count files clean"
