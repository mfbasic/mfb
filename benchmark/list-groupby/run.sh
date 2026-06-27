#!/usr/bin/env bash
# collections::groupBy stress: group a 2000-element List OF Integer into 100
# buckets. groupBy builds its map with set + hasKey in a loop and rebuilds each
# group list on append (O(n^2) today), so cap the default run count.
: "${BENCH_RUNS:=10}"
export BENCH_RUNS
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$here/../runner.sh"

bench_build_mfb "$here/mfb"
bench_build_c   "$here/c" list-groupby

echo "list-groupby — group a 2000-element list into 100 buckets:"
time_run "mfb"    "$MFB_OUT"
time_run "python" python3 "$here/python/main.py"
time_run "c -O0"  "$here/c/list-groupby-O0.out"
time_run "c -O2"  "$here/c/list-groupby-O2.out"
