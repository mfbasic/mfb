#!/usr/bin/env bash
# plan-88: regenerate the `.ncodesum` goldens in-place after an intended codegen
# change (artifact-gate has no update mode; test-accept skips byte-identity). For
# each existing `<pkg>.<target>[.app].ncodesum` golden, rebuild that fixture's
# `-ncode` dump for that target and overwrite the golden with the fresh sha256.
# Only EXISTING goldens are refreshed (shape unchanged). No new goldens.
#
# plan-118-C: this used to walk only `tests/byte-identity/*/golden/`, so the
# `.ncodesum` goldens elsewhere under tests/ — `rt-behavior/crypto/crypto-ec-valid`
# and the two `syntax/app/macos-app-mode-*` fixtures, whose sums are the
# cross-target ones the acceptance harness cannot produce — had to be
# hand-regenerated after every codegen change, and were repeatedly forgotten. It
# now walks every `*/golden/*.ncodesum` under tests/, and understands the
# `<target>.app` infix (`mfb build --app`) the gate already splits the same way.
#
# Usage: scripts/regen-ncodesum.sh <mfb-exe> [<host-target>]
set -u
MFB=${1:?usage: regen-ncodesum.sh <mfb-exe> [host-target]}
HOST=${2:-macos-aarch64}
updated=0
missing=0
while IFS= read -r gsum; do
  [ -f "$gsum" ] || continue
  fixturedir=$(dirname "$(dirname "$gsum")")   # tests/<...>/<fixture>
  base=$(basename "$gsum" .ncodesum)            # <name>.<target>[.app]
  target=${base##*.}                            # <target> or "app"
  name=${base%."$target"}
  # A `<target>.app` infix is an app-mode build, not a distinct target — the
  # same split `artifact-gate.sh` performs, kept in step with it.
  mode=""
  if [ "$target" = "app" ]; then
    mode="--app"
    target=${name##*.}
    name=${name%."$target"}
  fi
  targ=""
  [ "$target" = "$HOST" ] || targ="-target $target"
  # shellcheck disable=SC2086
  "$MFB" build -q -ncode $mode $targ "$fixturedir" >/dev/null 2>&1
  af="$fixturedir/$name.ncode"
  if [ -f "$af" ]; then
    shasum -a 256 "$af" | cut -d' ' -f1 > "$gsum"
    updated=$((updated + 1))
  else
    echo "MISSING actual for $gsum (built '$af')"
    missing=$((missing + 1))
  fi
done <<EOF
$(find tests -type f -name '*.ncodesum' -path '*/golden/*' | sort)
EOF
echo "regen-ncodesum: $updated golden(s) refreshed, $missing missing"
