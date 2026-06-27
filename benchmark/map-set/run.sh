#!/usr/bin/env bash
# Builds a 1000-item Map OF String TO Integer, then verifies each key.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$here/../runner.sh"

bench_build_mfb "$here/mfb"
bench_build_c   "$here/c" map-set

echo "map-set — build 1000-item map, then look up each key:"
time_run "mfb"    "$MFB_OUT"
time_run "python" python3 "$here/python/main.py"
time_run "c -O0"  "$here/c/map-set-O0.out"
time_run "c -O2"  "$here/c/map-set-O2.out"
