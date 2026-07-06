#!/usr/bin/env bash
# Build and run the unified benchmark for all three languages. Each language is
# a single self-contained program (benchmark/{mfb,c,python}) that times every
# micro-benchmark internally `--run` times and prints a grouped
# median/average/min/max table; this script just builds them and runs each in
# turn. The `empty` process-startup benchmark stays standalone — run
# ./benchmark/empty/run.sh for that.
#
# Usage:
#   ./benchmark/run.sh                 # 10 iterations per test (default)
#   ./benchmark/run.sh --run 50        # 50 iterations per test
#   ./benchmark/run.sh 50              # shorthand for --run 50
#   BENCH_RUNS=50 ./benchmark/run.sh   # environment override
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$here/.." && pwd)"

# Resolve the iteration count: --run N, a bare N, or $BENCH_RUNS (default 10).
runs="${BENCH_RUNS:-10}"
case "${1:-}" in
  --run) runs="${2:-$runs}" ;;
  ''|*[!0-9]*) : ;;   # not a plain number — keep default/env
  *) runs="$1" ;;
esac

MFB="${MFB:-$repo_root/target/debug/mfb}"
[ -x "$MFB" ] || MFB="$repo_root/target/release/mfb"
if [ ! -x "$MFB" ]; then
  echo "error: mfb binary not found (looked in target/debug and target/release)" >&2
  echo "build it first with: cargo build" >&2
  exit 1
fi

echo "==> building mfb worker package"
"$MFB" build "$here/mfb/workers" >/dev/null
mkdir -p "$here/mfb/packages"
cp "$here/mfb/workers/bench_workers.mfp" "$here/mfb/packages/bench_workers.mfp"

echo "==> building mfb benchmark"
"$MFB" build "$here/mfb" >/dev/null
mfb_out="$here/mfb/benchmark.out"

echo "==> building c benchmark (-O0 and -O2)"
cc -O0 -o "$here/c/bench-O0.out" "$here/c/main.c" "$here/c/list.c" -lm -lpthread
cc -O2 -o "$here/c/bench-O2.out" "$here/c/main.c" "$here/c/list.c" -lm -lpthread

# One shared timestamp for every log written by this run.
ts="$(date +%Y%m%d-%H%M%S)"

# run_one LABEL LOGNAME CMD... — run CMD, echo its table to the terminal, and
# also write it to "$here/LOGNAME-$ts.log" (checksums/progress stay on stderr).
run_one() {
  local label="$1" logname="$2"; shift 2
  local logfile="$here/${logname}-${ts}.log"
  printf '\n========================================================================\n'
  printf '  %s  (--run %s)  ->  %s\n' "$label" "$runs" "$(basename "$logfile")"
  printf '========================================================================\n'
  "$@" --run "$runs" | tee "$logfile"
}

run_one "mfb"    "mfb"    "$mfb_out"
run_one "c -O0"  "c-O0"   "$here/c/bench-O0.out"
run_one "c -O2"  "c-O2"   "$here/c/bench-O2.out"
run_one "python" "python" python3 "$here/python/main.py"

echo
echo "==> logs written (timestamp $ts):"
for n in mfb c-O0 c-O2 python; do echo "    $here/${n}-${ts}.log"; done

# Tidy up build artifacts (all git-ignored, but keep the tree clean).
rm -f "$here/c/bench-O0.out" "$here/c/bench-O2.out" \
      "$here/mfb/benchmark.out" "$here/mfb/workers/bench_workers.mfp" \
      "$here/mfb/packages/bench_workers.mfp"
