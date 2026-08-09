#!/bin/sh
# Regenerate collections_codegen_cover_rt .ncodesum byte-identity goldens after
# plan-86 D3 (native merge). Host = macos-aarch64.
set -e
ROOT="/Users/justinzaun/Development/mfb/.claude/worktrees/P-86"
MFB="$ROOT/target/release/mfb"
SRC="$ROOT/tests/byte-identity/collections"
G="$SRC/golden"
PKG="collections_codegen_cover_rt"
HOST="macos-aarch64"
TD="/tmp/coll-regen"
rm -rf "$TD"
cp -R "$SRC" "$TD"
rm -rf "$TD/golden"
for t in linux-aarch64 linux-riscv64 linux-x86_64 macos-aarch64 windows-x86_64; do
  rm -f "$TD/$PKG.ncode"
  if [ "$t" = "$HOST" ]; then
    "$MFB" build -q -ncode "$TD" >/dev/null 2>&1
  else
    "$MFB" build -q -ncode -target "$t" "$TD" >/dev/null 2>&1
  fi
  if [ ! -f "$TD/$PKG.ncode" ]; then
    echo "NO NCODE for $t"; ls "$TD"; exit 1
  fi
  h=$(shasum -a 256 "$TD/$PKG.ncode" | cut -d" " -f1)
  old=$(cat "$G/$PKG.$t.ncodesum")
  printf '%s\n' "$h" > "$G/$PKG.$t.ncodesum"
  echo "$t: $old -> $h"
done
