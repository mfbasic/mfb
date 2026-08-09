#!/bin/sh
# Regen the two rt-behavior CODEGEN goldens that churn under plan-86 F2.
set -e
ROOT="/Users/justinzaun/Development/mfb/.claude/worktrees/P-86"
MFB="$ROOT/target/release/mfb"
HOST="macos-aarch64"

# crypto-ec-valid: .ncodesum for 4 targets (no windows).
SRC="$ROOT/tests/rt-behavior/crypto/crypto-ec-valid"; PKG="crypto-ec-valid"; G="$SRC/golden"
TD="/tmp/rtb-cev"; rm -rf "$TD"; cp -R "$SRC" "$TD"; rm -rf "$TD/golden"
for t in linux-aarch64 linux-riscv64 linux-x86_64 macos-aarch64; do
  rm -f "$TD/$PKG.ncode"
  if [ "$t" = "$HOST" ]; then "$MFB" build -q -ncode "$TD" >/dev/null 2>&1
  else "$MFB" build -q -ncode -target "$t" "$TD" >/dev/null 2>&1; fi
  [ -f "$TD/$PKG.ncode" ] || { echo "NO NCODE cev/$t"; exit 1; }
  shasum -a 256 "$TD/$PKG.ncode" | cut -d" " -f1 > "$G/$PKG.$t.ncodesum"
done
echo "regenerated crypto-ec-valid"

# func_map_getor_hash_probe: full .ncode dump, host only.
SRC="$ROOT/tests/rt-behavior/collections/func_map_getor_hash_probe"; PKG="func_map_getor_hash_probe"; G="$SRC/golden"
TD="/tmp/rtb-mgh"; rm -rf "$TD"; cp -R "$SRC" "$TD"; rm -rf "$TD/golden"
rm -f "$TD/$PKG.ncode"
"$MFB" build -q -ncode "$TD" >/dev/null 2>&1
[ -f "$TD/$PKG.ncode" ] || { echo "NO NCODE mgh"; exit 1; }
cp "$TD/$PKG.ncode" "$G/$PKG.$HOST.ncode"
echo "regenerated func_map_getor_hash_probe"
