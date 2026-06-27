#!/usr/bin/env bash
# Builds a 1000-item list by appending a 10-item list 100 times.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$here/../runner.sh"

bench_build_mfb "$here/mfb"
bench_build_c   "$here/c" list-append2

echo "list-append2 — append a 10-item list 100 times (-> 1000 items):"
time_run "mfb"    "$MFB_OUT"
time_run "python" python3 "$here/python/main.py"
time_run "c -O0"  "$here/c/list-append2-O0.out"
time_run "c -O2"  "$here/c/list-append2-O2.out"
