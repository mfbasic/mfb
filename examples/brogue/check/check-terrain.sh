#!/usr/bin/env bash
# Compare the MFBASIC terrain generator against the oracle's own Architect.c.
#
# Builds check/terrain_dump.c against the whole oracle engine and the brogue
# project with mfb, then for every (level seed, depth) pair runs both and diffs
# the seven stage dumps (carve, loops, walls, lakes, filled, autogen,
# diagonals). Runs pairs in parallel; exits 1 if any pair differs, naming the
# first stage that does.
#
# Usage: check/check-terrain.sh [seedCount] [firstSeed]
#          seedCount level seeds (default 8) starting at firstSeed (default 1),
#          each at every depth 1..40
# Env:   MFB   the mfb binary (default: the repository's target/release/mfb)
#        JOBS  parallel pairs (default: the CPU count)

set -euo pipefail

here="$(cd "$(dirname "$0")/.." && pwd)"
repo="$(cd "$here/../.." && pwd)"
mfb="${MFB:-$repo/target/release/mfb}"
seed_count="${1:-8}"
first_seed="${2:-1}"
jobs="${JOBS:-$(getconf _NPROCESSORS_ONLN)}"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

oracle="$here/oracle/src"
engine=()
for f in "$oracle"/brogue/*.c; do
    [ "$(basename "$f")" = Architect.c ] || engine+=("$f")
done
cc -std=c99 -w -O1 -DBROGUE_EXTRA_VERSION='""' -DDATADIR=. \
    -I"$oracle/brogue" -I"$oracle/platform" -I"$oracle/variants" \
    "$here/check/terrain_dump.c" "$here/check/oracle_host.c" "${engine[@]}" \
    "$oracle"/variants/*.c "$oracle/platform/platformdependent.c" "$oracle/platform/null-platform.c" \
    -o "$work/terrain_dump"
"$mfb" build "$here" >/dev/null

compare_one() {
    local seed="$1" depth="$2" work="$3" port="$4"
    local c="$work/c.$seed.$depth" m="$work/m.$seed.$depth"
    "$work/terrain_dump" "$seed" "$depth" > "$c"
    if ! "$port" dig "$seed" "$depth" > "$m" 2> "$m.err"; then
        echo "seed $seed depth $depth: PORT FAILED: $(head -3 "$m.err" | tr '\n' ' ')"
        return 1
    fi
    if cmp -s "$c" "$m"; then
        rm -f "$c" "$m" "$m.err"
        return 0
    fi
    local line stage
    line="$(cmp "$c" "$m" | sed -E 's/.* line ([0-9]+).*/\1/')"
    stage="$(head -n "$line" "$c" | grep '^stage' | tail -1)"
    echo "seed $seed depth $depth: MISMATCH at line $line (in: $stage)"
    return 1
}
export -f compare_one

pairs=0
for ((s = first_seed; s < first_seed + seed_count; s++)); do
    for depth in $(seq 1 40); do
        echo "$s $depth"
        pairs=$((pairs + 1))
    done
done > "$work/pairs"

start=$(date +%s)
if xargs -P "$jobs" -L 1 bash -c 'compare_one "$0" "$1" "'"$work"'" "'"$here/build/brogue.out"'"' < "$work/pairs"; then
    echo "all $pairs (level seed, depth) pairs match ($(( $(date +%s) - start ))s, $jobs jobs)"
else
    echo "FAILED: see mismatches above"
    exit 1
fi
