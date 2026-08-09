#!/usr/bin/env bash
# Compare linux-aarch64 (and riscv64) .out bytes between the clean-main mfb and the
# plan-85 mfb, for libc-using fixtures (which exercise the staging+elision). Byte
# identity => the elision removed exactly the staging no-ops and nothing pre-existing.
set -u
MINE=target/release/mfb
CLEAN=/tmp/p85-clean/target/release/mfb
TARGET=${1:-linux-aarch64}
shift || true
pass=0; fail=0
for dir in "$@"; do
  rm -rf "$dir/build" 2>/dev/null
  MFB_TARGET="$TARGET" "$MINE" build -q -target "$TARGET" "$dir" >/dev/null 2>&1
  mine_sha=$(find "$dir/build" -name '*.out' -exec shasum -a 256 {} \; 2>/dev/null | awk '{print $1}' | sort | tr '\n' ' ')
  rm -rf "$dir/build" 2>/dev/null
  MFB_TARGET="$TARGET" "$CLEAN" build -q -target "$TARGET" "$dir" >/dev/null 2>&1
  clean_sha=$(find "$dir/build" -name '*.out' -exec shasum -a 256 {} \; 2>/dev/null | awk '{print $1}' | sort | tr '\n' ' ')
  rm -rf "$dir/build" 2>/dev/null
  if [ -n "$mine_sha" ] && [ "$mine_sha" = "$clean_sha" ]; then
    echo "IDENTICAL  $dir"; pass=$((pass+1))
  else
    echo "DIFFERS    $dir"; echo "  mine=$mine_sha"; echo "  clean=$clean_sha"; fail=$((fail+1))
  fi
done
echo "=== $TARGET byte-identity: identical=$pass differs=$fail ==="
