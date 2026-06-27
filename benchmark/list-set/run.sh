#!/usr/bin/env bash
# Fixed-width list set stress: 10 passes overwriting every element of a
# 200-element List OF Integer via collections::set. Each set rebuilds the whole
# list today (two allocs + two O(n) copies), so a single run is already ~1.5s;
# cap the default run count like list-sort. After the Phase 1 in-place fix this
# should be near-instant.
: "${BENCH_RUNS:=5}"
export BENCH_RUNS
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$here/../runner.sh"

bench_build_mfb "$here/mfb"
bench_build_c   "$here/c" list-set

echo "list-set — 10 passes of set-incrementing a 200-element List OF Integer:"
time_run "mfb"    "$MFB_OUT"
time_run "python" python3 "$here/python/main.py"
time_run "c -O0"  "$here/c/list-set-O0.out"
time_run "c -O2"  "$here/c/list-set-O2.out"
