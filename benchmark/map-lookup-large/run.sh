#!/usr/bin/env bash
# Builds a 20000-item Map OF Integer TO Integer, then looks up each key. Exercises
# map lookup scaling (linear scan O(n^2) vs the Phase 6 hash index O(1) average).
set -euo pipefail
: "${BENCH_RUNS:=200}"
export BENCH_RUNS
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$here/../runner.sh"

bench_build_mfb "$here/mfb"
bench_build_c   "$here/c" map-lookup-large

echo "map-lookup-large — build 20000-item map, then look up each key:"
time_run "mfb"    "$MFB_OUT"
time_run "python" python3 "$here/python/main.py"
time_run "c -O0"  "$here/c/map-lookup-large-O0.out"
time_run "c -O2"  "$here/c/map-lookup-large-O2.out"
