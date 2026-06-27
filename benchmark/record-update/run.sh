#!/usr/bin/env bash
# Copy-on-update stress: 10 passes incrementing every field of a 100-record list.
# A single mfb run exceeds ~100ms, so cap the default run count.
: "${BENCH_RUNS:=20}"
export BENCH_RUNS
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$here/../runner.sh"

bench_build_mfb "$here/mfb"
bench_build_c   "$here/c" record-update

echo "record-update — 10 passes of WITH-updating a 100-record list:"
time_run "mfb"    "$MFB_OUT"
time_run "python" python3 "$here/python/main.py"
time_run "c -O0"  "$here/c/record-update-O0.out"
time_run "c -O2"  "$here/c/record-update-O2.out"
