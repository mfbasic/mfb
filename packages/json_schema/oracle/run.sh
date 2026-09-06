#!/usr/bin/env bash
#
# run.sh — build everything the oracle needs, then run every mode.
#
# Runnable from a clean checkout: it compiles the package and the driver, checks
# that ajv is installed, and runs the comparison. The official test suite is
# optional; without it the `suite` mode skips itself and says so.
#
#     ./run.sh                     # everything, with the default compiler
#     ./run.sh path/to/mfb         # everything, with a compiler you name
#
# Exit status is 0 iff every disagreement was one the oracle already knows
# about and has a written reason for.

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../../.." && pwd)"
MFB="${1:-$ROOT/target/release/mfb}"

if [[ ! -x "$MFB" ]]; then
  echo "no compiler at $MFB — pass one as \$1, or run: cargo build --release --bin mfb" >&2
  exit 1
fi

if ! command -v node >/dev/null 2>&1; then
  echo "this oracle needs Node: https://nodejs.org" >&2
  exit 1
fi

if [[ ! -d "$HERE/node_modules/ajv" ]]; then
  echo "==> Installing ajv"
  (cd "$HERE" && npm install)
fi

echo "==> Building packages/json_schema and its driver"
"$MFB" build -q "$HERE/driver"

echo "==> Running the package's own tests"
"$MFB" test "$HERE/.."

echo "==> Comparing against ajv and the official suite"
cd "$HERE"
node oracle.mjs "${@:2}"
