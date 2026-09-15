#!/usr/bin/env bash
# Differential oracle: the builtin `compress::` package against zlib, twice over --
# Python's stdlib `zlib` (python/oracle.py) and Node's built-in `node:zlib`
# (node/oracle.mjs).
#
# For each mode, python/gen.py writes a seeded job file; the MFB probe (mfb/) and
# both judges read that same job and print one `case <index> <fields...>` line per
# case; this script compares the probe's lines with each judge's. The probe and the
# judges share no case table and no code.
#
# Usage: ./run.sh [path-to-mfb-binary] [mode ...]     (no modes = every mode)
# Exit: 0 every case agreed with both judges, 1 a disagreement, 2 harness failure.
set -uo pipefail
. "$(dirname "$0")/../crypto/_lib/harness.sh"

ALL_MODES="crc32 decode-raw decode-zlib decode-gzip"

# Case counts per mode, declared HERE rather than counted from any subject's output
# (see oracle_verdict). gen.py's own count is checked against these too.
expected_cases() {
  case $1 in
    crc32) echo 118 ;; # lengths 0..17, then 100 random lengths up to 1 MiB
    decode-raw) echo 150 ;; # 3 corpora x levels 0..9 x 5 strategies
    decode-zlib) echo 150 ;; # the same matrix, zlib-wrapped
    decode-gzip) echo 159 ;; # the same matrix gzip-wrapped, 3 multi-member files, 6 optional-header shapes
    mutate) echo 600 ;; # seeded 1-3 byte edits of 36 valid raw / zlib / gzip streams
    *) echo 0 ;;
  esac
}

# Case indices where `compress` deliberately differs from ONE judge, per mode. Declared here,
# never derived from output, and each is documented with probe evidence in README.md
# "Declared divergences". A skipped case is still printed as skipped; if it ever stops
# diverging, the run says so, so a stale entry cannot quietly hide a change.
declared_divergences() {
  case "$1 $2" in
    "decode-gzip node") echo "152" ;; # member + non-1f8b trailing bytes: compress ignores them (decided); Node refuses
    *) echo "" ;;
  esac
}

MFB_ARG=""
if [ $# -gt 0 ] && [ -f "$1" ]; then
  MFB_ARG=$1
  shift
fi
oracle_init "$0" ${MFB_ARG:+"$MFB_ARG"}
MODES=${*:-$ALL_MODES}

command -v python3 >/dev/null || die "python3 is required"
command -v node >/dev/null || die "node is required"
say "python zlib $(python3 -c 'import zlib; print(zlib.ZLIB_RUNTIME_VERSION)'), node zlib $(node -e 'console.log(process.versions.zlib)')"

# The probe prints nothing without a job, so no output marker is required.
oracle_build_mfb

ran_total=0
fail_total=0
expected_total=0
for mode in $MODES; do
  expected=$(expected_cases "$mode")
  [ "$expected" -gt 0 ] || die "unknown mode: $mode (modes: $ALL_MODES)"
  expected_total=$((expected_total + expected))

  job="$WORK/$mode.job"
  generated=$(python3 "$HERE/python/gen.py" "$mode" "$job") || die "gen.py failed for $mode"
  [ "$generated" = "$expected" ] ||
    die "gen.py wrote $generated case(s) for $mode but run.sh declares $expected"

  if ! MFB_COMPRESS_JOB="$job" MFB_COMPRESS_MODE="$mode" "$ORACLE_MFB_EXE" \
    >"$WORK/$mode.mfb.txt" 2>"$WORK/$mode.mfb.err"; then
    cat "$WORK/$mode.mfb.err" >&2
    die "the MFB probe failed on $mode"
  fi
  python3 "$HERE/python/oracle.py" "$mode" "$job" >"$WORK/$mode.python.txt" ||
    die "python/oracle.py failed on $mode"
  node "$HERE/node/oracle.mjs" "$mode" "$job" >"$WORK/$mode.node.txt" ||
    die "node/oracle.mjs failed on $mode"

  grep '^case ' "$WORK/$mode.mfb.txt" >"$WORK/$mode.mine.txt"
  ran=$(wc -l <"$WORK/$mode.mine.txt" | tr -d ' ')
  ran_total=$((ran_total + ran))

  mode_fail=0
  if [ "$mode" = mutate ]; then
    # Three-way, by verdict. Hard failures: compress accepts what BOTH zlibs refuse (lenient),
    # refuses what both accept (strict), or all accept with different output. Cases where the two
    # judges disagree are bucketed and listed for inspection, not counted as failures.
    for judge in python node; do
      [ "$(wc -l <"$WORK/$mode.$judge.txt" | tr -d ' ')" = "$expected" ] ||
        die "$judge answered $(wc -l <"$WORK/$mode.$judge.txt" | tr -d ' ') case(s) for $mode, expected $expected"
    done
    report=$(awk '
      FILENAME == ARGV[1] { py[$2] = $3 " " $4 " " $5; next }
      FILENAME == ARGV[2] { nd[$2] = $3 " " $4 " " $5; next }
      {
        i = $2; m = $3 " " $4 " " $5; p = py[i]; n = nd[i]
        mv = ($3 == "ok"); pv = (substr(p, 1, 2) == "ok"); nv = (substr(n, 1, 2) == "ok")
        if (p != n) {
          bucket["judges disagree (inspect)"]++
          if (shown++ < 8) printf "judges disagree: case %s  mfb=%s  python=%s  node=%s\n", i, m, p, n > "/dev/stderr"
          next
        }
        if (mv && pv && m == p) bucket["all accept, same output"]++
        else if (!mv && !pv) bucket["all refuse"]++
        else if (mv && !pv) { bucket["LENIENT: compress accepts, zlib refuses"]++; fail++; printf "LENIENT case %s  mfb=%s  zlib=%s\n", i, m, p > "/dev/stderr" }
        else if (!mv && pv) { bucket["STRICT: compress refuses, zlib accepts"]++; fail++; printf "STRICT case %s  mfb=%s  zlib=%s\n", i, m, p > "/dev/stderr" }
        else { bucket["OUTPUT differs"]++; fail++; printf "OUTPUT case %s  mfb=%s  zlib=%s\n", i, m, p > "/dev/stderr" }
      }
      END { for (b in bucket) printf "BUCKET %s: %d\n", b, bucket[b]; printf "FAILS %d\n", fail + 0 }
    ' "$WORK/$mode.python.txt" "$WORK/$mode.node.txt" "$WORK/$mode.mine.txt")
    printf '%s\n' "$report" | sed -n 's/^BUCKET /  mutate: /p' | sort
    mode_fail=$(printf '%s\n' "$report" | sed -n 's/^FAILS //p')
    fail_total=$((fail_total + mode_fail))
    say "$mode: $ran case(s), $mode_fail hard failure(s)"
    continue
  fi
  for judge in python node; do
    theirs="$WORK/$mode.$judge.txt"
    [ "$(wc -l <"$theirs" | tr -d ' ')" = "$expected" ] ||
      die "$judge answered $(wc -l <"$theirs" | tr -d ' ') case(s) for $mode, expected $expected"
    # A case fails if its line differs from the judge's line with the same index, unless the
    # index is a declared divergence for this (mode, judge).
    skip=$(declared_divergences "$mode" "$judge")
    result=$(awk -v judge="$judge" -v skip="$skip" '
      BEGIN { k = split(skip, s, " "); for (i = 1; i <= k; i++) skipped[s[i]] = 1 }
      NR == FNR { want[$2] = $0; next }
      ($2 in skipped) {
        declared++
        if ($0 == want[$2]) printf "note: declared divergence %s case %s no longer diverges\n", judge, $2 > "/dev/stderr"
        next
      }
      $0 != want[$2] {
        if (shown++ < 10) printf "FAIL %s\n  mfb:   %s\n  %-6s %s\n", judge, $0, judge ":", want[$2] > "/dev/stderr"
        n++
      }
      END { print n + 0, declared + 0 }
    ' "$theirs" "$WORK/$mode.mine.txt")
    fails=${result% *}
    declared=${result#* }
    [ "$fails" -gt "$mode_fail" ] && mode_fail=$fails
    if [ "$declared" -gt 0 ]; then
      say "$mode: $((ran - declared - fails))/$((ran - declared)) agreed with $judge ($declared declared divergence(s) skipped: case $skip)"
    else
      say "$mode: $((ran - fails))/$ran agreed with $judge"
    fi
  done
  fail_total=$((fail_total + mode_fail))
done

oracle_verdict "compress mfb-vs-zlib ($MODES)" "$ran_total" "$fail_total" "$expected_total"
