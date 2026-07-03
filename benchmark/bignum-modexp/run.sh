#!/usr/bin/env bash
# Bignum modexp over the P-256 prime: base-2^28 limb lists, schoolbook multiply,
# and bit-serial binary long-division reduction — distilled from the
# pure-MFBASIC ECDSA experiment where this reduction made one scalar mult ~30s.
# Each reduction walks ~560 bits of the double-width product and allocates
# several short-lived MIXED-SIZE lists per bit, which hits the arena
# allocator's first-fit free-list walk (the still-open allocator half of
# bug-01, planning/old-plans/bug-01-arena-alloc-quadratic.md): cost grows
# quadratically in cumulative allocations (5/10/20 exponent bits measured
# 3.1s/21s/113s per run), so the exponent is capped at 6 bits (10 modmuls,
# ~500k allocations, ~4s) until the allocator is fixed — after which this
# should be near-instant and the exponent can grow back to 63 bits. All three
# implementations use the identical algorithm (Python deliberately avoids
# native pow()); each prints checksum 1181356819.
: "${BENCH_RUNS:=5}"
export BENCH_RUNS
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$here/../runner.sh"

bench_build_mfb "$here/mfb"
bench_build_c   "$here/c" bignum-modexp

echo "bignum-modexp — 6-bit modexp over P-256, base-2^28 limbs, bit-serial division:"
time_run "mfb"    "$MFB_OUT"
time_run "python" python3 "$here/python/main.py"
time_run "c -O0"  "$here/c/bignum-modexp-O0.out"
time_run "c -O2"  "$here/c/bignum-modexp-O2.out"
