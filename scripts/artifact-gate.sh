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
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO="$(pwd)"
BYTE_ID="$REPO/tests/byte-identity"

# Usage: <mfb-exe> <builtin|all>. The second positional is a *selector*: a single
# `tests/byte-identity/<builtin>` name to scope the run to one built-in, or `all`
# to sweep every fixture under tests/ (the full gate). A missing selector prints
# usage and runs nothing, so a bare invocation can never be mistaken for a pass.
usage() {
  local builtins
  builtins=$(cd "$BYTE_ID" 2>/dev/null && for d in */; do [ -f "$d/project.json" ] && printf '%s ' "${d%/}"; done)
  cat >&2 <<EOF
Usage: $(basename "$0") <mfb-exe> <builtin|all>

Execution-free byte-identity gate: regenerate deterministic codegen artifact
dumps and diff them against committed goldens.

  <mfb-exe>   path to the mfb binary (e.g. target/release/mfb)
  <builtin>   run only tests/byte-identity/<builtin>/*
  all         run every fixture under tests/

Available builtins:
  ${builtins:-<none found>}
EOF
}

if [ "$#" -lt 2 ]; then
  usage
  exit 2
fi

MFB="$1"; SEL="$2"

if [ "$SEL" = "all" ]; then
  FIND_ROOT="$REPO/tests"
else
  FIND_ROOT="$BYTE_ID/$SEL"
  if [ ! -d "$FIND_ROOT" ]; then
    echo "Unknown builtin '$SEL'." >&2
    usage
    exit 2
  fi
fi

# Refuse to run concurrently with another artifact-gate — they thrash disk/CPU
# and can kill each other (0-byte artifacts / exit 144).
#
# `pgrep -f` matches our OWN transient children too: the subshells/pipeline
# members bash fork()s for a `$(...)` still carry the parent
# `bash scripts/artifact-gate.sh …` command line before they exec(). Excluding
# only `$$` (the main shell) missed those and reported a phantom "pid N" with no
# real concurrent run. Instead skip every candidate sharing our process group —
# our children inherit our PGID at fork() (excluded even mid-race), a separate
# run is launched into its own session/group. An already-exited candidate (empty
# PGID) is not a live run.
mypgid=$(ps -o pgid= -p "$$" | tr -d ' ')
other=""
for pid in $(pgrep -f 'artifact-gate\.sh'); do
  [ "$pid" = "$$" ] && continue
  cpgid=$(ps -o pgid= -p "$pid" 2>/dev/null | tr -d ' ')
  [ -z "$cpgid" ] && continue
  [ "$cpgid" = "$mypgid" ] && continue
  # bug-455: a candidate only counts if it is EXECUTING the script, not merely
  # mentioning it. Another session's wrapper shell
  # (`zsh -c "... scripts/artifact-gate.sh ..."`) carries the path inside its `-c`
  # string and matches `pgrep -f` while holding no lock at all -- observed
  # blocking a run whose rival was still in its `cargo build` stage, and causing
  # mutual-wait deadlocks between sessions politely queueing on each other's
  # text. A real invocation has the script as argv[0] (`./scripts/artifact-gate.sh`) or
  # argv[1] (`bash scripts/artifact-gate.sh`); a wrapper has `-c` there instead.
  cargs=$(ps -o args= -p "$pid" 2>/dev/null)
  ca0=${cargs%% *}
  carest=${cargs#* }
  ca1=${carest%% *}
  case "$ca0" in
    */artifact-gate.sh|artifact-gate.sh) ;;
    *)
      case "$ca1" in
        */artifact-gate.sh|artifact-gate.sh) ;;
        *) continue ;;
      esac
      ;;
  esac
  other=$pid
  break
done
if [ -n "$other" ]; then
  echo "Another artifact-gate (pid $other) is running." >&2
# Exit 98, not 1: a refusal is NOT a gate result. Sharing 1 with "found diffs"
# means a lock collision reads as a golden regression, and the reader spends
# their time on the wrong question (observed: `cargo test` and a manual
# `artifact-gate.sh all` refusing each other, and `tests/golden.rs` reporting it
# as a failed gate in 0.16s).
  exit 98
fi
# shellcheck source=artifact-kinds.sh
. "$SCRIPT_DIR/artifact-kinds.sh"
host_arch="$(uname -m)"; case "$host_arch" in arm64) A=aarch64;; x86_64) A=x86_64;; *) A=$host_arch;; esac
case "$(uname -s)" in Darwin) HOST_TGT="macos-$A";; Linux) HOST_TGT="linux-$A";; *) HOST_TGT="unknown-$A";; esac
diffs=0; checked=0; ran=0; builds=0

# The native artifact extensions, in the order they are reported (shared table).
NATIVE_EXTS="$ARTIFACT_NATIVE_KINDS"

while IFS= read -r pj; do
  td=$(dirname "$pj")
  rel="${td#"$REPO"/tests/}"; rel="${rel%/}"
  pkg=$(sed -n 's/.*"name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$td/project.json" | head -1)
  [ -n "$pkg" ] || continue
  g="$td/golden"; [ -d "$g" ] || continue
  ran=$((ran+1))
  # Per-fixture accounting so each test gets a status line (PASSED/DIFF/MISSING/SKIP).
  f_diffs=$diffs; f_checked=$checked

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

  # Pass 1: target-independent host dumps (ARTIFACT_HOST_KINDS). Built once, for
  # the host, since none depend on the backend. `-ast -ir` is the unconditional
  # base so the invocation is always a front-end-only dump (never a link); the
  # remaining host kinds (hex) are added only when their golden is present.
  # `-q` mirrors test-accept.sh so the two drivers issue the same build; the
  # gate discards stdout regardless, and `-q` never changes the dump files.
  flags="-ast -ir"
  for k in $ARTIFACT_HOST_KINDS; do
    case "$k" in ast|ir) continue ;; esac
    [ -f "$g/$pkg.$k" ] && flags="$flags $(artifact_build_flag "$k")"
  done
  rm -f "$td/$pkg".{ast,ir,hex,nir,nplan,nobj,ncode,mir} 2>/dev/null
  "$MFB" build -q $flags "$td" >/dev/null 2>&1
  builds=$((builds+1))
  for k in $ARTIFACT_HOST_KINDS; do
    gf="$g/$pkg.$k"; af="$td/$pkg.$k"
    [ -f "$gf" ] || continue
    checked=$((checked+1))
    if [ ! -f "$af" ]; then echo "MISSING $rel/$pkg.$k"; diffs=$((diffs+1)); continue; fi
    cmp -s "$gf" "$af" || { echo "DIFF $rel/$pkg.$k"; diffs=$((diffs+1)); }
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
    # `-q` mirrors test-accept.sh (parity; the gate discards stdout anyway).
    # shellcheck disable=SC2086
    "$MFB" build -q $tflags $targ $mode "$td" >/dev/null 2>&1
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

  # Per-fixture status. The MISSING/DIFF lines above already name each failing
  # golden; this rolls the fixture up into a single labeled result so a clean
  # run streams a PASSED line per test instead of running silent.
  f_checked_n=$((checked - f_checked)); f_diffs_n=$((diffs - f_diffs))
  if [ "$f_checked_n" -eq 0 ]; then
    echo "SKIP    $rel/$pkg (no matching goldens)"
  elif [ "$f_diffs_n" -eq 0 ]; then
    echo "PASSED  $rel/$pkg ($f_checked_n golden(s))"
  else
    echo "FAILED  $rel/$pkg ($f_diffs_n/$f_checked_n golden(s))"
  fi
done < <(find "$FIND_ROOT" -name project.json | sort)
echo "artifact-gate [$SEL]: $ran tests, $builds build(s), $checked golden(s) checked, $diffs diff(s)"
[ "$diffs" -eq 0 ]
