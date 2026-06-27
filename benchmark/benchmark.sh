#!/usr/bin/env bash
# Runs every benchmark in sequence. Each sub-benchmark's run.sh picks its own
# default iteration count (1000 for most; list-sort uses 5 because mfb's sort is
# ~O(n^3)). Override for the whole suite with BENCH_RUNS=N ./benchmark/benchmark.sh
set -uo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Run order: startup, compute, then the collection/string memory benchmarks,
# with the very slow list-sort last.
benchmarks=(
  empty
  primes
  list-append
  list-append2
  list-prepend
  list-copy
  map-set
  string-concat
  list-sort
)

failed=()
start="$(perl -MTime::HiRes=time -e 'printf "%.3f\n", time')"

for b in "${benchmarks[@]}"; do
  printf '\n========================================================================\n'
  printf '  %s\n' "$b"
  printf '========================================================================\n'
  if ! "$here/$b/run.sh"; then
    echo "*** $b FAILED ***"
    failed+=("$b")
  fi
done

end="$(perl -MTime::HiRes=time -e 'printf "%.3f\n", time')"
printf '\n========================================================================\n'
printf 'total wall time: %.1f s\n' "$(perl -e "printf '%.1f', $end - $start")"
if [ "${#failed[@]}" -gt 0 ]; then
  printf 'failed: %s\n' "${failed[*]}"
  exit 1
fi
printf 'all %d benchmarks completed\n' "${#benchmarks[@]}"
