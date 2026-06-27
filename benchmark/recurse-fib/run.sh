#!/usr/bin/env bash
# Naive recursive Fibonacci fib(35) — pure recursive call/return overhead.
# fib(35) makes ~29M calls, so a single mfb run far exceeds ~100ms; use a
# reduced run count so the default suite stays fast.
: "${BENCH_RUNS:=20}"
export BENCH_RUNS
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$here/../runner.sh"

bench_build_mfb "$here/mfb"
bench_build_c   "$here/c" recurse-fib

echo "recurse-fib — naive recursive fib(35):"
time_run "mfb"    "$MFB_OUT"
time_run "python" python3 "$here/python/main.py"
time_run "c -O0"  "$here/c/recurse-fib-O0.out"
time_run "c -O2"  "$here/c/recurse-fib-O2.out"
