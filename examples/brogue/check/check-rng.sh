#!/usr/bin/env bash
# Compare the MFBASIC RNG port against the oracle's own Math.c.
#
# Builds check/rng_dump.c against oracle/src and the brogue project with mfb,
# runs both over a fixed seed list, and diffs their output line for line.
# Exits 0 when every seed matches, 1 on the first mismatch (printing the diff).
#
# Usage: check/check-rng.sh [count]      (count defaults to 20000 calls per seed)
# Env:   MFB   the mfb binary (default: the repository's target/release/mfb)

set -euo pipefail

here="$(cd "$(dirname "$0")/.." && pwd)"
repo="$(cd "$here/../.." && pwd)"
mfb="${MFB:-$repo/target/release/mfb}"
count="${1:-20000}"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

cc -std=c99 -O1 \
    -I"$here/oracle/src/brogue" -I"$here/oracle/src/platform" -I"$here/oracle/src/variants" \
    "$here/check/rng_dump.c" "$here/oracle/src/brogue/Math.c" "$here/oracle/src/brogue/GlobalsBase.c" \
    -o "$work/rng_dump"
"$mfb" build "$here" >/dev/null

# c_seed:mfb_seed. The port takes a seed as a signed 64-bit pattern, so a seed
# above 2^63-1 is passed to it as the negative Integer with the same bits.
# Seeds below 2^32 take Brogue's backward-compatible path; the rest its
# 64-bit path.
seeds=(
    1:1 2:2 12345:12345 99999999:99999999 4294967295:4294967295
    4294967296:4294967296 1234567890123:1234567890123
    9223372036854775807:9223372036854775807
    9223372036854775808:-9223372036854775808
    18446744073709551615:-1
)

for pair in "${seeds[@]}"; do
    c_seed="${pair%%:*}"
    mfb_seed="${pair##*:}"
    if ! diff <("$work/rng_dump" "$c_seed" "$count") \
              <("$here/build/brogue.out" rng "$mfb_seed" "$count") > "$work/diff"; then
        echo "seed $c_seed: MISMATCH"
        head -20 "$work/diff"
        exit 1
    fi
    echo "seed $c_seed: match ($count calls)"
done
