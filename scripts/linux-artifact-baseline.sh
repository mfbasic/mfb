#!/usr/bin/env bash
# Capture (or verify) a byte-exact baseline of every artifact the compiler emits
# for the three Linux targets, both libc flavors.
#
# Why this exists: bug-321 refactors the three Linux backends and its ONLY
# acceptance criterion is byte-identical output. The repo commits zero Linux
# artifact goldens (`find tests -path "*/golden/*" -name "*linux*"` -> 0) and
# `scripts/artifact-gate.sh` derives its target from `uname`, so nothing in the
# tree can detect a byte-level change to a Linux backend. This substitutes for
# the gate that does not exist.
#
# The compiler cross-compiles, so artifacts are produced here on the host — no
# Linux box is needed to CAPTURE them. Linux boxes are only needed to RUN the
# resulting binaries, which is a separate behavioral proof.
#
# The baseline is a manifest of SHA-256 hashes, not the artifacts themselves:
# storing them costs multiple GB, and a hash is sufficient to *detect* a change.
# When `verify` reports a diff, re-run that one fixture/target by hand with the
# same flags to see the actual bytes.
#
# Usage:
#   scripts/linux-artifact-baseline.sh <mfb-exe> capture <manifest>
#   scripts/linux-artifact-baseline.sh <mfb-exe> verify  <manifest>
#   FILTER=<substring> ... restrict to fixtures whose path contains it
#   JOBS=<n>            ... fixtures to build concurrently (default: CPU count)
#
# Use a RELEASE `mfb`. A debug build cross-compiles ~1000 fixtures x 3 targets at
# roughly 6 manifest lines/minute here -- a multi-day run. Release plus the
# per-fixture parallelism below brings a full capture into the tens of minutes.
set -u

ROOT=$(cd "$(dirname "$0")/.." && pwd)
MFB=${1:?usage: linux-artifact-baseline.sh <mfb-exe> <capture|verify> <manifest>}
MODE=${2:?usage: linux-artifact-baseline.sh <mfb-exe> <capture|verify> <manifest>}
MANIFEST=${3:?usage: linux-artifact-baseline.sh <mfb-exe> <capture|verify> <manifest>}
[ "$MODE" = capture ] || [ "$MODE" = verify ] || { echo "mode must be capture|verify" >&2; exit 2; }
MFB=$(cd "$(dirname "$MFB")" && pwd)/$(basename "$MFB")
FILTER=${FILTER:-}
JOBS=${JOBS:-$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 4)}

TARGETS="linux-aarch64 linux-x86_64 linux-riscv64"
# Every intermediate the backends can emit. `.mir`/`.nir` are captured per-target
# too: if a refactor made them diverge by target, that is itself the regression.
DUMPS="--nir --nplan --nobj --ncode --mir"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
tmp_manifest="$work/manifest"
: > "$tmp_manifest"

# One fixture per worker. Each worker owns a private scratch directory, so the
# concurrent cross-builds cannot collide; results are written to per-fixture
# files and concatenated at the end, keeping the manifest deterministic
# regardless of completion order.
emit_fixture() {
  project=$1
  slot=$2
  proj=$(dirname "$project")
  rel=${proj#"$ROOT"/tests/}
  name=$(sed -n 's/.*"name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$project" | head -1)
  [ -n "$name" ] || return 0
  # Hash-suffixed so two distinct fixture paths can never share a part file.
  out="$WORKDIR/parts/$(printf '%s' "$rel" | shasum -a 256 | cut -c1-32)"
  : > "$out"

  for target in $TARGETS; do
    # Build a scratch copy so a cross-build cannot leave artifacts in the tree.
    scratch="$WORKDIR/w$slot"
    rm -rf "$scratch"; cp -R "$proj" "$scratch"; rm -rf "$scratch/build"
    if "$MFB" build -q $DUMPS --target "$target" "$scratch" >"$WORKDIR/log$slot" 2>&1; then
      status=ok
    else
      # A fixture that does not build for a target is still a baseline fact: if
      # the refactor changes which fixtures build, that must show up as a diff.
      status=build-failed
    fi
    echo "$rel|$target|STATUS|$status" >> "$out"

    for f in "$scratch/$name".*; do
      [ -f "$f" ] || continue
      case "$f" in *.mfb|*.json) continue;; esac
      echo "$rel|$target|$(basename "$f")|$(shasum -a 256 "$f" | awk '{print $1}')" >> "$out"
    done
    for exe in "$scratch/build/"*.out; do
      [ -f "$exe" ] || continue
      echo "$rel|$target|$(basename "$exe")|$(shasum -a 256 "$exe" | awk '{print $1}')" >> "$out"
    done
  done
}

mkdir -p "$work/parts"
export WORKDIR=$work MFB ROOT TARGETS DUMPS
export -f emit_fixture 2>/dev/null || true

> "$work/projects"
find "$ROOT/tests" -name project.json | sort | while IFS= read -r project; do
  rel=$(dirname "$project"); rel=${rel#"$ROOT"/tests/}
  case "$rel" in
    *"$FILTER"*) printf '%s\n' "$project" >> "$work/projects" ;;
  esac
done
n=$(grep -c . < "$work/projects" || true)

xargs -P "$JOBS" -I{} bash -c 'emit_fixture "$1" "$$"' _ {} < "$work/projects"

cat "$work"/parts/* > "$tmp_manifest" 2>/dev/null || :

sort "$tmp_manifest" -o "$tmp_manifest"

if [ "$MODE" = capture ]; then
  cp "$tmp_manifest" "$MANIFEST"
  echo "baseline captured: $n fixture(s) x 3 target(s), $(wc -l < "$MANIFEST" | tr -d ' ') artifact hash(es) -> $MANIFEST"
  exit 0
fi

[ -f "$MANIFEST" ] || { echo "no baseline at $MANIFEST — run 'capture' first" >&2; exit 2; }
if diff -u "$MANIFEST" "$tmp_manifest" > "$work/diff"; then
  echo "linux artifact baseline verified: $n fixture(s), $(wc -l < "$tmp_manifest" | tr -d ' ') hash(es), no differences"
  exit 0
fi
echo "linux artifact baseline FAILED — differences below (- baseline, + now):" >&2
grep -E '^[-+][^-+]' "$work/diff" | head -80 >&2
echo "..." >&2
echo "total differing lines: $(grep -cE '^[-+][^-+]' "$work/diff")" >&2
exit 1
