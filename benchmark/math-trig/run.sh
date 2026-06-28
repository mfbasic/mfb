#!/usr/bin/env bash
# Forward-trig custom-kernel stress test: sin/cos/tan/atan2 over 2M iterations.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$here/../runner.sh"

# Compute-heavy: a single run is slow, so default to fewer repetitions.
BENCH_RUNS="${BENCH_RUNS:-50}"

bench_build_mfb "$here/mfb"
bench_build_c   "$here/c" math-trig

echo "math-trig — sin/cos/tan/atan2 over 2M iterations:"
time_run "mfb"    "$MFB_OUT"
time_run "python" python3 "$here/python/main.py"
time_run "c -O0"  "$here/c/math-trig-O0.out"
time_run "c -O2"  "$here/c/math-trig-O2.out"
