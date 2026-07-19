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
set -u

ROOT=$(cd "$(dirname "$0")/.." && pwd)
MFB=${1:?usage: linux-artifact-baseline.sh <mfb-exe> <capture|verify> <manifest>}
MODE=${2:?usage: linux-artifact-baseline.sh <mfb-exe> <capture|verify> <manifest>}
MANIFEST=${3:?usage: linux-artifact-baseline.sh <mfb-exe> <capture|verify> <manifest>}
[ "$MODE" = capture ] || [ "$MODE" = verify ] || { echo "mode must be capture|verify" >&2; exit 2; }
MFB=$(cd "$(dirname "$MFB")" && pwd)/$(basename "$MFB")
FILTER=${FILTER:-}

TARGETS="linux-aarch64 linux-x86_64 linux-riscv64"
# Every intermediate the backends can emit. `.mir`/`.nir` are captured per-target
# too: if a refactor made them diverge by target, that is itself the regression.
DUMPS="--nir --nplan --nobj --ncode --mir"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
tmp_manifest="$work/manifest"
: > "$tmp_manifest"

n=0
while IFS= read -r project; do
  proj=$(dirname "$project")
  rel=${proj#"$ROOT"/tests/}
  case "$rel" in *"$FILTER"*) ;; *) continue;; esac
  name=$(sed -n 's/.*"name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$project" | head -1)
  [ -n "$name" ] || continue
  n=$((n+1))

  for target in $TARGETS; do
    # Build a scratch copy so a cross-build cannot leave artifacts in the tree.
    rm -rf "$work/p"; cp -R "$proj" "$work/p"; rm -rf "$work/p/build"
    if "$MFB" build -q $DUMPS --target "$target" "$work/p" >"$work/log" 2>&1; then
      status=ok
    else
      # A fixture that does not build for a target is still a baseline fact: if
      # the refactor changes which fixtures build, that must show up as a diff.
      status=build-failed
    fi
    echo "$rel|$target|STATUS|$status" >> "$tmp_manifest"

    for f in "$work/p/$name".*; do
      [ -f "$f" ] || continue
      case "$f" in *.mfb|*.json) continue;; esac
      echo "$rel|$target|$(basename "$f")|$(shasum -a 256 "$f" | awk '{print $1}')" >> "$tmp_manifest"
    done
    for exe in "$work/p/build/"*.out; do
      [ -f "$exe" ] || continue
      echo "$rel|$target|$(basename "$exe")|$(shasum -a 256 "$exe" | awk '{print $1}')" >> "$tmp_manifest"
    done
  done
done < <(find "$ROOT/tests" -name project.json | sort)

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
