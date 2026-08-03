#!/usr/bin/env bash
# plan-78 lowering/regalloc benchmark harness.
#
# Measures the wall-clock of the native lowering + register-allocation pass on
# three fixed probes, so plan-78 B/C have a repeatable before/after number and
# the perf goal is checkable:
#
#   trivial    — a one-line program; the fixed front-end + link floor.
#   one-regex  — a single constant `regex::match`, which lowers the whole regex
#                engine inline as one ~1M-instruction function. This is the
#                workload the plan speeds up (the analysis str::eq/SipHash hot
#                loops + the colored_mask_at spill quadratic).
#   acceptance — `mfb test tests/acceptance`, the full suite compile + run. The
#                acceptance app is a TESTING project (no `main` entry), so it is
#                built and executed through `mfb test`, not `build`; its
#                wall-clock is what plan-78-C's perf goal is stated against and
#                is dominated by the same lowering/regalloc compile cost.
#
# The trivial/one-regex probes are compiled with `-ncode` (native codegen dump,
# no assemble/link) so the timing isolates lowering/regalloc + serialization
# from linker noise. Each is run against both the debug and release compiler.
# Deterministic, no network. Also prints the inlined regex function's
# instruction + vreg count (via the `MFB_BENCH_LOWERING` size probe in
# regalloc::allocate).
#
# Usage: bash scripts/bench-lowering.sh
set -euo pipefail

cd "$(dirname "$0")/.."

DEBUG=target/debug/mfb
RELEASE=target/release/mfb
PROBES_DIR=scripts/bench-probes

# Build both compilers untimed — we measure probe compile time, not the time to
# build the compiler itself.
echo "building compilers (untimed)…" >&2
cargo build --bin mfb >/dev/null 2>&1
cargo build --release --bin mfb >/dev/null 2>&1

# Wall-clock `mfb build -q -ncode <dir>`, printing seconds to 2 decimals. Cleans
# the probe's build/ + stale .ncode first so the timing is a cold compile.
probe() {
  local exe="$1" dir="$2"
  rm -rf "$dir/build"
  find "$dir" -maxdepth 1 -name '*.ncode' -delete 2>/dev/null || true
  python3 -c '
import subprocess, time, sys
t = time.monotonic()
r = subprocess.call(sys.argv[1:], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
elapsed = time.monotonic() - t
sys.stdout.write(f"{elapsed:.2f}" + ("" if r == 0 else f" (BUILD FAILED exit {r})"))
' "$exe" build -q -ncode "$dir"
}

# Wall-clock `mfb test <dir>` (compile + run the TESTING app), printing seconds.
probe_test() {
  local exe="$1" dir="$2"
  rm -rf "$dir/build"
  python3 -c '
import subprocess, time, sys
t = time.monotonic()
r = subprocess.call(sys.argv[1:], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
elapsed = time.monotonic() - t
sys.stdout.write(f"{elapsed:.2f}" + ("" if r == 0 else f" (exit {r})"))
' "$exe" test "$dir"
}

row() {
  local label="$1" dir="$2"
  local d r
  d=$(probe "$DEBUG" "$dir")
  r=$(probe "$RELEASE" "$dir")
  printf '  %-12s debug %-10s release %s\n' "$label" "$d" "$r"
}

row_test() {
  local label="$1" dir="$2"
  local d r
  d=$(probe_test "$DEBUG" "$dir")
  r=$(probe_test "$RELEASE" "$dir")
  printf '  %-12s debug %-10s release %s\n' "$label" "$d" "$r"
}

echo "== plan-78 lowering benchmark =="
row      "trivial"    "$PROBES_DIR/trivial"
row      "one-regex"  "$PROBES_DIR/one-regex"
row_test "acceptance" "tests/acceptance"

echo "== inlined regex function size (pre-allocation) =="
# Re-run the one-regex probe with the size probe enabled and surface the largest
# function's instruction/vreg counts.
rm -rf "$PROBES_DIR/one-regex/build"
MFB_BENCH_LOWERING=1 "$DEBUG" build -q -ncode "$PROBES_DIR/one-regex" 2>&1 >/dev/null \
  | grep '^MFB_BENCH_LOWERING' | sort -t= -k2 -rn | head -1 \
  | sed 's/^MFB_BENCH_LOWERING: /  regex fn: /'
