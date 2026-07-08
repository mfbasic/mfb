#!/usr/bin/env bash
# Regenerate golden files in-place: run the acceptance harness to produce
# "actual" outputs, then overwrite each EXISTING golden file with its freshly
# produced actual. New golden files are never created (so a test's golden set is
# unchanged in shape; only contents are refreshed). Intended for mass mechanical
# migrations (plan-01-functions.md §5) where many goldens change at once.
#
# Usage: scripts/sync-goldens.sh <mfb-exe> [test-name ...]
# With no test names, syncs every test under tests/.
set -u
ROOT=$(cd "$(dirname "$0")/.." && pwd)
MFB_EXE=${1:?usage: sync-goldens.sh <mfb-exe> [test ...]}
shift || true
ACTUAL=$(mktemp -d)
bash "$ROOT/scripts/test-accept.sh" "$MFB_EXE" "$ACTUAL" >/dev/null 2>&1 || true

tests=("$@")
if [ "${#tests[@]}" -eq 0 ]; then
  tests=()
  # Every project.json is a test at any depth, under tests/{syntax,rt-error,
  # rt-behavior}/<feature>/* (plus the tests/acceptance app).
  while IFS= read -r pj; do
    d=$(dirname "$pj")
    tests+=("${d#"$ROOT/tests/"}")
  done < <(find "$ROOT"/tests -name project.json | sort)
fi

count=0
for t in "${tests[@]}"; do
  gdir="$ROOT/tests/$t/golden"
  adir="$ACTUAL/$t"
  [ -d "$gdir" ] || continue
  [ -d "$adir" ] || continue
  for gf in "$gdir"/*; do
    [ -f "$gf" ] || continue
    name=$(basename "$gf")
    if [ -f "$adir/$name" ]; then
      cp "$adir/$name" "$gf"
      count=$((count + 1))
    fi
  done
done
echo "synced $count golden file(s) across ${#tests[@]} test(s)"
rm -rf "$ACTUAL"
