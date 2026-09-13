#!/usr/bin/env bash
# Capture (or verify) a byte-exact baseline of every artifact the compiler emits
# for a set of targets: every codegen dump, plus every linked `.out` (both libc
# flavors on Linux).
#
# Why this exists: bug-321 refactored the three Linux backends with byte-identical
# output as its only acceptance criterion. `scripts/artifact-gate.sh` now compares
# committed goldens for every target, Linux included, but it only sees fixtures
# that HAVE a golden, and only the dump kinds those goldens pin. This manifest
# covers what it cannot: the linked `.out` of every fixture, and fixtures that
# commit no golden at all.
#
# The linked `.out` matters on its own: the entry stub and runtime-helper bodies
# are linked per executable, not into the package object the gate compares, and
# bug-85 proved that is exactly where a token-audit miss becomes a silent crash.
# `--targets macos-aarch64` is the full-executable oracle for the host (it
# replaces the old in-tree full-executable oracle script); the default is the three Linux targets.
#
# The compiler cross-compiles, so artifacts are produced here on the host — no
# Linux box is needed to CAPTURE them. Linux boxes are only needed to RUN the
# resulting binaries, which is a separate behavioral proof
# (`scripts/linux-runtime-proof.sh`).
#
# Every build runs in a scratch copy of its fixture, never in the tree, so this
# needs no gate lock and can run beside `artifact-gate.sh` or `test-accept.sh`.
#
# The baseline is a manifest of SHA-256 hashes, not the artifacts themselves:
# storing them costs multiple GB, and a hash is sufficient to *detect* a change.
# When `verify` reports a diff, re-run that one fixture/target by hand with the
# same flags to see the actual bytes.
#
# Usage:
#   scripts/artifact-baseline.sh <mfb-exe> capture <manifest> [--targets t1,t2,...]
#   scripts/artifact-baseline.sh <mfb-exe> verify  <manifest> [--targets t1,t2,...]
#   --targets      ... comma-separated build targets
#                      (default: linux-aarch64,linux-x86_64,linux-riscv64)
#   FILTER=<substring> ... restrict to fixtures whose path contains it
#   JOBS=<n>            ... fixtures to build concurrently (default: CPU count)
#
# Use a RELEASE `mfb`. A debug build cross-compiles ~1000 fixtures x 3 targets at
# roughly 6 manifest lines/minute here -- a multi-day run. Release plus the
# per-fixture parallelism below brings a full capture into the tens of minutes.
set -u

USAGE="usage: artifact-baseline.sh <mfb-exe> <capture|verify> <manifest> [--targets t1,t2,...]"
ROOT=$(cd "$(dirname "$0")/.." && pwd)
MFB=${1:?$USAGE}
MODE=${2:?$USAGE}
MANIFEST=${3:?$USAGE}
[ "$MODE" = capture ] || [ "$MODE" = verify ] || { echo "mode must be capture|verify" >&2; exit 2; }
shift 3
TARGETS="linux-aarch64 linux-x86_64 linux-riscv64"
while [ "$#" -gt 0 ]; do
  case "$1" in
    --targets)
      [ "$#" -ge 2 ] && [ -n "$2" ] || { echo "$USAGE" >&2; exit 2; }
      TARGETS=$(printf '%s' "$2" | tr ',' ' ')
      shift 2 ;;
    *) echo "$USAGE" >&2; exit 2 ;;
  esac
done
MFB=$(cd "$(dirname "$MFB")" && pwd)/$(basename "$MFB")
FILTER=${FILTER:-}
JOBS=${JOBS:-$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 4)}
ntargets=$(set -- $TARGETS; echo $#)

# The sha256 of a file (or of stdin, with no argument), as bare hex. macOS and most
# glibc Linux ship `shasum` (perl); Alpine ships only busybox `sha256sum`. Without
# either, every hash would be empty, and a capture and a later verify on that host
# would agree on emptiness and report "no differences" whatever changed, so stop.
if command -v shasum >/dev/null 2>&1; then
  sha256_of() { shasum -a 256 "$@" | cut -d' ' -f1; }
elif command -v sha256sum >/dev/null 2>&1; then
  sha256_of() { sha256sum "$@" | cut -d' ' -f1; }
else
  echo "artifact-baseline: neither shasum nor sha256sum is on PATH" >&2
  exit 2
fi

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
  out="$WORKDIR/parts/$(printf '%s' "$rel" | sha256_of | cut -c1-32)"
  : > "$out"

  for target in $TARGETS; do
    # Build a scratch copy so a cross-build cannot leave artifacts in the tree.
    scratch="$WORKDIR/w$slot"
    rm -rf "$scratch"; cp -R "$proj" "$scratch"; rm -rf "$scratch/build"
    # A relative symlink that climbs out of the fixture (a `vendor/` entry pointing
    # into `packages/<pkg>/vendor/`) dangles in the scratch copy, and the build then
    # fails on a file that exists in the tree. Replace every link that dangles here
    # but resolves in the fixture with a copy of its target. Links that still resolve
    # (inside the fixture, or absolute) are kept as links: the fs fixtures' symlinks
    # are the thing their programs test.
    find "$scratch" -type l | while IFS= read -r link; do
      [ -e "$link" ] && continue
      orig="$proj/${link#"$scratch"/}"
      [ -e "$orig" ] || continue
      rm -f "$link"; cp -RL "$orig" "$link"
    done
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
      echo "$rel|$target|$(basename "$f")|$(sha256_of "$f")" >> "$out"
    done
    # The linked executables need their own build: a build given dump flags writes
    # the dumps and stops without linking, so the build above never produces
    # `build/*.out` (plan-131-B measured 0 `.out` lines in a full manifest before
    # this second build existed). A fixture that does not link yields no `.out`
    # line, which a later capture shows as a missing line.
    rm -rf "$scratch/build"
    "$MFB" build -q --target "$target" "$scratch" >>"$WORKDIR/log$slot" 2>&1
    for exe in "$scratch/build/"*.out; do
      [ -f "$exe" ] || continue
      echo "$rel|$target|$(basename "$exe")|$(sha256_of "$exe")" >> "$out"
    done
  done
}

mkdir -p "$work/parts"
export WORKDIR=$work MFB ROOT TARGETS DUMPS
export -f emit_fixture sha256_of 2>/dev/null || true

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
  echo "baseline captured: $n fixture(s) x $ntargets target(s), $(wc -l < "$MANIFEST" | tr -d ' ') artifact hash(es) -> $MANIFEST"
  exit 0
fi

[ -f "$MANIFEST" ] || { echo "no baseline at $MANIFEST — run 'capture' first" >&2; exit 2; }
if diff -u "$MANIFEST" "$tmp_manifest" > "$work/diff"; then
  echo "artifact baseline verified: $n fixture(s) x $ntargets target(s), $(wc -l < "$tmp_manifest" | tr -d ' ') hash(es), no differences"
  exit 0
fi
echo "artifact baseline FAILED — differences below (- baseline, + now):" >&2
grep -E '^[-+][^-+]' "$work/diff" | head -80 >&2
echo "..." >&2
echo "total differing lines: $(grep -cE '^[-+][^-+]' "$work/diff")" >&2
exit 1
