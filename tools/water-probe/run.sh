#!/bin/sh
# run.sh — build and run water-probe, the bug-716 regression check.
#
# Usage: run.sh <path-to-mfb-binary>

set -eu

MFB_BIN="${1:-}"
if [ -z "$MFB_BIN" ]; then
  echo "usage: run.sh <path-to-mfb-binary>" >&2
  exit 2
fi

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)

"$MFB_BIN" build "$SCRIPT_DIR" >"$SCRIPT_DIR/build.log" 2>&1 || {
  echo "FAIL: water-probe did not build" >&2
  cat "$SCRIPT_DIR/build.log" >&2
  exit 1
}

"$SCRIPT_DIR/build/water-probe.out"
