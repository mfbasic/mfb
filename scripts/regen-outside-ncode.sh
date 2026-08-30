#!/usr/bin/env bash
# Regenerate the `.ncode` / `.ncodesum` goldens that live OUTSIDE
# `tests/byte-identity/` — `scripts/regen-ncodesum.sh` only sweeps that tree, so
# an intended codegen change leaves these stale and `artifact-gate.sh all` keeps
# reporting them (plan-99; the trap is recorded in `.ai/testing-gates.md`).
#
# Same contract as regen-ncodesum.sh: only EXISTING goldens are refreshed (a
# fixture's golden set never changes shape), each is rebuilt for the target named
# in its filename, and an `.app`-suffixed target builds with `--app`.
#
# Usage: scripts/regen-outside-ncode.sh <mfb-exe> [<host-target>]
set -u
MFB=${1:?usage: regen-outside-ncode.sh <mfb-exe> [host-target]}
HOST=${2:-macos-aarch64}
updated=0
missing=0
for golden in tests/*/*/*/golden/*.ncode tests/*/*/*/golden/*.ncodesum; do
  [ -f "$golden" ] || continue
  case "$golden" in tests/byte-identity/*) continue ;; esac
  fixturedir=$(dirname "$(dirname "$golden")")
  file=$(basename "$golden")
  ext=${file##*.}                     # ncode | ncodesum
  base=${file%."$ext"}                # <name>.<target>[.app]
  target=${base##*.}                  # <target> or "app"
  name=${base%."$target"}
  app=""
  if [ "$target" = "app" ]; then
    app="--app"
    target=${name##*.}
    name=${name%."$target"}
  fi
  targ=""
  [ "$target" = "$HOST" ] || targ="-target $target"
  # shellcheck disable=SC2086
  "$MFB" build -q -ncode $targ $app "$fixturedir" >/dev/null 2>&1
  actual="$fixturedir/$name.ncode"
  if [ -f "$actual" ]; then
    if [ "$ext" = "ncodesum" ]; then
      shasum -a 256 "$actual" | cut -d' ' -f1 > "$golden"
    else
      cp "$actual" "$golden"
    fi
    updated=$((updated + 1))
  else
    echo "MISSING actual for $golden (built '$actual')"
    missing=$((missing + 1))
  fi
done
echo "regen-outside-ncode: $updated golden(s) refreshed, $missing missing"
