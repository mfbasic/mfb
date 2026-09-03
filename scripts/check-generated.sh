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

# The vector bodies live as BODY consts across src/codegen/builtins/vector/
# {func_,helper_}*.rs since the package.mfb split; a dedicated checker extracts
# and compares them per-FUNC instead of a single-artifact byte compare.
if python3 scripts/check_vector_bodies.py; then :; else status=1; fi
# plan-118-B: the per-SCALAR general-category and Script tables are rodata run
# tables now, not generated MFBASIC. `gen_unicode_script_table.py` imports
# `gen_regex_scripts.runs()` rather than re-reading the UCD, so the script name
# table and the script run table cannot disagree about a scalar.
check scripts/gen_unicode_gencat_table.py src/codegen/string/unicode/unicode_gencat_ranges.txt
check scripts/gen_unicode_script_table.py src/codegen/string/unicode/unicode_script_ranges.txt
check scripts/gen_regex_scripts.py src/codegen/string/unicode/unicode_script_names.mfb
# plan-123: the `Codepage` enum and the 27 WHATWG legacy single-byte tables are
# derived from the vendored index files under tools/codepage-index/. Nobody can
# review 3,342 mappings by eye, so the artifact is never hand-edited -- this gate
# is what makes that true.
check scripts/gen_codepage_tables.py src/codegen/builtins/encoding/helper_codepage_table.rs

exit "$status"
