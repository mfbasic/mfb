#!/usr/bin/env bash
# Computes and prints the first 1000 prime numbers.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$here/../runner.sh"

bench_build_mfb "$here/mfb"
bench_build_c   "$here/c" primes

echo "primes — first 1000 primes:"
time_run "mfb"    "$MFB_OUT"
time_run "python" python3 "$here/python/main.py"
time_run "c -O0"  "$here/c/primes-O0.out"
time_run "c -O2"  "$here/c/primes-O2.out"
