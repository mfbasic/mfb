#!/usr/bin/env bash
# Regenerate golden files in-place: run the acceptance harness to produce
# "actual" outputs, then overwrite each EXISTING golden file with its freshly
# produced actual. New golden files are never created (so a test's golden set is
# unchanged in shape; only contents are refreshed). Suits both mass mechanical
# migrations (no filter → every test) and refreshing a handful of fixtures.
#
# Usage: scripts/sync-goldens.sh <mfb-exe> [name-glob ...]
#   name-glob: forwarded verbatim to test-accept.sh, matched against each test's
#     relative path AND its basename. So a basename ("bug155_toInt_named_args"),
#     a full path ("rt-behavior/general/bug155_toInt_named_args"), or a glob
#     ("func_math_*") all work. With NO glob, every test under tests/ is synced.
#
# The filter matters: it is forwarded so test-accept.sh only *runs* the matching
# tests. Without it a single-fixture sync still executed the full ~15-min cycle
# (the old bug), and the copy step reconstructed tests/<arg>/golden from the arg
# — so a basename silently synced nothing. This version drives the copy off what
# test-accept.sh actually produced, so it is arg-shape agnostic and only runs the
# tests you asked for.
set -u
ROOT=$(cd "$(dirname "$0")/.." && pwd)
MFB_EXE=${1:?usage: sync-goldens.sh <mfb-exe> [name-glob ...]}
shift || true
ACTUAL=$(mktemp -d)

# Forward the name globs so test-accept.sh runs ONLY the matching tests. With no
# globs, "$@" is empty and it runs everything (mass-migration mode).
bash "$ROOT/scripts/test-accept.sh" "$MFB_EXE" "$ACTUAL" "$@" >/dev/null 2>&1 || true

# Copy off what test-accept.sh actually produced: it creates an actual dir only
# for tests that passed the filter (matches_filter runs before the mkdir), so
# every dir under $ACTUAL that maps to a tests/<rel>/golden is one we should
# sync. This makes the copy independent of how the filter args were spelled.
count=0
tests=0
while IFS= read -r adir; do
  rel=${adir#"$ACTUAL/"}
  gdir="$ROOT/tests/$rel/golden"
  [ -d "$gdir" ] || continue
  tests=$((tests + 1))
  for gf in "$gdir"/*; do
    [ -f "$gf" ] || continue
    name=$(basename "$gf")
    if [ -f "$adir/$name" ]; then
      cp "$adir/$name" "$gf"
      count=$((count + 1))
    fi
  done
done < <(find "$ACTUAL" -mindepth 1 -type d | sort)

echo "synced $count golden file(s) across $tests test(s)"
rm -rf "$ACTUAL"
