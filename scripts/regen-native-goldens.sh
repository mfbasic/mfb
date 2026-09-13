#!/usr/bin/env bash
# Regenerate the per-target native goldens in place, after an intended codegen
# change. `artifact-gate.sh` has no write mode; this is its write half.
#
# For every fixture with a `golden/` directory, every existing golden named
# `<pkg>.<target>[.app].<kind>` or `<pkg>.<target>[.app].<kind>sum`, where
# <kind> is one of `$ARTIFACT_NATIVE_KINDS` (scripts/artifact-kinds.sh), is
# rebuilt for the target (and app mode) named in its filename and rewritten:
# a raw golden with the fresh dump, a `sum` golden with its sha256. The
# enumeration is `artifact-gate.sh`'s Pass 2, so the goldens rewritten here are
# exactly the goldens the gate checks. Front-end host dumps (`.ast/.ir/.hex`)
# are not touched; `sync-goldens.sh` owns those.
#
# Only EXISTING goldens are rewritten; a fixture's golden set never changes
# shape, and no golden is ever created.
#
# A failed build never writes a golden. The dump path carries no target infix,
# so every target of a fixture writes the SAME `<pkg>.<kind>` file: the stale
# dumps are removed before each build, and nothing is hashed or copied unless
# THIS build exited 0 and produced the dump. Without that, a failed build
# re-hashes the previously built target's dump into this target's golden -- a
# wrong but self-consistent value the gate then reports as green forever
# (observed: the macos-aarch64 sum written into 26 windows-x86_64 goldens).
#
# The host target comes from `uname`, as in `artifact-gate.sh`, so a golden for
# the host target is built without `-target` on any host.
#
# Usage: scripts/regen-native-goldens.sh <mfb-exe> [fixture-dir...]
#   With no fixture dirs, every fixture under tests/ is swept.
#   Exits non-zero if any build failed or any expected dump was missing.
#
# bug-513: this script word-splits `$targ` into `-target <t>`, which zsh does not
# do. Re-exec under bash rather than trusting the shebang to have been honoured
# (the file is not always executable in a fresh worktree).
if [ -n "${ZSH_VERSION:-}" ]; then
  exec bash "$0" "$@"
fi
set -u
MFB=${1:?usage: regen-native-goldens.sh <mfb-exe> [fixture-dir...]}
shift
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$SCRIPT_DIR/.." && pwd)"
MFB="$(cd "$(dirname "$MFB")" && pwd)/$(basename "$MFB")"

# bug-470: this script rewrites and deletes the SAME fixture dumps that
# `artifact-gate.sh` and `test-accept.sh` do, so it contends with them for the
# tree and must take the same per-tree lock. Regenerate-then-gate is the normal
# workflow after an intended codegen change.
GATE_LOCK_HOLDER="regen-native-goldens.sh"
GATE_LOCK_TREE="$REPO"
# shellcheck source=gate-lock.sh
. "$SCRIPT_DIR/gate-lock.sh"
gate_lock_acquire || exit $?

# shellcheck source=artifact-kinds.sh
. "$SCRIPT_DIR/artifact-kinds.sh"

# The sha256 of a file, as 64 lowercase hex characters. macOS and most glibc
# Linux ship `shasum` (perl); Alpine ships only busybox `sha256sum`. A missing
# tool must stop the run: `shasum … | cut … > golden` with no `shasum` writes an
# EMPTY sum into every golden and still counts it as rewritten.
if command -v shasum >/dev/null 2>&1; then
  sha256_of() { shasum -a 256 "$1" | cut -d' ' -f1; }
elif command -v sha256sum >/dev/null 2>&1; then
  sha256_of() { sha256sum "$1" | cut -d' ' -f1; }
else
  echo "regen-native-goldens: neither shasum nor sha256sum is on PATH" >&2
  exit 2
fi

host_arch="$(uname -m)"; case "$host_arch" in arm64) A=aarch64;; x86_64) A=x86_64;; *) A=$host_arch;; esac
case "$(uname -s)" in Darwin) HOST_TGT="macos-$A";; Linux) HOST_TGT="linux-$A";; *) HOST_TGT="unknown-$A";; esac

if [ "$#" -eq 0 ]; then
  set -- "$REPO/tests"
fi

updated=0; failed=0; builds=0
while IFS= read -r pj; do
  td=$(dirname "$pj")
  rel="${td#"$REPO"/}"
  pkg=$(sed -n 's/.*"name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$pj" | head -1)
  [ -n "$pkg" ] || continue
  g="$td/golden"; [ -d "$g" ] || continue

  # Which targets does this fixture carry native goldens for? Derived from the
  # golden filenames, exactly as artifact-gate.sh derives them.
  targets=""
  for ext in $ARTIFACT_NATIVE_KINDS; do
    for suffix in "$ext" "${ext}sum"; do
      for gf in "$g/$pkg."*".$suffix"; do
        [ -f "$gf" ] || continue
        base="${gf##*/}"; t="${base#"$pkg."}"; t="${t%".$suffix"}"
        case " $targets " in *" $t "*) ;; *) targets="$targets $t" ;; esac
      done
    done
  done

  for t in $targets; do
    tflags=""
    for ext in $ARTIFACT_NATIVE_KINDS; do
      { [ -f "$g/$pkg.$t.$ext" ] || [ -f "$g/$pkg.$t.${ext}sum" ]; } && tflags="$tflags -$ext"
    done
    [ -n "$tflags" ] || continue
    # A `<target>.app` infix is an app-mode build, not a distinct target.
    mode=""; tt="$t"
    case "$t" in *.app) mode="--app"; tt="${t%.app}" ;; esac
    targ=""
    [ "$tt" = "$HOST_TGT" ] || targ="-target $tt"

    rm -f "$td/$pkg".{nir,nplan,nobj,ncode,mir} 2>/dev/null
    builds=$((builds+1))
    # shellcheck disable=SC2086
    if ! "$MFB" build -q $tflags $targ $mode "$td" >/dev/null 2>&1; then
      echo "BUILD FAILED $rel ($t) — its goldens left unchanged"
      failed=$((failed+1))
      rm -f "$td/$pkg".{nir,nplan,nobj,ncode,mir} 2>/dev/null
      continue
    fi
    for ext in $ARTIFACT_NATIVE_KINDS; do
      af="$td/$pkg.$ext"
      for gf in "$g/$pkg.$t.${ext}sum" "$g/$pkg.$t.$ext"; do
        [ -f "$gf" ] || continue
        if [ ! -f "$af" ]; then
          echo "MISSING dump $rel/$pkg.$ext for ${gf#"$REPO"/} — golden left unchanged"
          failed=$((failed+1))
          continue
        fi
        case "$gf" in
          *sum)
            digest=$(sha256_of "$af")
            case "$digest" in
              *[!0-9a-f]*|"")
                echo "BAD DIGEST '$digest' for ${gf#"$REPO"/} — golden left unchanged"
                failed=$((failed+1))
                continue ;;
            esac
            if [ "${#digest}" -ne 64 ]; then
              echo "BAD DIGEST '$digest' for ${gf#"$REPO"/} — golden left unchanged"
              failed=$((failed+1))
              continue
            fi
            printf '%s\n' "$digest" > "$gf" ;;
          *) cp "$af" "$gf" ;;
        esac
        updated=$((updated+1))
      done
    done
    rm -f "$td/$pkg".{nir,nplan,nobj,ncode,mir} 2>/dev/null
  done
done < <(for d in "$@"; do find "$d" -name project.json; done | sort -u)

echo "regen-native-goldens: $builds build(s), $updated golden(s) rewritten, $failed failure(s)"
[ "$failed" -eq 0 ]
