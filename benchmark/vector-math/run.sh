#!/usr/bin/env bash
# 3D vector-math throughput: 200k iterations of normalize/cross/lerp/scale/dot/
# length/distance over the vector:: Float3 surface. Float-heavy (four sqrts and
# a dozen multiplies per iteration), so it defaults to fewer runs like the other
# math benchmarks. Override with BENCH_RUNS=N.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$here/../runner.sh"

: "${BENCH_RUNS:=50}"
export BENCH_RUNS

bench_build_mfb "$here/mfb"
bench_build_c   "$here/c" vector-math

echo "vector-math — Float3 geometry over 200k iterations:"
time_run "mfb"    "$MFB_OUT"
time_run "python" python3 "$here/python/main.py"
time_run "c -O0"  "$here/c/vector-math-O0.out"
time_run "c -O2"  "$here/c/vector-math-O2.out"
