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
set -uo pipefail
. "$(dirname "$0")/../_lib/harness.sh"
oracle_init "$0" "$@"

# What `mfb/src/main.mfb` is expected to emit, declared HERE rather than counted
# from its output. See oracle_verdict.
#   EXPECTED_PER_INPUT -- the emitOne(...) calls in emitAll()
#   EXPECTED_INPUTS    -- the `lengths` list plus the literal inputs after it
EXPECTED_PER_INPUT=11
EXPECTED_INPUTS=37
EXPECTED_CASES=$((EXPECTED_PER_INPUT * EXPECTED_INPUTS))

# The build reports CRYPTO_SHA1_INSECURE for the deliberate SHA-1 case; it is a
# warning, not an error, so a zero exit is still the pass condition.
oracle_run_mfb
oracle_build_rust hashref

# --- ask the reference for every case, in one batch ---------------------------
# One process per case would be most of this script's wall clock at ~400 cases,
# so the reference takes the whole job on stdin and answers in order.
sed -n 's/^case //p' "$ORACLE_MFB_OUT" >"$WORK/cases.txt"
awk '{print $1, $2, $3}' "$WORK/cases.txt" >"$WORK/requests.txt"

if ! "$ORACLE_REF_BIN" batch <"$WORK/requests.txt" >"$WORK/theirs.txt" 2>"$WORK/ref-stderr.txt"; then
  [ -s "$WORK/ref-stderr.txt" ] && cat "$WORK/ref-stderr.txt" >&2
  die "the reference failed on the batch"
fi

sent=$(wc -l <"$WORK/requests.txt" | tr -d ' ')
back=$(wc -l <"$WORK/theirs.txt" | tr -d ' ')
# A reference that dies halfway must not look like a short but passing run.
[ "$sent" = "$back" ] || die "sent $sent request(s) but got $back answer(s) back"

# --- compare, case by case ----------------------------------------------------
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
oracle_verdict "crypto::hash mfb-vs-rust" \
  "$(printf '%s' "$summary" | awk '{print $2}')" \
  "$(printf '%s' "$summary" | awk '{print $3}')" \
  "$EXPECTED_CASES"
