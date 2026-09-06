#!/usr/bin/env bash
#
# Self-test for `test-accept.sh`'s own machinery (bug-320).
#
# The acceptance harness executes 462 fixture programs. Before bug-320 it did so
# with no timeout, so any program that failed to exit wedged the whole suite with
# no output, no named failing fixture, and no exit code — the least diagnosable
# failure mode available, and indistinguishable from a slow machine.
#
# The regression this guards is "a program that blocks forever fails *that*
# fixture and lets the suite continue". Proving that with a real fixture would
# mean shipping a program that hangs, and every acceptance run would then burn the
# full timeout on it forever. So the watchdog is exercised directly here instead.
#
# Run: scripts/test-accept-selftest.sh
set -u

ROOT=$(cd "$(dirname "$0")/.." && pwd)
HARNESS="$ROOT/scripts/test-accept.sh"

# Pull the helper out of the harness rather than sourcing it — test-accept.sh runs
# a full suite on load. If the function is ever renamed, extraction yields nothing
# and this test fails loudly, which is the correct outcome.
helper=$(sed -n '/^run_with_watchdog() {/,/^}/p' "$HARNESS")
if [ -z "$helper" ]; then
  echo "FAIL: could not extract run_with_watchdog from $HARNESS" >&2
  exit 1
fi
eval "$helper"

failures=0

check() {
  local label=$1 expected=$2 actual=$3
  if [ "$expected" = "$actual" ]; then
    echo "ok   - $label"
  else
    echo "FAIL - $label: expected [$expected], got [$actual]" >&2
    failures=$((failures + 1))
  fi
}

# 1. A normal program's stdout/stderr reach the log and its exit code survives —
#    the watchdog must be transparent, since that output is diffed as build.log.
out=$(run_with_watchdog /bin/sh -c 'echo out; echo err >&2; exit 7' 2>&1)
rc=$?
check "passthrough output" "out
err" "$out"
check "passthrough exit code" "7" "$rc"

# 2. A signal reports as 128+N, matching what the shell reported when the harness
#    invoked the program directly, so existing `[exit N]` goldens do not churn.
run_with_watchdog /bin/sh -c 'kill -9 $$' >/dev/null 2>&1
check "signal maps to 128+N" "137" "$?"

# 3. The bug itself: a program that never exits is bounded, prints `timeout` into
#    the fixture log so it diffs against build.log, and yields 99.
start=$(date +%s)
out=$(MFB_ACCEPT_RUN_TIMEOUT=2 run_with_watchdog /bin/sh -c 'sleep 300' 2>&1)
rc=$?
elapsed=$(($(date +%s) - start))
check "hung program prints timeout" "timeout" "$out"
check "hung program exits 99" "99" "$rc"
if [ "$elapsed" -le 10 ]; then
  echo "ok   - hung program is bounded (${elapsed}s)"
else
  echo "FAIL - hung program not bounded: ${elapsed}s" >&2
  failures=$((failures + 1))
fi

# 4. stdin is /dev/null regardless of the harness's own fd 0. plan-15's broadcast
#    reader subscribes to fd 0; on a live pipe it blocks forever, which is exactly
#    how this was originally triggered (`nohup ./scripts/test-accept.sh ... &`).
#    Without the child-side redirect, `cat` here never sees EOF.
out=$( { sleep 60 | MFB_ACCEPT_RUN_TIMEOUT=5 run_with_watchdog \
  /bin/sh -c 'cat; echo reached-eof'; } 2>&1 )
check "stdin is /dev/null under a live pipe" "reached-eof" "$out"

# 5. bug-455: the rival guard must match only a process actually EXECUTING the
#    harness, never one whose command line merely mentions it. Another session's
#    wrapper shell (`zsh -c "... scripts/test-accept.sh ..."`) matches
#    `pgrep -f` while holding no lock -- observed blocking a run whose "rival"
#    was still in its `cargo build` stage, and deadlocking two sessions politely
#    queueing on each other's text.
#
#    The guard decides from the candidate's argv: a real invocation has the
#    script as argv[0] (`./scripts/test-accept.sh`) or argv[1]
#    (`bash scripts/test-accept.sh`), a wrapper has `-c` there. That decision is
#    exercised here over argv strings, so the check needs no live processes and
#    cannot itself race.
classify_argv() {                      # $1 = a full command line
  cargs=$1
  ca0=${cargs%% *}
  carest=${cargs#* }
  ca1=${carest%% *}
  case "$ca0" in
    */test-accept.sh|test-accept.sh) echo rival; return ;;
  esac
  case "$ca1" in
    */test-accept.sh|test-accept.sh) echo rival; return ;;
  esac
  echo skipped
}

check "wrapper mentioning the harness in a -c string is not a rival" "skipped" \
  "$(classify_argv "/bin/zsh -c eval scripts/test-accept.sh target/release/mfb /tmp/x")"
check "wrapper mentioning it after other text is not a rival" "skipped" \
  "$(classify_argv "/bin/bash -c cargo test && scripts/test-accept.sh a b")"
check "direct invocation is a rival" "rival" \
  "$(classify_argv "scripts/test-accept.sh target/release/mfb /tmp/out")"
check "absolute direct invocation is a rival" "rival" \
  "$(classify_argv "/repo/scripts/test-accept.sh target/release/mfb /tmp/out")"
check "shell-prefixed invocation is a rival" "rival" \
  "$(classify_argv "bash scripts/test-accept.sh target/release/mfb /tmp/out")"
check "the lighter selftest harness is not a rival" "skipped" \
  "$(classify_argv "bash scripts/test-accept-selftest.sh")"

# 6. bug-456: an `MFB_OPT=<n>` sweep at a non-default level must stop comparing
#    the per-target native dumps and must keep comparing everything else.
#
#    Before the fix every such sweep exited 1 with a fixed set of mismatches --
#    the dial rewriting the code those goldens pin, exactly as designed -- so a
#    real `.run` or build.log regression sat in noise the operator had to
#    recognise by eye. The two halves are tested separately because getting only
#    the first half right yields a sweep that passes by comparing nothing.
#
#    Both decisions are EXTRACTED from the shipping scripts, never restated
#    here: `artifact_kind_is_level_variant` by sourcing the shared kinds table,
#    `level_variant_skip_for` and `compare_native_output` out of the harness. A
#    rename yields nothing and fails this file loudly, which is correct.
# shellcheck source=artifact-kinds.sh
. "$ROOT/scripts/artifact-kinds.sh"
if ! command -v artifact_kind_is_level_variant >/dev/null 2>&1; then
  echo "FAIL: artifact-kinds.sh defines no artifact_kind_is_level_variant" >&2
  exit 1
fi
for fn in level_variant_skip_for compare_native_output; do
  body=$(sed -n "/^$fn() {/,/^}/p" "$HARNESS")
  if [ -z "$body" ]; then
    echo "FAIL: could not extract $fn from $HARNESS" >&2
    exit 1
  fi
  eval "$body"
done

# 6a. Which levels skip. `-O1` is the flagless default, so MFB_OPT=1 is
#     plan-100's "-O1 == default" gate and must still compare everything.
check "unset MFB_OPT compares everything"  "0" "$(level_variant_skip_for '')"
check "MFB_OPT=1 compares everything"      "0" "$(level_variant_skip_for 1)"
check "MFB_OPT=0 skips level-variant"      "1" "$(level_variant_skip_for 0)"
check "MFB_OPT=2 skips level-variant"      "1" "$(level_variant_skip_for 2)"
check "MFB_OPT=3 skips level-variant"      "1" "$(level_variant_skip_for 3)"
check "a non-numeric MFB_OPT is not a level" "0" "$(level_variant_skip_for x)"

# 6b. Which kinds are level-variant. EVERY per-target native dump is emitted
#     downstream of the `-O`-gated NIR passes, so all five are skipped -- not
#     just the two that happened to drift when this was first measured. The host
#     kinds are produced before native lowering and stay compared; so does every
#     kind the harness owns outside the shared table (build.log, .run, .testrun,
#     .audit, .mfp, .info, the coverage sidecars, .ncodesum).
for k in $ARTIFACT_NATIVE_KINDS; do
  if artifact_kind_is_level_variant "$k"; then
    echo "ok   - .$k is level-variant"
  else
    echo "FAIL - .$k should be level-variant" >&2
    failures=$((failures + 1))
  fi
done
for k in $ARTIFACT_HOST_KINDS log run testrun audit mfp info covmap.json covdata covfail ncodesum; do
  if artifact_kind_is_level_variant "$k"; then
    echo "FAIL - .$k must stay compared at every level" >&2
    failures=$((failures + 1))
  else
    echo "ok   - .$k stays compared at every level"
  fi
done

# 6c. The comparison seam itself. `compare_native_output` is driven with a
#     recording stub in place of `compare_optional_output`, so both halves are
#     observable: what it skips (and counts) and what it still hands on.
compared=""
compare_optional_output() { compared="$compared $1"; }
probe_golden=$(mktemp)
probe_native() {                       # $1 = skip flag, $2.. = kinds
  skip_level_variant=$1; shift
  compared=""; level_variant_skipped=0
  for k in "$@"; do
    compare_native_output "$k" "$k" "$probe_golden" "$probe_golden"
  done
}

probe_native 1 ncode mir nir nplan nobj ast ir
check "at -O3 the native dumps drop out and the host kinds do not" \
  " ast ir" "$compared"
check "at -O3 every native golden is counted as skipped" "5" "$level_variant_skipped"

probe_native 1 ast ir hex
check "at -O3 a run with only host goldens compares all of them" \
  " ast ir hex" "$compared"
check "at -O3 a host golden is never counted as skipped" "0" "$level_variant_skipped"

probe_native 0 ncode mir nir nplan nobj
check "at the default level every native dump is compared" \
  " ncode mir nir nplan nobj" "$compared"
check "at the default level nothing is counted as skipped" "0" "$level_variant_skipped"

# A kind with no golden is not counted: the summary reports goldens skipped, not
# kinds considered, so a fixture carrying none must not inflate the number.
skip_level_variant=1; compared=""; level_variant_skipped=0
compare_native_output ncode ncode "$probe_golden.absent" "$probe_golden.absent"
check "a native kind with no golden is not counted" "0" "$level_variant_skipped"
rm -f "$probe_golden"
unset -f compare_optional_output

echo
if [ "$failures" -eq 0 ]; then
  echo "test-accept selftest: all checks passed"
  exit 0
fi
echo "test-accept selftest: $failures check(s) failed" >&2
exit 1
