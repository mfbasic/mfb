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

ALL_MODES="crc32 decode-raw"

# Case counts per mode, declared HERE rather than counted from any subject's output
# (see oracle_verdict). gen.py's own count is checked against these too.
expected_cases() {
  case $1 in
    crc32) echo 118 ;; # lengths 0..17, then 100 random lengths up to 1 MiB
    decode-raw) echo 150 ;; # 3 corpora x levels 0..9 x 5 strategies
    *) echo 0 ;;
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
  for judge in python node; do
    theirs="$WORK/$mode.$judge.txt"
    [ "$(wc -l <"$theirs" | tr -d ' ')" = "$expected" ] ||
      die "$judge answered $(wc -l <"$theirs" | tr -d ' ') case(s) for $mode, expected $expected"
    # A case fails if its line differs from the judge's line with the same index.
    fails=$(awk -v judge="$judge" '
      NR == FNR { want[$2] = $0; next }
      $0 != want[$2] {
        if (shown++ < 10) printf "FAIL %s\n  mfb:   %s\n  %-6s %s\n", judge, $0, judge ":", want[$2] > "/dev/stderr"
        n++
      }
      END { print n + 0 }
    ' "$theirs" "$WORK/$mode.mine.txt")
    [ "$fails" -gt "$mode_fail" ] && mode_fail=$fails
    say "$mode: $((ran - fails))/$ran agreed with $judge"
  done
  fail_total=$((fail_total + mode_fail))
done

oracle_verdict "compress mfb-vs-zlib ($MODES)" "$ran_total" "$fail_total" "$expected_total"
