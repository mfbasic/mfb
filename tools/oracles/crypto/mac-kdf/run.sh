#!/usr/bin/env bash
# Differential cross-check: MFBASIC `crypto::hmac` / `crypto::hkdf` /
# `crypto::pbkdf2` against the RustCrypto reference in `rust/`, over the FULL
# `crypto::Hash` matrix -- all nine selectors.
#
# `tests/rt_crypto_mac_kdf_interop.rs` already checks the subset `ring` can
# compute, on every `cargo test`. This covers the rest (the SHA-3 family, and
# SHA-224 for the two KDFs) and overlaps the rest rather than abutting it, so a
# disagreement between the two references would show up here.
#
# Usage: ./run.sh [path-to-mfb-binary]
set -uo pipefail
. "$(dirname "$0")/../_lib/harness.sh"
oracle_init "$0" "$@"

# Declared here, not counted from the subject's output. See oracle_verdict.
#   14 hmac inputs + 4 hkdf inputs + 3 pbkdf2 inputs, each over 9 hashes.
EXPECTED_CASES=$(((14 + 4 + 3) * 9))

oracle_run_mfb
oracle_build_rust mackdfref

# --- ask the reference for every case, in one batch ---------------------------
# One process per case would dominate the wall clock at ~190 cases.
sed -n 's/^case //p' "$ORACLE_MFB_OUT" >"$WORK/cases.txt"
# The request is the whole line minus the trailing result, which makes the three
# members' differing field counts irrelevant here.
awk '{ NF = NF - 1; print }' "$WORK/cases.txt" >"$WORK/requests.txt"
awk '{ print $NF }' "$WORK/cases.txt" >"$WORK/mine.txt"

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
    member = $1; algo = $2; mine = $NF
    key = member "/" algo
    total[key]++; ran++
    if (mine != theirs[FNR]) {
      req = ""
      for (i = 1; i < NF; i++) req = req (i > 1 ? " " : "") $i
      printf "FAIL %s\n  request:   %s\n  mfb:       %s\n  reference: %s\n", \
             key, req, mine, theirs[FNR]
      fail++
    } else {
      ok[key]++
    }
  }
  END {
    n = 0
    for (k in total) { n++; names[n] = k }
    # Deterministic report order, so two runs are diffable.
    for (i = 1; i < n; i++)
      for (j = i + 1; j <= n; j++)
        if (names[j] < names[i]) { t = names[i]; names[i] = names[j]; names[j] = t }
    for (i = 1; i <= n; i++) {
      k = names[i]
      printf "%-18s %3d/%-3d agreed\n", k, ok[k] + 0, total[k]
    }
    printf "SUMMARY %d %d\n", ran + 0, fail + 0
  }
' "$WORK/theirs.txt" "$WORK/cases.txt" >"$WORK/report.txt"

grep -v '^SUMMARY ' "$WORK/report.txt"
summary=$(grep '^SUMMARY ' "$WORK/report.txt") || die "the comparison produced no summary"
oracle_verdict "crypto mac/kdf mfb-vs-rust" \
  "$(printf '%s' "$summary" | awk '{print $2}')" \
  "$(printf '%s' "$summary" | awk '{print $3}')" \
  "$EXPECTED_CASES"
