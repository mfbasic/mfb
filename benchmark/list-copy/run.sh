#!/usr/bin/env bash
# Copies a 1000-item string list 1000 times, then a 1000-item record list.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$here/../runner.sh"

bench_build_mfb "$here/mfb"
bench_build_c   "$here/c" list-copy

echo "list-copy — copy a 1000-item list 1000 times (strings, then records):"
time_run "mfb"    "$MFB_OUT"
time_run "python" python3 "$here/python/main.py"
time_run "c -O0"  "$here/c/list-copy-O0.out"
time_run "c -O2"  "$here/c/list-copy-O2.out"
