#!/usr/bin/env bash
#
# run.sh — build everything the oracle needs, then run every mode.
#
# Runnable from a clean checkout: it compiles the package and the probe, checks
# that jose is installed, runs the package's own tests, and then the comparison.
#
#     ./run.sh                     # everything, with the default compiler
#     ./run.sh path/to/mfb         # everything, with a compiler you name
#     ./run.sh '' corpus cross     # only some modes
#
# Exit status is 0 iff every case agreed, or diverged for a reason declared in
# divergences.json.

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../../.." && pwd)"
MFB="${1:-}"
if [[ -z "$MFB" ]]; then
  MFB="$ROOT/target/release/mfb"
fi

if [[ ! -x "$MFB" ]]; then
  echo "no compiler at $MFB — pass one as \$1, or run: cargo build --release --bin mfb" >&2
  exit 1
fi

if ! command -v node >/dev/null 2>&1; then
  echo "this oracle needs Node 18 or newer: https://nodejs.org" >&2
  exit 1
fi

if [[ ! -d "$HERE/node_modules/jose" ]]; then
  echo "==> Installing jose"
  (cd "$HERE" && npm install)
fi

echo "==> Building packages/jwt"
"$MFB" build -q "$HERE/.."

echo "==> Running the package's own tests"
"$MFB" test "$HERE/.."

echo "==> Building the probe"
mkdir -p "$HERE/probe/packages"
cp "$HERE/../jwt.mfp" "$HERE/probe/packages/jwt.mfp"
"$MFB" build -q "$HERE/probe"

echo "==> Comparing against jose"
cd "$HERE"
node diff.mjs "${@:2}"
