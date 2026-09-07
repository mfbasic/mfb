#!/usr/bin/env bash
# Differential cross-check: MFBASIC `crypto::hash` / `crypto::shake256` against
# the RustCrypto reference in `rust/`.
#
# Builds `mfb/`, runs it to hash a spread of inputs with every algorithm the
# package exposes, then re-derives every digest with `rust/` and compares. The
# two sides share no code and no case table: `mfb/` prints the input it hashed
# alongside the digest, and this script feeds those printed inputs to the
# reference. That is the whole point -- a test built from values the
# implementation produced only ratifies the implementation.
#
# Usage: ./run.sh [path-to-mfb-binary]
#   Defaults to <repo>/target/release/mfb, building it if absent.
#
# Exit codes match the sibling oracles:
#   0  every case agreed
#   1  at least one case disagreed
#   2  the harness could not run (build failure, no cases, wrong count)
set -uo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../../../.." && pwd)

# What `mfb/src/main.mfb` is expected to emit, declared HERE rather than counted
# from its output. A count derived from the producer is true by construction: if
# the program died after 40 cases, "40 of 40 agreed" is a pass. These two
# constants are the only thing that can catch that.
#   EXPECTED_PER_INPUT -- the emitOne(...) calls in emitAll()
#   EXPECTED_INPUTS    -- the `lengths` list plus the literal inputs after it
EXPECTED_PER_INPUT=11
EXPECTED_INPUTS=37
EXPECTED_CASES=$((EXPECTED_PER_INPUT * EXPECTED_INPUTS))

WORK=$(mktemp -d "${TMPDIR:-/tmp}/hash-xcheck.XXXXXX") || exit 2
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

# --- 2. build and run the MFBASIC subject -----------------------------------
# The build reports CRYPTO_SHA1_INSECURE for the deliberate SHA-1 case; it is a
# warning, not an error, so a zero exit here is still the pass condition.
say "building mfb/"
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
  [ -s "$WORK/mfb-stderr.txt" ] && cat "$WORK/mfb-stderr.txt" >&2
  die "no built flavor ran and emitted case lines"
fi
say "ran $exe"

# --- 3. build the RustCrypto reference --------------------------------------
REF_BIN="$HERE/rust/target/release/hashref"
say "building rust/"
(cd "$HERE/rust" && cargo build --release --quiet) || die "cannot build the rust reference"
[ -x "$REF_BIN" ] || die "no reference binary at $REF_BIN"

# --- 4. ask the reference for every case, in one batch -----------------------
# One process per case would be most of this script's wall clock at ~400 cases,
# so the reference takes the whole job on stdin and answers in order.
sed -n 's/^case //p' "$MFB_OUT" >"$WORK/cases.txt"
awk '{print $1, $2, $3}' "$WORK/cases.txt" >"$WORK/requests.txt"

if ! "$REF_BIN" batch <"$WORK/requests.txt" >"$WORK/theirs.txt" 2>"$WORK/ref-stderr.txt"; then
  [ -s "$WORK/ref-stderr.txt" ] && cat "$WORK/ref-stderr.txt" >&2
  die "the reference failed on the batch"
fi

sent=$(wc -l <"$WORK/requests.txt" | tr -d ' ')
back=$(wc -l <"$WORK/theirs.txt" | tr -d ' ')
# A reference that dies halfway must not look like a short but passing run.
[ "$sent" = "$back" ] || die "sent $sent request(s) but got $back answer(s) back"

# --- 5. compare, case by case ------------------------------------------------
awk '
  NR == FNR { theirs[FNR] = $0; next }
  {
    algo = $1; outlen = $2; inhex = $3; mine = $4
    bytes = (inhex == "-") ? 0 : length(inhex) / 2
    total[algo]++; ran++
    if (length(mine) != outlen * 2) {
      printf "FAIL %s outlen=%s input=%d bytes -- mfb returned %d hex chars, expected %d\n", \
             algo, outlen, bytes, length(mine), outlen * 2
      fail++
    } else if (mine != theirs[FNR]) {
      printf "FAIL %s outlen=%s input=%d bytes\n  mfb:       %s\n  reference: %s\n", \
             algo, outlen, bytes, mine, theirs[FNR]
      fail++
    } else {
      ok[algo]++
    }
  }
  END {
    n = 0
    for (a in total) { n++; names[n] = a }
    # Deterministic report order, so two runs are diffable.
    for (i = 1; i < n; i++)
      for (j = i + 1; j <= n; j++)
        if (names[j] < names[i]) { t = names[i]; names[i] = names[j]; names[j] = t }
    for (i = 1; i <= n; i++) {
      a = names[i]
      printf "%-9s %3d/%-3d agreed\n", a, ok[a] + 0, total[a]
    }
    printf "SUMMARY %d %d\n", ran + 0, fail + 0
  }
' "$WORK/theirs.txt" "$WORK/cases.txt" >"$WORK/report.txt"

grep -v '^SUMMARY ' "$WORK/report.txt"
summary=$(grep '^SUMMARY ' "$WORK/report.txt") || die "the comparison produced no summary"
ran=$(printf '%s' "$summary" | awk '{print $2}')
fail=$(printf '%s' "$summary" | awk '{print $3}')

# --- 6. verdict --------------------------------------------------------------
say "crypto::hash mfb-vs-rust: $ran case(s), $fail failure(s)"
if [ "$ran" -ne "$EXPECTED_CASES" ]; then
  say "ran $ran case(s) but expected $EXPECTED_CASES -- that is a harness bug, not a pass" >&2
  exit 2
fi
[ "$fail" -eq 0 ] || exit 1
exit 0
