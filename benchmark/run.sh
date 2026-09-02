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
mfb_out="$here/mfb/build/benchmark.out"

echo "==> building c benchmark (-O0 and -O2)"
c_srcs=("$here/c/main.c" "$here/c/listmatrix.c" "$here/c/mapbench.c" "$here/c/mapmatrix.c" "$here/c/mathbench.c" \
        "$here/c/vectorbench.c" "$here/c/bitsbench.c" "$here/c/stringbench.c" \
        "$here/c/parsebench.c" "$here/c/parson.c" "$here/c/libcsv.c" \
        "$here/c/mathpipe.c" "$here/c/churnbench.c" "$here/c/strbuildbench.c" \
        "$here/c/regexbench.c" "$here/c/arenabench.c" "$here/c/scalarbench.c" \
        "$here/c/encodingbench.c" "$here/c/datetimebench.c" \
        "$here/c/dispatchbench.c" "$here/c/cryptobench.c" \
        "$here/c/serializebench.c" "$here/c/setmatrix.c" \
        "$here/c/widthbench.c" "$here/c/pipelinebench.c" "$here/c/convertbench.c")
cc -O0 -o "$here/c/bench-O0.out" "${c_srcs[@]}" -lm -lpthread
cc -O2 -o "$here/c/bench-O2.out" "${c_srcs[@]}" -lm -lpthread

# One shared timestamp for every log written by this run.
ts="$(date +%Y%m%d-%H%M%S)"

# run_one LABEL LOGNAME CMD... — run CMD, echo its table to the terminal, write
# it to "$here/LOGNAME-$ts.log", and capture stderr (the per-row `test_* = N`
# checksums, plus any diagnostics) to "$here/LOGNAME-$ts.sums" for validation.
run_one() {
  local label="$1" logname="$2"; shift 2
  local logfile="$here/${logname}-${ts}.log"
  local sumsfile="$here/${logname}-${ts}.sums"
  printf '\n========================================================================\n'
  printf '  %s  (--run %s)  ->  %s\n' "$label" "$runs" "$(basename "$logfile")"
  printf '========================================================================\n'
  "$@" --run "$runs" 2>"$sumsfile" | tee "$logfile"
}

run_one "mfb"    "mfb"    "$mfb_out"
run_one "c -O0"  "c-O0"   "$here/c/bench-O0.out"
run_one "c -O2"  "c-O2"   "$here/c/bench-O2.out"
run_one "python" "python" python3 "$here/python/main.py"

echo
echo "==> logs written (timestamp $ts):"
for n in mfb c-O0 c-O2 python; do echo "    $here/${n}-${ts}.log"; done

# Cross-validate the per-row checksums: every shared `test_<name>` key must
# agree wherever the README claims bit-for-bit peers (mismatches are listed;
# a few rows are documented approximations). c-O0 vs c-O2 must ALWAYS agree —
# a disagreement means the optimizer changed observable work, and is fatal.
echo
echo "==> validating checksums"
python3 - "$here/mfb-${ts}.sums" "$here/c-O0-${ts}.sums" \
          "$here/c-O2-${ts}.sums" "$here/python-${ts}.sums" <<'EOF'
import re, sys
names = ["mfb", "c-O0", "c-O2", "python"]
maps = []
for p in sys.argv[1:5]:
    m = {}
    for line in open(p, encoding="utf-8", errors="replace"):
        mm = re.match(r"(test_\w+) = (-?\d+)\s*$", line)
        if mm:
            m[mm.group(1)] = mm.group(2)
    maps.append(m)
keys = sorted(set().union(*maps))
shared = [k for k in keys if sum(k in m for m in maps) > 1]
bad, hard_bad = [], []
for k in shared:
    vals = {names[i]: maps[i][k] for i in range(4) if k in maps[i]}
    if len(set(vals.values())) > 1:
        bad.append((k, vals))
        if vals.get("c-O0") is not None and vals.get("c-O2") is not None \
           and vals["c-O0"] != vals["c-O2"]:
            hard_bad.append(k)
print("    %d checksum keys, %d shared across >=2 targets, %d mismatched"
      % (len(keys), len(shared), len(bad)))
for k, vals in bad:
    print("    MISMATCH %s: %s" % (k, "  ".join("%s=%s" % nv for nv in vals.items())))
if bad:
    print("    (a few rows are documented approximations -- see"
          " 'Coverage vs. throughput' in benchmark/README.md)")
if hard_bad:
    print("ERROR: c-O0 and c-O2 disagree (same source): " + ", ".join(hard_bad))
    sys.exit(1)
EOF

# Tidy up build artifacts (all git-ignored, but keep the tree clean).
rm -f "$here/c/bench-O0.out" "$here/c/bench-O2.out" \
      "$here/mfb/build/benchmark.out" "$here/mfb/workers/bench_workers.mfp" \
      "$here/mfb/packages/bench_workers.mfp"
