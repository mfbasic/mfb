#!/usr/bin/env bash
# bug-387 byte-identity gate: compare the app-mode `-ncode` of the 3 app fixtures
# and (optionally) the full exe-oracle corpus for each target against the pre-fix
# baselines recorded in /tmp/bug387. Any nonzero diff means the change moved a byte
# — a failed change for this output-preserving refactor.
#
# usage: bug387-gate.sh <mfb-exe> [app|full]
#   app  (default): just the 3 app-fixture -ncode dumps × 4 targets (fast, ~30s)
#   full: also the exe-oracle console/library corpus per target (slow, ~15min)
set -u
MFB=$1; MODE=${2:-app}
ROOT=$(cd "$(dirname "$0")/.." && pwd); cd "$ROOT" || exit 2
BASE=/tmp/bug387
rc=0

echo "== app-mode -ncode gate =="
: > "$BASE/app-ncode-now.txt"
for fx in macos-app-mode-io macos-app-mode-term macos-app-mode-plumbing; do
  d=tests/syntax/app/$fx
  pkg=$(sed -n 's/.*"name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$d/project.json" | head -1)
  for tgt in macos-aarch64 linux-x86_64 linux-aarch64 windows-x86_64; do
    rm -f "$d/$pkg".ncode 2>/dev/null
    MFB_HOME=$(mktemp -d) "$MFB" build -q -ncode --app -target "$tgt" "$d" >/dev/null 2>&1
    if [ -f "$d/$pkg.ncode" ]; then
      echo "$fx $tgt $(shasum -a 256 "$d/$pkg.ncode" | cut -d' ' -f1)" >> "$BASE/app-ncode-now.txt"
    else
      echo "$fx $tgt BUILD_FAIL" >> "$BASE/app-ncode-now.txt"
    fi
    rm -f "$d/$pkg".ncode 2>/dev/null
  done
done
if diff -u "$BASE/app-ncode-base.txt" "$BASE/app-ncode-now.txt"; then
  echo "app-ncode: byte-identical"
else
  echo "app-ncode: DIFF (above)"; rc=1
fi

if [ "$MODE" = full ]; then
  for tgt in linux-x86_64 windows-x86_64 linux-riscv64 linux-aarch64; do
    echo "== exe-oracle $tgt =="
    if [ -f "$BASE/oracle-$tgt.txt" ]; then
      ./scripts/exe-oracle.sh "$MFB" "$tgt" compare "$BASE/oracle-$tgt.txt" || rc=1
    else
      echo "no baseline oracle-$tgt.txt — skipped"
    fi
  done
fi

[ "$rc" -eq 0 ] && echo "BUG387-GATE: PASS (byte-identical)" || echo "BUG387-GATE: FAIL"
exit $rc
