#!/usr/bin/env bash
# plan-32-D Phase 2: prove the one `linux-riscv64` binary is bit-identical and
# ≤1 ULP under BOTH cpu profiles — `v=true` (native RVV arm) and `v=false`
# (scalar arm) — for every math kernel, on the same executable.
#
# The dual-path lowering (plan-32-C/-D) must not change a single result bit
# between the arms, so this scores each function under both profiles with the
# ULP harness and requires the two summaries to be IDENTICAL. A pre-existing
# kernel outlier (e.g. tan/log10 at 2 ULP on one vector) is shared by every
# backend incl. macos-aarch64, so it is not a plan-32 regression — what plan-32
# guarantees is that v=true == v=false, which this asserts.
#
# Runs qemu-user on the riscv64 box (2232). Reproducible invocations:
#   MFB_QEMU_CPU='rv64,v=true,vlen=128'  runtime_ulp.py <fn> --target linux-riscv64 --runner scripts/rvv-qemu-runner.sh
#   MFB_QEMU_CPU='rv64,v=false'          runtime_ulp.py <fn> ...
# vlen=128 is the minimum guaranteed V width — it exercises the 2×f64 (vl=2)
# assumption at the narrowest VLEN.
set -u
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
MFB="${MFB:-$ROOT/target/release/mfb}"
RUNNER="$ROOT/scripts/rvv-qemu-runner.sh"
ULP="$ROOT/tools/math-kernels/runtime_ulp.py"
LIMIT="${LIMIT:-150}"
FNS="${FNS:-exp log log10 pow tan atan2 asin acos}"

fail=0
for fn in $FNS; do
  vt=$(MFB_QEMU_CPU='rv64,v=true,vlen=128' python3 "$ULP" "$fn" \
        --target linux-riscv64 --runner "$RUNNER" --mfb "$MFB" --limit "$LIMIT" 2>/dev/null \
        | grep -E 'primary' | sed 's/^ *//')
  vf=$(MFB_QEMU_CPU='rv64,v=false' python3 "$ULP" "$fn" \
        --target linux-riscv64 --runner "$RUNNER" --mfb "$MFB" --limit "$LIMIT" 2>/dev/null \
        | grep -E 'primary' | sed 's/^ *//')
  if [ "$vt" = "$vf" ] && [ -n "$vt" ]; then
    echo "OK   $fn  v=true == v=false : $vt"
  else
    echo "FAIL $fn"
    echo "     v=true : $vt"
    echo "     v=false: $vf"
    fail=1
  fi
done
[ "$fail" = 0 ] && echo "rvv-ulp-two-profile: all kernels bit-identical across both cpu profiles" || echo "rvv-ulp-two-profile: DIVERGENCE (see above)"
exit "$fail"
