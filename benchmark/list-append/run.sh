#!/usr/bin/env bash
# Builds a MUT Integer array and a MUT String array via 1000 appends each.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$here/../runner.sh"

bench_build_mfb "$here/mfb"
bench_build_c   "$here/c" list-append

echo "list-append — 1000 appends (int + string array):"
time_run "mfb"    "$MFB_OUT"
time_run "python" python3 "$here/python/main.py"
time_run "c -O0"  "$here/c/list-append-O0.out"
time_run "c -O2"  "$here/c/list-append-O2.out"
