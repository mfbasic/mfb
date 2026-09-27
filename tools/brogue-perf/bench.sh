#!/usr/bin/env bash
# bug-707: measure examples/brogue's `dig` user-CPU-time cost against the C
# oracle's own `terrain_dump`, for a fixed (level seed, depth) matrix, and
# fail if the port/oracle ratio exceeds a budget.
#
# This is a *measurement* gate, not a correctness check (that is
# examples/brogue/check/check-terrain.sh, which this tool never replaces —
# run it after any change here shows a diff). It builds the real
# examples/brogue tree and the real oracle/src engine; it never copies or
# reimplements either.
#
# Usage: tools/brogue-perf/bench.sh [--budget N] [--runs N] [--quiet]
#   --budget N   fail if median(port user s) / median(oracle user s) > N
#                (default: see BUDGET_RATIO below)
#   --runs N     repetitions per (seed, depth) pair to take the median of
#                (default 5; this machine is shared and noisy)
#   --quiet      print only the final summary line
# Env:  MFB   the mfb binary (default: the repository's target/release/mfb)
#
# Also builds a --debug port and reports its arena.0.alloc_calls for the
# first matrix pair, for change-tracking (not gated on).

set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
brogue="$(cd "$here/../../examples/brogue" && pwd)"
repo="$(cd "$here/../.." && pwd)"
mfb="${MFB:-$repo/target/release/mfb}"

# bug-707's proposed goal was "within 10x of the oracle"; its two identified
# fixes (landed) cut allocation count 96.3% but left the ratio at ~120x on a
# quiet machine (median of 7, tools/brogue-perf lock held) -- see bug-737,
# which picks up the remaining gap as a compiler/runtime question rather than
# re-litigating this budget. 150 leaves headroom over that measurement so the
# gate fails only on a real regression, not on ordinary run-to-run noise.
BUDGET_RATIO="${BUDGET_RATIO:-150}"
RUNS=5
QUIET=0

while [ $# -gt 0 ]; do
    case "$1" in
        --budget) BUDGET_RATIO="$2"; shift 2 ;;
        --runs) RUNS="$2"; shift 2 ;;
        --quiet) QUIET=1; shift ;;
        *) echo "usage: $0 [--budget N] [--runs N] [--quiet]" >&2; exit 2 ;;
    esac
done

# Fixed matrix: three level seeds already used in the bug doc's repro plus
# two more depths, small enough to run in a few seconds per side.
MATRIX=(
    "1 5"
    "1 10"
    "1 20"
    "2 5"
    "3 15"
)

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

log() { [ "$QUIET" = 1 ] || echo "$@" >&2; }

log "building oracle terrain_dump..."
engine=()
for f in "$brogue"/oracle/src/brogue/*.c; do
    [ "$(basename "$f")" = Architect.c ] || engine+=("$f")
done
cc -std=c99 -w -O1 -DBROGUE_EXTRA_VERSION='""' -DDATADIR=. \
    -I"$brogue/oracle/src/brogue" -I"$brogue/oracle/src/platform" -I"$brogue/oracle/src/variants" \
    "$brogue/check/terrain_dump.c" "$brogue/check/oracle_host.c" "${engine[@]}" \
    "$brogue"/oracle/src/variants/*.c "$brogue/oracle/src/platform/platformdependent.c" \
    "$brogue/oracle/src/platform/null-platform.c" \
    -o "$work/terrain_dump"

log "building port (release)..."
"$mfb" build "$brogue" >/dev/null

# median of N `/usr/bin/time -p CMD ARGS...` user-time readings, in seconds.
median_user_time() {
    local n="$1"; shift
    local -a times=()
    local i out u
    for ((i = 0; i < n; i++)); do
        out="$(TIMEFORMAT='%3U'; { time "$@" >/dev/null; } 2>&1 1>/dev/null)"
        times+=("$out")
    done
    printf '%s\n' "${times[@]}" | sort -n | awk -v n="$n" 'NR==int((n+1)/2){print; exit}'
}

total_port="0"
total_oracle="0"
for pair in "${MATRIX[@]}"; do
    read -r seed depth <<< "$pair"
    p="$(median_user_time "$RUNS" "$brogue/build/brogue.out" dig "$seed" "$depth")"
    o="$(median_user_time "$RUNS" "$work/terrain_dump" "$seed" "$depth")"
    log "seed=$seed depth=$depth  port(median)=${p}s  oracle(median)=${o}s"
    total_port="$(awk -v a="$total_port" -v b="$p" 'BEGIN{printf "%.4f", a+b}')"
    total_oracle="$(awk -v a="$total_oracle" -v b="$o" 'BEGIN{printf "%.4f", a+b}')"
done

# Guard against an oracle time at/under the shell timer's resolution floor
# (0.000s), which would make the ratio computation divide by zero.
if awk -v o="$total_oracle" 'BEGIN{exit !(o<=0)}'; then
    total_oracle="0.001"
fi

ratio="$(awk -v p="$total_port" -v o="$total_oracle" 'BEGIN{printf "%.1f", p/o}')"

log "building port (--debug) for allocation counters..."
"$mfb" build --debug "$brogue" >/dev/null
read -r seed0 depth0 <<< "${MATRIX[0]}"
debug_err="$work/debug.err"
"$brogue/build/brogue.out" dig "$seed0" "$depth0" > /dev/null 2> "$debug_err" || true
allocs="$(grep -m1 '^arena\.0\.alloc_calls ' "$debug_err" | awk '{print $2}')"
allocs="${allocs:-unknown}"

echo "port total user=${total_port}s  oracle total user=${total_oracle}s  ratio=${ratio}x  budget=${BUDGET_RATIO}x  (seed ${seed0} depth ${depth0} alloc_calls=${allocs})"

if awk -v r="$ratio" -v b="$BUDGET_RATIO" 'BEGIN{exit !(r>b)}'; then
    echo "FAIL: ratio ${ratio}x exceeds budget ${BUDGET_RATIO}x"
    exit 1
fi
echo "OK: ratio ${ratio}x within budget ${BUDGET_RATIO}x"
