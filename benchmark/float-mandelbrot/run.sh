#!/usr/bin/env bash
# Counts grid points inside the Mandelbrot set (600x600, 100 iterations).
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# A single mfb run takes ~150ms, so fewer repetitions keep the suite quick.
: "${BENCH_RUNS:=20}"
export BENCH_RUNS
source "$here/../runner.sh"

bench_build_mfb "$here/mfb"
bench_build_c   "$here/c" float-mandelbrot

echo "float-mandelbrot — in-set cell count (600x600, 100 iter):"
time_run "mfb"    "$MFB_OUT"
time_run "python" python3 "$here/python/main.py"
time_run "c -O0"  "$here/c/float-mandelbrot-O0.out"
time_run "c -O2"  "$here/c/float-mandelbrot-O2.out"
