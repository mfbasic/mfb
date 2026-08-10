#!/usr/bin/env bash
# Regenerate every .ncodesum / .app.ncodesum golden under tests/ for the plan-85
# token-vocabulary rename (%arg->%argMFB, %ret->%retMFB, %sysarg->%argSys). The
# .ncode dump embeds the neutral token names in its parameter-location metadata, so
# each codegen-cover golden's sha changes on every target even where the machine
# bytes (.out) are byte-identical (proven by the exe-oracle: aarch64/riscv64 fully
# byte-identical). This rebuilds each fixture per target/mode and rewrites the sha.
# Idempotent — an unchanged golden gets the same sha (git no-op). Mirrors the
# target/`--app` decoding in scripts/artifact-gate.sh.
set -u
cd /Users/justinzaun/Development/mfb/.claude/worktrees/P-85 || exit 2
MFB=target/release/mfb
HOST_TGT=macos-aarch64
n=0
while IFS= read -r gsum; do
  [ -f "$gsum" ] || continue
  base=$(basename "$gsum")                 # <pkg>.<target>[.app].ncodesum
  fixdir=$(dirname "$(dirname "$gsum")")     # tests/.../<name>
  rest="${base%.ncodesum}"                   # <pkg>.<target>[.app]
  mode=""
  case "$rest" in *.app) mode="--app"; rest="${rest%.app}" ;; esac
  target="${rest##*.}"                        # <target>
  pkg="${rest%.$target}"                      # <pkg>
  targ=""
  [ "$target" = "$HOST_TGT" ] || targ="-target $target"
  rm -f "$fixdir/$pkg".ncode 2>/dev/null
  # shellcheck disable=SC2086
  MFB_TARGET="$target" "$MFB" build -q -ncode $targ $mode "$fixdir" >/dev/null 2>&1
  af="$fixdir/$pkg.ncode"
  if [ -f "$af" ]; then
    printf '%s\n' "$(shasum -a 256 "$af" | cut -d" " -f1)" > "$gsum"
    n=$((n+1))
  else
    echo "WARN: no .ncode produced for $pkg ($target${mode:+ $mode})"
  fi
  rm -f "$fixdir/$pkg".ncode 2>/dev/null
done < <(find tests -name '*.ncodesum' | sort)
echo "=== regenerated $n .ncodesum goldens ==="
