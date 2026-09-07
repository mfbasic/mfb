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
#   Defaults to <repo>/target/release/mfb, building it if absent.
#
# Exit codes match `rust/openssl-xcheck.sh`:
#   0  every case agreed
#   1  at least one case disagreed
#   2  the harness could not run (build failure, no cases, wrong count)
set -uo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../../../.." && pwd)

# The number of cases `mfb/src/main.mfb` is expected to emit, declared HERE
# rather than counted from its output. A count derived from the producer is true
# by construction: if the program dies after case 3, "3 of 3 agreed" is a pass.
# This constant is the only thing that can catch that, so keep it in step with
# the `emit(...)` calls in `mfb/src/main.mfb`.
EXPECTED_CASES=12

WORK=$(mktemp -d "${TMPDIR:-/tmp}/argon2id-xcheck.XXXXXX") || exit 2
trap 'rm -rf "$WORK"' EXIT
MFB_OUT="$WORK/mfb-output.txt"

say() { printf '%s\n' "$*"; }
die() { printf '%s\n' "$*" >&2; exit 2; }

# --- 1. the mfb compiler ----------------------------------------------------
MFB_EXE=${1:-${MFB_EXE:-$ROOT/target/release/mfb}}
if [ ! -x "$MFB_EXE" ]; then
  say "building $MFB_EXE"
  (cd "$ROOT" && cargo build --release --bin mfb) || die "cannot build mfb"
fi
[ -x "$MFB_EXE" ] || die "no such mfb binary: $MFB_EXE"

# --- 2. build the MFBASIC subject -------------------------------------------
say "building mfb/ with $("$MFB_EXE" --version 2>/dev/null || echo mfb)"
rm -rf "$HERE/mfb/build"
build_log=$("$MFB_EXE" build -q "$HERE/mfb" 2>&1) || {
  printf '%s\n' "$build_log" >&2
  die "mfb build failed"
}

# `mfb build` can emit several libc flavors on one host; only some of them load
# here. Rather than guess from `ldd`, try each and keep the first that actually
# produces case lines -- a flavor this host cannot exec fails to run at all.
paths=$(printf '%s\n' "$build_log" | sed -n 's/^Wrote executable to //p')
[ -n "$paths" ] || { printf '%s\n' "$build_log" >&2; die "build reported no executable"; }

: >"$MFB_OUT"
exe=""
while IFS= read -r candidate; do
  [ -n "$candidate" ] || continue
  [ -x "$candidate" ] || continue
  if "$candidate" >"$MFB_OUT" 2>"$WORK/mfb-stderr.txt" && grep -q '^case ' "$MFB_OUT"; then
    exe=$candidate
    break
  fi
done <<EOF
$paths
EOF
if [ -z "$exe" ]; then
  cat "$WORK/mfb-stderr.txt" >&2 2>/dev/null
  die "no built flavor ran and emitted case lines"
fi
say "ran $exe"

# --- 3. build the Rust reference --------------------------------------------
REF_BIN="$HERE/rust/target/release/argon2ref"
say "building rust/"
(cd "$HERE/rust" && cargo build --release --quiet) || die "cannot build the rust reference"
[ -x "$REF_BIN" ] || die "no reference binary at $REF_BIN"

# --- 4. compare, case by case -----------------------------------------------
# `mfb/` writes "-" for an empty password or salt, because an empty field would
# shift every later field left when `read` splits on spaces. Undo that here.
unfield() { [ "$1" = "-" ] && printf '' || printf '%s' "$1"; }

ran=0; fail=0
while read -r pw sa m t p l mine; do
  ran=$((ran + 1))
  theirs=$("$REF_BIN" run "$(unfield "$pw")" "$(unfield "$sa")" "$m" "$t" "$p" "$l" 2>"$WORK/ref-stderr.txt")
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
done < <(sed -n 's/^case //p' "$MFB_OUT")

# --- 5. verdict --------------------------------------------------------------
say "argon2id mfb-vs-rust: $ran case(s), $fail failure(s)"
if [ "$ran" -ne "$EXPECTED_CASES" ]; then
  say "ran $ran case(s) but expected $EXPECTED_CASES -- that is a harness bug, not a pass" >&2
  exit 2
fi
[ "$fail" -eq 0 ] || exit 1
exit 0
