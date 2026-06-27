#!/usr/bin/env bash
# Writes 100000 integer lines (0..99999) to stdout.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$here/../runner.sh"

bench_build_mfb "$here/mfb"
bench_build_c   "$here/c" io-write

echo "io-write — write 100000 lines (integers 0..99999) to stdout:"
time_run "mfb"    "$MFB_OUT"
time_run "python" python3 "$here/python/main.py"
time_run "c -O0"  "$here/c/io-write-O0.out"
time_run "c -O2"  "$here/c/io-write-O2.out"
