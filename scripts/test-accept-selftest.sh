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

echo
if [ "$failures" -eq 0 ]; then
  echo "test-accept selftest: all checks passed"
  exit 0
fi
echo "test-accept selftest: $failures check(s) failed" >&2
exit 1
