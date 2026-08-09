#!/usr/bin/env bash
# Full-corpus byte-identity gate for plan-85: record clean-main baselines with the
# detached clean binary, then compare the plan-85 binary, for the byte-identical
# targets (linux-aarch64, linux-riscv64). Serial (exe-oracle shares build dirs).
set -u
cd /Users/justinzaun/Development/mfb/.claude/worktrees/P-85 || exit 2
MINE=target/release/mfb
CLEAN=/tmp/p85-clean/target/release/mfb
for T in linux-aarch64 linux-riscv64; do
  echo "=== $T: recording clean-main baseline ==="
  bash scripts/exe-oracle.sh "$CLEAN" "$T" record "/tmp/p85-base-$T.txt" 2>&1 | tail -1
  echo "=== $T: comparing plan-85 binary ==="
  if bash scripts/exe-oracle.sh "$MINE" "$T" compare "/tmp/p85-base-$T.txt" 2>&1 | tail -1; then
    echo "RESULT $T: BYTE-IDENTICAL"
  else
    echo "RESULT $T: DIFFERS -- see /tmp/exe-oracle.diff"
    cp /tmp/exe-oracle.diff "/tmp/p85-diff-$T.txt" 2>/dev/null
  fi
done
echo "=== P85-FULL-BYTE-GATE DONE ==="
