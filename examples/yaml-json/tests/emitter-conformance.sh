#!/usr/bin/env bash
#
# emitter-conformance.sh — bug-710 regression check for the yamljson
# example's `to-yaml` emitter (examples/yaml-json/src/emit.mfb).
#
# Checks two independent defects:
#
#   (a) the emitter must escape every code point a conforming YAML reader
#       (1.1 or 1.2) cannot accept literally in a scalar: the C1 controls
#       (U+0080-U+009F, which includes NEL U+0085), and the YAML-1.1
#       line-break characters LINE SEPARATOR (U+2028) and PARAGRAPH
#       SEPARATOR (U+2029). Quoting alone is not enough — `quoteScalar`
#       delegates to `json::stringify`, which does not escape these code
#       points, so they must be escaped in the quoted output too. Checked
#       for both a plain scalar and a multi-line (block-scalar-eligible)
#       string, since both had the same gap.
#   (b) `to-yaml` output must end with exactly one trailing newline (it
#       used to end with two: `toYaml` appended its own, and its only
#       caller's `io::print` appends another).
#
# Verified against PyYAML (`python3 -c 'import yaml'` — required; this
# script does not attempt to vendor a YAML parser itself, matching
# packages/yaml/oracle/pyyaml_diff.py's existing use of PyYAML as the
# reference implementation elsewhere in this repo).
#
# Usage:
#     examples/yaml-json/tests/emitter-conformance.sh [path/to/mfb]

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
APP="$ROOT/examples/yaml-json"
MFB="${1:-$ROOT/target/release/mfb}"
BIN="$APP/build/yamljson.out"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

if [[ ! -x "$MFB" ]]; then
  echo "no compiler at $MFB — pass one as \$1, or run: cargo build --release --bin mfb" >&2
  exit 1
fi
if ! python3 -c 'import yaml' >/dev/null 2>&1; then
  echo "PyYAML not importable by python3 — required as the reference YAML reader" >&2
  exit 1
fi

echo "==> Building packages/yaml and the example"
"$MFB" build -q "$ROOT/packages/yaml"
mkdir -p "$APP/packages"
cp "$ROOT/packages/yaml/yaml.mfp" "$APP/packages/yaml.mfp"
"$MFB" build -q "$APP"

failures=0

# yaml_accepts <name> <json-value-py-literal>
# Writes {"value": <json-value>} as JSON, runs it through to-yaml, and
# asserts PyYAML's safe_load accepts the result and recovers the same value.
yaml_accepts() {
  local name="$1" py_value="$2"
  local jf="$WORK/$name.json" yf="$WORK/$name.yaml"
  python3 -c "import json,sys; json.dump({'value': $py_value}, open(sys.argv[1], 'w'))" "$jf"
  if ! "$BIN" to-yaml "$jf" > "$yf" 2>"$WORK/$name.err"; then
    echo "FAIL $name: to-yaml exited non-zero: $(cat "$WORK/$name.err")"
    failures=$((failures + 1))
    return
  fi
  if ! python3 -c "
import sys, yaml
with open(sys.argv[1]) as f:
    got = yaml.safe_load(f)
want = $py_value
if not isinstance(got, dict) or got.get('value') != want:
    print('mismatch: got', repr(got), 'want value', repr(want))
    sys.exit(1)
" "$yf"; then
    echo "FAIL $name: PyYAML did not accept/recover the value; emitted:"
    sed 's/^/     /' "$yf"
    failures=$((failures + 1))
    return
  fi
  echo "ok   $name"
}

echo "==> Checking (a): unsafe code points are escaped, not passed through raw"
yaml_accepts c1-control   "'abc\u0081def'"
yaml_accepts nel          "'abc\u0085def'"
yaml_accepts line-sep     "'abc def'"
yaml_accepts para-sep     "'abc def'"
yaml_accepts block-nel    "'line one\nline\u0085two\nline three\n'"

echo "==> Checking (b): exactly one trailing newline"
"$BIN" to-yaml "$APP/samples/package.json" > "$WORK/pkg.yaml"
# Want the last two bytes to be "<something-not-\n> \n", not "\n \n".
last_byte="$(tail -c 1 "$WORK/pkg.yaml" | od -An -c | tr -d ' ')"
second_last_byte="$(tail -c 2 "$WORK/pkg.yaml" | head -c 1 | od -An -c | tr -d ' ')"
if [[ "$last_byte" != '\n' ]]; then
  echo "FAIL trailing-newline: output does not even end in a newline (last byte: $last_byte)"
  failures=$((failures + 1))
elif [[ "$second_last_byte" == '\n' ]]; then
  echo "FAIL trailing-newline: output ends in two newlines (bug-710b regressed)"
  failures=$((failures + 1))
else
  echo "ok   trailing-newline (exactly one)"
fi

echo
if [[ "$failures" -gt 0 ]]; then
  echo "$failures check(s) failed"
  exit 1
fi
echo "all checks passed"
