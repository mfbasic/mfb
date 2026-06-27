#!/usr/bin/env bash
# Ackermann ack(3,7)=1021 — deeply nested recursive call/return overhead.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$here/../runner.sh"

bench_build_mfb "$here/mfb"
bench_build_c   "$here/c" recurse-ackermann

echo "recurse-ackermann — Ackermann ack(3,7):"
time_run "mfb"    "$MFB_OUT"
time_run "python" python3 "$here/python/main.py"
time_run "c -O0"  "$here/c/recurse-ackermann-O0.out"
time_run "c -O2"  "$here/c/recurse-ackermann-O2.out"
