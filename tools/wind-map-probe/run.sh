#!/bin/sh
# run.sh — build and run wind-map-probe, and assert its measured frame item
# counts match the fixed shape (bug-712): the static map contributes exactly
# one item (a canvas::Group node) to a frame's scene, at any zoom, and that
# stays true on a repeated call with the same View (mapItems's own cache).
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
  echo "FAIL: wind-map-probe did not build" >&2
  cat "$SCRIPT_DIR/build.log" >&2
  exit 1
}

if [ "$(uname -s)" != "Darwin" ]; then
  echo "skip: wind-map-probe is an --app build, only exercised on macOS here" >&2
  exit 0
fi

EXE="$SCRIPT_DIR/build/wind-map-probe.app/Contents/MacOS/wind-map-probe"

# Run headless (no window needed) with a watchdog, the same shape
# scripts/test-macapp.sh uses for every other --app runtime probe.
OUT=$(MFB_MACAPP_HEADLESS=1 perl -e '
  my $limit = shift @ARGV;
  my $pid = open(my $fh, "-|");
  if ($pid == 0) { exec($ARGV[0]) or exit 127; }
  local $SIG{ALRM} = sub { kill "KILL", $pid; exit 99; };
  alarm $limit; local $/; my $o = <$fh>; close($fh); print $o;
' 15 "$EXE")

echo "$OUT"

FAIL=0
check() {
  case "$OUT" in
    *"$1"*) ;;
    *) echo "FAIL: expected to see '$1'" >&2; FAIL=1 ;;
  esac
}

check "whole-world frame items: 1"
check "10x zoom frame items: 1"
check "10x zoom frame items again (cache hit): 1"

if [ "$FAIL" -ne 0 ]; then
  echo "FAIL: wind-map-probe — the static map is still costing more than one item a frame" >&2
  exit 1
fi

echo "PASS: wind-map-probe — the static map contributes exactly one item (a canvas::Group node) to a frame's scene, at any zoom"
