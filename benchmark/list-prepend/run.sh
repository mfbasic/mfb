#!/usr/bin/env bash
# Builds a 1000-item list by prepending one item at a time.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$here/../runner.sh"

bench_build_mfb "$here/mfb"
bench_build_c   "$here/c" list-prepend

echo "list-prepend — 1000 prepends:"
time_run "mfb"    "$MFB_OUT"
time_run "python" python3 "$here/python/main.py"
time_run "c -O0"  "$here/c/list-prepend-O0.out"
time_run "c -O2"  "$here/c/list-prepend-O2.out"
