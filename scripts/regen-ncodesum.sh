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
#
# bug-513: this script relies on word-splitting `$targ` into `-target <t>`. zsh
# does not word-split, so under `zsh scripts/regen-ncodesum.sh` every
# cross-target build is handed `-target windows-x86_64` as ONE argument and
# fails. The file is not always executable in a fresh worktree, which is exactly
# what tempts `zsh scripts/...` — so re-exec under bash rather than trusting the
# shebang to have been honoured.
if [ -n "${ZSH_VERSION:-}" ]; then
  exec bash "$0" "$@"
fi
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
  af="$fixturedir/$name.ncode"
  # The dump path carries no target infix, so every target of a fixture writes
  # the SAME file. Removing it first, and refusing to hash unless THIS target's
  # build succeeded, is what keeps a failed build from re-hashing the previously
  # built target's dump into this target's golden — which silently ratifies a
  # wrong sum (observed: the macos-aarch64 sum written into 26 windows-x86_64
  # goldens, all of which had been correct).
  rm -f "$af"
  # shellcheck disable=SC2086
  if ! "$MFB" build -q -ncode $mode $targ "$fixturedir" >/dev/null 2>&1; then
    echo "BUILD FAILED for $gsum ($target${mode:+ }$mode) — golden left unchanged"
    missing=$((missing + 1))
    continue
  fi
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
