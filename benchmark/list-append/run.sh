#!/usr/bin/env bash
# Runs the MFBASIC, Python, and C "append 1000 times" benchmarks, timing each.
# Each builds a MUT Integer array and a MUT String array via repeated append.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$here/../.." && pwd)"
mfb="$repo_root/target/debug/mfb"

# High-resolution wall-clock seconds since epoch (macOS `date` lacks %N).
now() { perl -MTime::HiRes=time -e 'printf "%.9f\n", time'; }

runs=1000

# Runs "$@" $runs times, discarding stdout, and prints the average wall time.
time_run() {
  local label="$1"; shift
  local start end total=0
  for ((i = 0; i < runs; i++)); do
    start="$(now)"
    "$@" >/dev/null
    end="$(now)"
    total="$(perl -e "printf '%.9f', $total + ($end - $start)")"
  done
  printf '%-8s %8.3f ms avg over %d runs\n' \
    "$label" "$(perl -e "printf '%.3f', $total / $runs * 1000")" "$runs"
}

# Build the MFBASIC project so append.out exists.
"$mfb" build "$here/mfb" >/dev/null

# Build the C version at two optimization levels.
cc -O0 -o "$here/c/append-O0.out" "$here/c/main.c"
cc -O2 -o "$here/c/append-O2.out" "$here/c/main.c"

echo "append 1000 times (int + string array):"
time_run "mfb"    "$here/mfb/append.out"
time_run "python" python3 "$here/python/main.py"
time_run "c -O0"  "$here/c/append-O0.out"
time_run "c -O2"  "$here/c/append-O2.out"
