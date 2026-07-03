#!/usr/bin/env bash
# Bignum modexp over the P-256 prime: base-2^28 limb lists, schoolbook multiply,
# and bit-serial binary long-division reduction — distilled from the
# pure-MFBASIC ECDSA experiment. Each reduction walks ~560 bits of the
# double-width product and allocates several short-lived MIXED-SIZE lists per
# bit — the workload that exposed bug-01's fourth value-semantic collection
# leak (the value-append singleton) and drove the allocator-01 quick-bin +
# designated-victim redesign. Both fixed: the full 63-bit exponent
# (~5M allocations) now runs in ~0.1 s with flat memory, where it previously
# extrapolated to ~19 minutes. All three implementations use the identical
# algorithm (Python deliberately avoids native pow()); each prints checksum
# 1627198717.
: "${BENCH_RUNS:=100}"
export BENCH_RUNS
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$here/../runner.sh"

bench_build_mfb "$here/mfb"
bench_build_c   "$here/c" bignum-modexp

echo "bignum-modexp — 63-bit modexp over P-256, base-2^28 limbs, bit-serial division:"
time_run "mfb"    "$MFB_OUT"
time_run "python" python3 "$here/python/main.py"
time_run "c -O0"  "$here/c/bignum-modexp-O0.out"
time_run "c -O2"  "$here/c/bignum-modexp-O2.out"
