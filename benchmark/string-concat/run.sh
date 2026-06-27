#!/usr/bin/env bash
# Builds a string with 1000 concatenations.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$here/../runner.sh"

bench_build_mfb "$here/mfb"
bench_build_c   "$here/c" string-concat

echo "string-concat — 1000 concatenations:"
time_run "mfb"    "$MFB_OUT"
time_run "python" python3 "$here/python/main.py"
time_run "c -O0"  "$here/c/string-concat-O0.out"
time_run "c -O2"  "$here/c/string-concat-O2.out"
