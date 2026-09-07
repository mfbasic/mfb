#!/usr/bin/env bash
# Differential cross-check: the MFBASIC `crypto::argon2id` against the
# clean-room Rust reference in `rust/`.
#
# Builds `mfb/`, runs it to derive a spread of tags, then re-derives every one
# of them from scratch with `rust/` and compares. The two sides share no code
# and no case table: `mfb/` prints the parameters it used, and this script feeds
# those printed parameters to the reference. That is the whole point -- a test
# built from values the implementation produced only ratifies the implementation.
#
# Usage: ./run.sh [path-to-mfb-binary]
set -uo pipefail
. "$(dirname "$0")/../_lib/harness.sh"
oracle_init "$0" "$@"

# The number of cases `mfb/src/main.mfb` is expected to emit, declared HERE
# rather than counted from its output. See oracle_verdict. Keep it in step with
# the `emit(...)` calls in `mfb/src/main.mfb`.
EXPECTED_CASES=12

oracle_run_mfb
oracle_build_rust argon2ref

# --- compare, case by case ----------------------------------------------------
# `mfb/` writes "-" for an empty password or salt, because an empty field would
# shift every later field left when `read` splits on spaces. Undo that here.
unfield() { [ "$1" = "-" ] && printf '' || printf '%s' "$1"; }

ran=0; fail=0
while read -r pw sa m t p l mine; do
  ran=$((ran + 1))
  theirs=$("$ORACLE_REF_BIN" run "$(unfield "$pw")" "$(unfield "$sa")" "$m" "$t" "$p" "$l" \
           2>"$WORK/ref-stderr.txt")
  label="m=$m t=$t p=$p l=$l pw=$pw salt=$sa"
  if [ -z "$theirs" ]; then
    fail=$((fail + 1))
    say "FAIL $label"
    say "  reference produced nothing: $(cat "$WORK/ref-stderr.txt")"
  elif [ "$mine" = "$theirs" ]; then
    say "OK   $label  $mine"
  else
    fail=$((fail + 1))
    say "FAIL $label"
    say "  mfb:       $mine"
    say "  reference: $theirs"
  fi
  # A tag of the wrong LENGTH agrees with nothing, but check it explicitly so a
  # length bug reads as a length bug rather than as an opaque digest mismatch.
  if [ "${#mine}" -ne $((l * 2)) ]; then
    fail=$((fail + 1))
    say "FAIL $label -- mfb returned ${#mine} hex chars, expected $((l * 2))"
  fi
done < <(sed -n 's/^case //p' "$ORACLE_MFB_OUT")

oracle_verdict "argon2id mfb-vs-rust" "$ran" "$fail" "$EXPECTED_CASES"
