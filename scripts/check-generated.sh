#!/usr/bin/env sh
# Generated-artifact integrity gate (bug-339 A1).
#
# Several source files are machine-generated and carry a "do not edit by hand"
# banner, but nothing invoked them: not build.rs, not this workflow, no Makefile.
# A landed optimization once lived ONLY in `src/builtins/vector_package.mfb` while
# its generator still emitted the old body, so a maintainer who followed the
# banner and re-ran the generator would silently revert it — with no signal.
#
# This script re-runs each generator and fails if the checked-in artifact no
# longer matches, so "re-run the generator" is always safe and drift cannot land.
#
# Each entry is "<generator> <artifact>". A generator writes the artifact to
# stdout; its progress/stats go to stderr (discarded here) so only the artifact
# bytes are compared.
set -eu
ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"

status=0
tmp=$(mktemp)
trap 'rm -f "$tmp"' EXIT

check() {
  generator=$1
  artifact=$2
  if [ ! -f "$generator" ]; then
    echo "check-generated: missing generator '$generator'" >&2
    status=1
    return
  fi
  if [ ! -f "$artifact" ]; then
    echo "check-generated: missing artifact '$artifact'" >&2
    status=1
    return
  fi
  python3 "$generator" >"$tmp" 2>/dev/null
  if cmp -s "$artifact" "$tmp"; then
    echo "ok: $artifact matches $generator"
  else
    echo "DRIFT: $artifact does not match \`python3 $generator\`." >&2
    echo "       Re-run it to regenerate, or move any hand-landed change into" >&2
    echo "       the generator so the two agree:" >&2
    echo "         python3 $generator > $artifact" >&2
    diff -u "$artifact" "$tmp" | sed -n '1,40p' >&2 || true
    status=1
  fi
}

check scripts/gen_vector_package.py src/builtins/vector_package.mfb
check scripts/gen_regex_unicode.py src/builtins/regex_unicode.mfb

exit "$status"
