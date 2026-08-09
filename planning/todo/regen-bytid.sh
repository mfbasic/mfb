#!/bin/sh
# General byte-identity .ncodesum regen (plan-86 F2 broad churn). Pass builtin
# names as args (e.g. `sh regen-bytid.sh strings collections json`); regenerates
# all 5 targets' .ncodesum for each `tests/byte-identity/<builtin>/`. Host=macos-aarch64.
set -e
ROOT="/Users/justinzaun/Development/mfb/.claude/worktrees/P-86"
MFB="$ROOT/target/release/mfb"
HOST="macos-aarch64"
for b in "$@"; do
  SRC="$ROOT/tests/byte-identity/$b"
  G="$SRC/golden"
  # PKG = the *_codegen_cover_rt basename from project.json.
  PKG=$(grep '"name"' "$SRC/project.json" | head -1 | sed 's/.*"name"[^"]*"\([^"]*\)".*/\1/')
  TD="/tmp/bytid-$b"
  rm -rf "$TD"; cp -R "$SRC" "$TD"; rm -rf "$TD/golden"
  for t in linux-aarch64 linux-riscv64 linux-x86_64 macos-aarch64 windows-x86_64; do
    [ -f "$G/$PKG.$t.ncodesum" ] || continue
    rm -f "$TD/$PKG.ncode"
    if [ "$t" = "$HOST" ]; then
      "$MFB" build -q -ncode "$TD" >/dev/null 2>&1
    else
      "$MFB" build -q -ncode -target "$t" "$TD" >/dev/null 2>&1
    fi
    [ -f "$TD/$PKG.ncode" ] || { echo "NO NCODE $b/$t"; exit 1; }
    shasum -a 256 "$TD/$PKG.ncode" | cut -d" " -f1 > "$G/$PKG.$t.ncodesum"
  done
  echo "regenerated $b ($PKG)"
done
