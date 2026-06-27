#!/usr/bin/env bash
# Approximates pi via the Leibniz series (1,000,000 terms).
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$here/../runner.sh"

bench_build_mfb "$here/mfb"
bench_build_c   "$here/c" float-leibniz

echo "float-leibniz — Leibniz series for pi (1e6 terms):"
time_run "mfb"    "$MFB_OUT"
time_run "python" python3 "$here/python/main.py"
time_run "c -O0"  "$here/c/float-leibniz-O0.out"
time_run "c -O2"  "$here/c/float-leibniz-O2.out"
