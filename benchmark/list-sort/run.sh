#!/usr/bin/env bash
# Builds a list of 50 random integers, then copies and sorts it once.
#
# NOTE: mfb's collections::sort is an insertion sort whose every swap calls
# collections::set, which today reallocates and copies the whole list — so it is
# ~O(n^3): a single 200-element sort already takes ~150s, and 1000 elements would
# run for hours. The list is therefore only 50 elements and is sorted once per
# program run, with the runner defaulting to few iterations. Override with
# BENCH_RUNS=N if desired.
set -euo pipefail
: "${BENCH_RUNS:=5}"
export BENCH_RUNS
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$here/../runner.sh"

bench_build_mfb "$here/mfb"
bench_build_c   "$here/c" list-sort

echo "list-sort — sort a 50-element random list (sorts=1 per run):"
time_run "mfb"    "$MFB_OUT"
time_run "python" python3 "$here/python/main.py"
time_run "c -O0"  "$here/c/list-sort-O0.out"
time_run "c -O2"  "$here/c/list-sort-O2.out"
