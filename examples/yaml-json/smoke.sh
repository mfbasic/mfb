#!/usr/bin/env bash
#
# smoke.sh — check every command-line path of the yamljson example.
#
# The `yaml` package has its own test suite (`mfb test packages/yaml`), and
# `scripts/yaml_oracle_diff.py` differential-tests it against PyYAML. Neither
# touches this program's ARGUMENT HANDLING, which is how `yamljson <file>`
# shipped comparing `fs::pathExtension` against "yaml" when that call returns
# ".yaml" — every by-extension invocation failed and nothing noticed. This
# script is the check that would have.
#
# It builds the package and the example first, so it is runnable from a clean
# checkout. Usage:
#
#     examples/yaml-json/smoke.sh [path/to/mfb]

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
APP="$ROOT/examples/yaml-json"
MFB="${1:-$ROOT/target/release/mfb}"
BIN="$APP/build/yamljson.out"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

if [[ ! -x "$MFB" ]]; then
  echo "no compiler at $MFB — pass one as \$1, or run: cargo build --release --bin mfb" >&2
  exit 1
fi

echo "==> Building packages/yaml and the example"
"$MFB" build -q "$ROOT/packages/yaml"
mkdir -p "$APP/packages"
cp "$ROOT/packages/yaml/yaml.mfp" "$APP/packages/yaml.mfp"
"$MFB" build -q "$APP"

failures=0

# check <name> <expected-exit> <expected-substring-of-output> -- <args...>
check() {
  local name="$1" want_status="$2" want_text="$3"
  shift 4                                   # name, status, text, and the `--`
  local output status
  set +e
  output="$("$BIN" "$@" 2>&1)"
  status=$?
  set -e
  if [[ "$status" != "$want_status" ]]; then
    echo "FAIL $name: exit $status, wanted $want_status"
    echo "     output: ${output:0:200}"
    failures=$((failures + 1))
    return
  fi
  if [[ "$output" != *"$want_text"* ]]; then
    echo "FAIL $name: output does not contain '$want_text'"
    echo "     output: ${output:0:200}"
    failures=$((failures + 1))
    return
  fi
  echo "ok   $name"
}

printf 'a: 1\nb: [x, y]\n'          > "$WORK/in.yaml"
cp "$WORK/in.yaml"                    "$WORK/in.yml"
printf '{"a": 1, "b": ["x", "y"]}\n' > "$WORK/in.json"
printf -- '--- one\n--- two\n'      > "$WORK/two.yaml"
printf 'a: 1\na: 2\n'               > "$WORK/dup.yaml"
printf 'a: 1\n'                     > "$WORK/in.txt"

echo "==> Checking"
# The two explicit directions.
check "to-json"            0 '"b": ['            -- to-json "$WORK/in.yaml"
check "to-json --compact"  0 '{"a":1,"b":["x","y"]}' -- to-json "$WORK/in.yaml" --compact
check "to-yaml"            0 'b:'                -- to-yaml "$WORK/in.json"

# Direction picked from the extension — all three spellings.
check "by extension .yaml" 0 '"a": 1'            -- "$WORK/in.yaml"
check "by extension .yml"  0 '"a": 1'            -- "$WORK/in.yml"
check "by extension .json" 0 'a: 1'              -- "$WORK/in.json"

# A stream of several documents becomes a JSON array rather than losing one.
check "multi-document"     0 '["one","two"]'     -- to-json "$WORK/two.yaml" --compact

# Usage and argument errors exit 2; a failure while working exits 1.
check "--help"             0 'Usage:'            -- --help
check "-h"                 0 'Usage:'            -- -h
check "no arguments"       2 'Usage:'            --
check "unknown option"     2 'unknown option'    -- --bogus "$WORK/in.yaml"
check "two input files"    2 'more than one'     -- to-json "$WORK/in.yaml" "$WORK/in.yml"
check "unknown extension"  1 'cannot tell the direction' -- "$WORK/in.txt"
check "missing file"       1 '77030001'          -- to-json "$WORK/nope.yaml"
check "rejected YAML"      1 'duplicate mapping key' -- to-json "$WORK/dup.yaml"

echo
if [[ "$failures" -gt 0 ]]; then
  echo "$failures check(s) failed"
  exit 1
fi
echo "all checks passed"
