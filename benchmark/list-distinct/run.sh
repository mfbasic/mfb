#!/usr/bin/env bash
# collections::distinct stress: dedupe a 5000-element List OF Integer with heavy
# duplication. distinct is contains()-in-a-loop (O(n^2)) today, so cap the
# default run count.
: "${BENCH_RUNS:=50}"
export BENCH_RUNS
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$here/../runner.sh"

bench_build_mfb "$here/mfb"
bench_build_c   "$here/c" list-distinct

echo "list-distinct — dedupe a 5000-element list (1000 distinct values):"
time_run "mfb"    "$MFB_OUT"
time_run "python" python3 "$here/python/main.py"
time_run "c -O0"  "$here/c/list-distinct-O0.out"
time_run "c -O2"  "$here/c/list-distinct-O2.out"
