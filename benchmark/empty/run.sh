#!/usr/bin/env bash
# Empty entry function — measures process startup/shutdown time only.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$here/../runner.sh"

bench_build_mfb "$here/mfb"
bench_build_c   "$here/c" empty

echo "empty — process startup/shutdown only:"
time_run "mfb"    "$MFB_OUT"
time_run "python" python3 "$here/python/main.py"
time_run "c -O0"  "$here/c/empty-O0.out"
time_run "c -O2"  "$here/c/empty-O2.out"
