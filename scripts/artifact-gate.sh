#!/usr/bin/env bash
# Fast codegen gate: regenerate ONLY deterministic artifact dumps (no link/run)
# and diff against committed goldens. `mfb build -<x>` writes `$pkg.<ext>` with
# no target infix; the corresponding golden for native artifacts carries the
# target infix (the acceptance harness renames on move), so map accordingly.
#
# MULTI-TARGET. A fixture's native goldens are discovered by filename, so a
# `$pkg.linux-aarch64.ncode` golden is regenerated with `-target linux-aarch64`
# even on a macOS host. Without this the gate could only ever see the host
# backend, and the Linux-only code paths — `audio/alsa`, `tls/openssl`, and every
# `linux_*` target module — had no byte-identity coverage at all on the machine
# where the work actually happens.
set -u
MFB="$1"; REPO="$(pwd)"
host_arch="$(uname -m)"; case "$host_arch" in arm64) A=aarch64;; x86_64) A=x86_64;; *) A=$host_arch;; esac
case "$(uname -s)" in Darwin) HOST_TGT="macos-$A";; Linux) HOST_TGT="linux-$A";; *) HOST_TGT="unknown-$A";; esac
diffs=0; checked=0; ran=0; builds=0

# The native artifact extensions, in the order they are reported.
NATIVE_EXTS="nir nplan nobj ncode mir"

while IFS= read -r pj; do
  td=$(dirname "$pj")
  rel="${td#"$REPO"/tests/}"; rel="${rel%/}"
  pkg=$(sed -n 's/.*"name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$td/project.json" | head -1)
  [ -n "$pkg" ] || continue
  g="$td/golden"; [ -d "$g" ] || continue
  ran=$((ran+1))

  # Which targets does this fixture carry native goldens for? Derived from the
  # golden filenames (`$pkg.<target>.<ext>`), so adding a golden for a new target
  # is all it takes to have the gate cover it.
  targets=""
  for ext in $NATIVE_EXTS; do
    for suffix in "$ext" "${ext}sum"; do
      for gf in "$g/$pkg."*".$suffix"; do
        [ -f "$gf" ] || continue
        base="${gf##*/}"; t="${base#"$pkg."}"; t="${t%".$suffix"}"
        case " $targets " in *" $t "*) ;; *) targets="$targets $t" ;; esac
      done
    done
  done

  # Pass 1: target-independent dumps (AST / IR / package binary). Built once,
  # for the host, since none of them depend on the backend.
  flags="-ast -ir"
  [ -f "$g/$pkg.hex" ] && flags="$flags -br"
  rm -f "$td/$pkg".{ast,ir,hex,nir,nplan,nobj,ncode,mir} 2>/dev/null
  "$MFB" build $flags "$td" >/dev/null 2>&1
  builds=$((builds+1))
  for pair in "ast:ast" "ir:ir" "hex:hex"; do
    ae="${pair%%:*}"; ge="${pair##*:}"
    gf="$g/$pkg.$ge"; af="$td/$pkg.$ae"
    [ -f "$gf" ] || continue
    checked=$((checked+1))
    if [ ! -f "$af" ]; then echo "MISSING $rel/$pkg.$ge"; diffs=$((diffs+1)); continue; fi
    cmp -s "$gf" "$af" || { echo "DIFF $rel/$pkg.$ge"; diffs=$((diffs+1)); }
  done
  rm -f "$td/$pkg".{ast,ir,hex,nir,nplan,nobj,ncode,mir} 2>/dev/null

  # Pass 2: one build per target that has native goldens.
  for t in $targets; do
    tflags=""
    for ext in $NATIVE_EXTS; do
      { [ -f "$g/$pkg.$t.$ext" ] || [ -f "$g/$pkg.$t.${ext}sum" ]; } && tflags="$tflags -$ext"
    done
    [ -n "$tflags" ] || continue
    # A `<target>.app` infix is an app-mode build, not a distinct target
    # (`macos_app_mode_term.macos-aarch64.app.ncode`). Split the mode off the
    # target before deciding whether a `-target` flag is needed.
    mode=""; tt="$t"
    case "$t" in *.app) mode="--app"; tt="${t%.app}" ;; esac
    targ=""
    [ "$tt" = "$HOST_TGT" ] || targ="-target $tt"
    rm -f "$td/$pkg".{nir,nplan,nobj,ncode,mir} 2>/dev/null
    # shellcheck disable=SC2086
    "$MFB" build $tflags $targ $mode "$td" >/dev/null 2>&1
    builds=$((builds+1))
    for ext in $NATIVE_EXTS; do
      af="$td/$pkg.$ext"
      # A `.<ext>sum` golden holds the sha256 of the dump instead of the dump.
      # Same byte-identity signal; the dumps for the runtime-heavy backends run
      # to tens of megabytes each and cannot be committed. On a failure,
      # regenerate the dump locally and diff it by hand.
      gsum="$g/$pkg.$t.${ext}sum"
      if [ -f "$gsum" ]; then
        checked=$((checked+1))
        if [ ! -f "$af" ]; then echo "MISSING $rel/$pkg.$t.$ext"; diffs=$((diffs+1));
        else
          want=$(cut -d" " -f1 <"$gsum")
          got=$(shasum -a 256 "$af" | cut -d" " -f1)
          [ "$want" = "$got" ] || { echo "DIFF $rel/$pkg.$t.$ext (sha256)"; diffs=$((diffs+1)); }
        fi
      fi
      gf="$g/$pkg.$t.$ext"
      [ -f "$gf" ] || continue
      checked=$((checked+1))
      if [ ! -f "$af" ]; then echo "MISSING $rel/$pkg.$t.$ext"; diffs=$((diffs+1)); continue; fi
      cmp -s "$gf" "$af" || { echo "DIFF $rel/$pkg.$t.$ext"; diffs=$((diffs+1)); }
    done
    rm -f "$td/$pkg".{nir,nplan,nobj,ncode,mir} 2>/dev/null
  done
done < <(find "$REPO"/tests -name project.json | sort)
echo "artifact-gate: $ran tests, $builds build(s), $checked golden(s) checked, $diffs diff(s)"
[ "$diffs" -eq 0 ]
