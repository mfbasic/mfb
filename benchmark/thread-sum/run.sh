#!/usr/bin/env bash
# Parallel sum of 0..39,999,999 across four worker threads. mfb runs the workers
# on real OS threads (true parallelism); Python's GIL serializes them; C uses
# pthreads. The mfb worker lives in a compiled .mfp package built here first.
set -euo pipefail
: "${BENCH_RUNS:=20}"
export BENCH_RUNS
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$here/../runner.sh"

# Build the worker package and stage its .mfp where the executable expects it.
"$MFB" build "$here/mfb/workers" >/dev/null
mkdir -p "$here/mfb/packages"
cp "$here/mfb/workers/thread_sum_workers.mfp" "$here/mfb/packages/thread_sum_workers.mfp"
BENCH_ARTIFACTS+=(
  "$here/mfb/workers/thread_sum_workers.mfp"
  "$here/mfb/packages/thread_sum_workers.mfp"
)

bench_build_mfb "$here/mfb"
bench_build_c   "$here/c" thread-sum

echo "thread-sum — parallel sum of 0..39,999,999 over 4 worker threads:"
time_run "mfb"    "$MFB_OUT"
time_run "python" python3 "$here/python/main.py"
time_run "c -O0"  "$here/c/thread-sum-O0.out"
time_run "c -O2"  "$here/c/thread-sum-O2.out"
