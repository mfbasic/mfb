#!/usr/bin/env bash
# Fast codegen gate: regenerate ONLY deterministic artifact dumps (no link/run)
# and diff against committed goldens. `mfb build -<x>` writes `$pkg.<ext>` with
# no target infix; the corresponding golden for native artifacts carries the
# target infix (the acceptance harness renames on move), so map accordingly.
set -u
MFB="$1"; REPO="$(pwd)"
host_arch="$(uname -m)"; case "$host_arch" in arm64) A=aarch64;; x86_64) A=x86_64;; *) A=$host_arch;; esac
case "$(uname -s)" in Darwin) TGT="macos-$A";; Linux) TGT="linux-$A";; *) TGT="unknown-$A";; esac
diffs=0; checked=0; ran=0
# Every project.json is a test at any depth, under tests/{syntax,rt-error,
# rt-behavior}/<feature>/* (plus the tests/acceptance app). Dirs without a
# golden/ are skipped below (e.g. behavioral acceptance suites).
while IFS= read -r pj; do
  td=$(dirname "$pj")
  rel="${td#"$REPO"/tests/}"; rel="${rel%/}"
  pkg=$(sed -n 's/.*"name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$td/project.json" | head -1)
  [ -n "$pkg" ] || continue
  g="$td/golden"; [ -d "$g" ] || continue
  flags="-ast -ir"
  [ -f "$g/$pkg.hex" ] && flags="$flags -br"
  [ -f "$g/$pkg.$TGT.nir" ] && flags="$flags -nir"
  [ -f "$g/$pkg.$TGT.nplan" ] && flags="$flags -nplan"
  [ -f "$g/$pkg.$TGT.nobj" ] && flags="$flags -nobj"
  [ -f "$g/$pkg.$TGT.ncode" ] && flags="$flags -ncode"
  [ -f "$g/$pkg.$TGT.mir" ] && flags="$flags -mir"
  ran=$((ran+1))
  rm -f "$td/$pkg".{ast,ir,hex,nir,nplan,nobj,ncode,mir} 2>/dev/null
  "$MFB" build $flags "$td" >/dev/null 2>&1
  # ext_actual (what mfb writes) : ext_golden (infix for native)
  for pair in "ast:ast" "ir:ir" "hex:hex" "nir:$TGT.nir" "nplan:$TGT.nplan" "nobj:$TGT.nobj" "ncode:$TGT.ncode" "mir:$TGT.mir"; do
    ae="${pair%%:*}"; ge="${pair##*:}"
    gf="$g/$pkg.$ge"; af="$td/$pkg.$ae"
    [ -f "$gf" ] || continue
    checked=$((checked+1))
    if [ ! -f "$af" ]; then echo "MISSING $rel/$pkg.$ge"; diffs=$((diffs+1)); continue; fi
    cmp -s "$gf" "$af" || { echo "DIFF $rel/$pkg.$ge"; diffs=$((diffs+1)); }
  done
  rm -f "$td/$pkg".{ast,ir,hex,nir,nplan,nobj,ncode,mir} 2>/dev/null
done < <(find "$REPO"/tests -name project.json | sort)
echo "artifact-gate: $ran tests, $checked golden(s) checked, $diffs diff(s)"
[ "$diffs" -eq 0 ]
