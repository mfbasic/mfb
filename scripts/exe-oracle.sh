#!/usr/bin/env bash
# bug-387 full-executable byte-identity oracle.
#
# The artifact-gate covers only the package `.ncode`/`.nobj`; bug-85 proved the
# entry stub and runtime-helper bodies (linked per-executable, NOT in the package
# object) are exactly where a token-audit miss becomes a silent crash. This gate
# cross-builds every executable-producing fixture for a target and records the
# sha256 of each produced `.out`, so a later build can be diffed byte-for-byte.
#
# usage:
#   exe-oracle.sh <mfb-exe> <target> record  <manifest-file>
#   exe-oracle.sh <mfb-exe> <target> compare <manifest-file>
set -u
EXE=$1; TARGET=$2; MODE=$3; MANIFEST=$4
ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT" || exit 2

export MFB_HOME
MFB_HOME=$(mktemp -d)
trap 'rm -rf "$MFB_HOME"' EXIT

tmp=$(mktemp)
built=0; fixtures=0
while IFS= read -r pj; do
  td=$(dirname "$pj")
  rel="${td#"$ROOT"/}"
  fixtures=$((fixtures+1))
  rm -rf "$td/build" 2>/dev/null
  out=$(MFB_TARGET="$TARGET" "$EXE" build -q -target "$TARGET" "$rel" 2>/dev/null)
  # Collect every produced executable path this fixture reported.
  while IFS= read -r p; do
    [ -n "$p" ] || continue
    [ -f "$td/$p" ] || { [ -f "$p" ] && td_p="$p" || continue; }
    fp="$td/$p"; [ -f "$fp" ] || fp="$p"
    h=$(shasum -a 256 "$fp" | awk '{print $1}')
    # Key by fixture-relative executable name so it is host-path independent.
    echo "$rel/$(basename "$p") $h" >>"$tmp"
    built=$((built+1))
  done < <(printf '%s\n' "$out" | sed -n 's/^Wrote executable to //p')
  rm -rf "$td/build" 2>/dev/null
  # `mfb build` writes a compiled `<pkg>.mfp` at the fixture root for package
  # fixtures; tracked `.mfp` only ever live under `golden/` or `packages/`
  # subdirs, so removing the fixture-root ones (maxdepth 1) leaves the tree clean
  # without touching any committed file.
  find "$td" -maxdepth 1 -name '*.mfp' -delete 2>/dev/null
done < <(find "$ROOT/tests" -name project.json | sort)

sort "$tmp" -o "$tmp"
echo "fixtures=$fixtures executables=$built target=$TARGET" >&2

if [ "$MODE" = record ]; then
  cp "$tmp" "$MANIFEST"
  echo "recorded $built executable hashes -> $MANIFEST" >&2
  rm -f "$tmp"
  exit 0
fi

# compare
if [ ! -f "$MANIFEST" ]; then echo "no manifest $MANIFEST" >&2; exit 2; fi
if diff -u "$MANIFEST" "$tmp" >/tmp/exe-oracle.diff 2>&1; then
  echo "OK: $built executables byte-identical for $TARGET" >&2
  rm -f "$tmp"
  exit 0
else
  echo "DIFF for $TARGET (see /tmp/exe-oracle.diff):" >&2
  grep -E '^[-+]' /tmp/exe-oracle.diff | grep -vE '^[-+]{3}' | head -60 >&2
  rm -f "$tmp"
  exit 1
fi
