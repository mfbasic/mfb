#!/usr/bin/env bash
# Inverse-trig custom-kernel stress test: asin/acos/atan over 2M iterations.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$here/../runner.sh"

# Compute-heavy: a single run is slow, so default to fewer repetitions.
BENCH_RUNS="${BENCH_RUNS:-50}"

bench_build_mfb "$here/mfb"
bench_build_c   "$here/c" math-invtrig

echo "math-invtrig — asin/acos/atan over 2M iterations:"
time_run "mfb"    "$MFB_OUT"
time_run "python" python3 "$here/python/main.py"
time_run "c -O0"  "$here/c/math-invtrig-O0.out"
time_run "c -O2"  "$here/c/math-invtrig-O2.out"
