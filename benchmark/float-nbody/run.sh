#!/usr/bin/env bash
# Classic 5-body simulation from the Computer Language Benchmarks Game (100k steps).
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$here/../runner.sh"

bench_build_mfb "$here/mfb"
bench_build_c   "$here/c" float-nbody

echo "float-nbody — 5-body simulation (100k steps):"
time_run "mfb"    "$MFB_OUT"
time_run "python" python3 "$here/python/main.py"
time_run "c -O0"  "$here/c/float-nbody-O0.out"
time_run "c -O2"  "$here/c/float-nbody-O2.out"
