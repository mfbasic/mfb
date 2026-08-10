#!/usr/bin/env bash
# Regenerate the native artifact goldens (.ncode/.mir + their .ncodesum) for the
# fixture dirs passed as args, using the current compiler. Mirrors
# artifact-gate.sh's Pass-2 build (target + app-mode parsed from the golden
# filename) but WRITES each golden instead of diffing it.
#
# SCOPED to the fixture dirs on the command line (bounded blast radius) — it
# never sweeps the tree. Used once to re-baseline the rt-behavior/syntax goldens
# that plan-88-B's allocation unification (the x0-optimised OOM emit becoming an
# explicit `mov_imm 77010001`) shifted but never re-baselined (commit 8a336ea95
# "goldens pending"; B re-baselined only tests/byte-identity/*.ncodesum). The
# change is byte-only — the runtime error code 77010001 (ErrOutOfMemory) is
# unchanged — so this brings the goldens in step with the intended, rt-error-
# verified codegen. Only native kinds are touched; host front-end dumps
# (ast/ir/hex) are left alone.
#
# Usage: scripts/regen-rt-goldens.sh <mfb-exe> <fixture-dir>...
set -u
MFB=${1:?usage: regen-rt-goldens.sh <mfb-exe> <fixture-dir>...}
shift
host_arch="$(uname -m)"; case "$host_arch" in arm64) A=aarch64;; x86_64) A=x86_64;; *) A=$host_arch;; esac
case "$(uname -s)" in Darwin) HOST_TGT="macos-$A";; Linux) HOST_TGT="linux-$A";; *) HOST_TGT="unknown-$A";; esac
updated=0
for td in "$@"; do
  pj="$td/project.json"; [ -f "$pj" ] || { echo "SKIP no project.json: $td"; continue; }
  pkg=$(sed -n 's/.*"name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$pj" | head -1)
  g="$td/golden"; [ -d "$g" ] || continue
  for gf in "$g/$pkg."*; do
    [ -f "$gf" ] || continue
    base="${gf##*/}"                 # <pkg>.<t>.<ext>[sum]
    rest="${base#"$pkg."}"           # <t>.<ext>[sum]
    ext="${rest##*.}"                # ext or extsum
    t="${rest%.*}"                   # <t> (may carry an `.app` mode suffix)
    issum=0; k="$ext"
    case "$ext" in *sum) issum=1; k="${ext%sum}" ;; esac
    case " nir nplan nobj ncode mir " in *" $k "*) ;; *) continue ;; esac  # native kinds only
    mode=""; tt="$t"
    case "$t" in *.app) mode="--app"; tt="${t%.app}" ;; esac
    targ=""; [ "$tt" = "$HOST_TGT" ] || targ="-target $tt"
    rm -f "$td/$pkg".{nir,nplan,nobj,ncode,mir} 2>/dev/null
    # shellcheck disable=SC2086
    "$MFB" build -q "-$k" $targ $mode "$td" >/dev/null 2>&1
    af="$td/$pkg.$k"
    if [ ! -f "$af" ]; then echo "MISSING actual for $gf (built '$af')"; continue; fi
    if [ "$issum" = 1 ]; then
      shasum -a 256 "$af" | cut -d' ' -f1 > "$gf"
    else
      cp "$af" "$gf"
    fi
    updated=$((updated + 1)); echo "updated $gf"
  done
  rm -f "$td/$pkg".{nir,nplan,nobj,ncode,mir} 2>/dev/null
done
echo "regen-rt-goldens: $updated golden(s) refreshed"
