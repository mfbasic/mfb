#!/usr/bin/env bash
# plan-88: regenerate the byte-identity .ncodesum goldens in-place after an
# intended codegen change (artifact-gate has no update mode; test-accept skips
# byte-identity). For each existing `<pkg>.<target>.ncodesum` golden, rebuild the
# fixture's `-ncode` dump for that target and overwrite the golden with the fresh
# sha256. Only EXISTING goldens are refreshed (shape unchanged). No new goldens.
#
# Usage: scripts/regen-ncodesum.sh <mfb-exe> [<host-target>]
set -u
MFB=${1:?usage: regen-ncodesum.sh <mfb-exe> [host-target]}
HOST=${2:-macos-aarch64}
updated=0
missing=0
for gsum in tests/byte-identity/*/golden/*.ncodesum; do
  [ -f "$gsum" ] || continue
  fixturedir=$(dirname "$(dirname "$gsum")")   # tests/byte-identity/<pkg>
  base=$(basename "$gsum" .ncodesum)            # <name>.<target>
  target=${base##*.}                            # <target>
  name=${base%."$target"}                       # <name> (project/pkg name)
  targ=""
  [ "$target" = "$HOST" ] || targ="-target $target"
  # shellcheck disable=SC2086
  "$MFB" build -q -ncode $targ "$fixturedir" >/dev/null 2>&1
  af="$fixturedir/$name.ncode"
  if [ -f "$af" ]; then
    shasum -a 256 "$af" | cut -d' ' -f1 > "$gsum"
    updated=$((updated + 1))
  else
    echo "MISSING actual for $gsum (built '$af')"
    missing=$((missing + 1))
  fi
done
echo "regen-ncodesum: $updated golden(s) refreshed, $missing missing"
