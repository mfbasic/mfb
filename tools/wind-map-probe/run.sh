#!/bin/sh
# run.sh — build and run wind-map-probe, and assert its measured frame item
# counts match the fixed shape (bug-712): the static map contributes exactly
# one item (a canvas::Group node) to a frame's scene, at any zoom, and that
# stays true on a repeated call with the same View (mapItems's own cache).
#
# This probe exercises the REAL examples/weather/src/land.mfb and earth.mfb, and
# the real examples/weather/resources/weather/coastline.bin — copied into a scratch
# staging directory fresh on every run, never committed here. If land.mfb or
# earth.mfb regresses, the next run stages the regressed file and this probe
# catches it; a committed fork of those files could not.
#
# probe/main.mfb (the only file that IS committed under this tool) is
# probe-only glue: it calls mapItems() directly and prints how many items
# it returns. It is not a copy of anything in examples/weather.
#
# Usage: run.sh <path-to-mfb-binary>

set -eu

MFB_BIN="${1:-}"
if [ -z "$MFB_BIN" ]; then
  echo "usage: run.sh <path-to-mfb-binary>" >&2
  exit 2
fi
MFB_BIN=$(cd "$(dirname "$MFB_BIN")" && pwd)/$(basename "$MFB_BIN")

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
WEATHER_SRC="$SCRIPT_DIR/../../examples/weather/src"
WEATHER_RESOURCES="$SCRIPT_DIR/../../examples/weather/resources/weather"

STAGE=$(mktemp -d)
cleanup() {
  status=$?
  rm -rf "$STAGE"
  exit $status
}
trap cleanup EXIT INT TERM

mkdir -p "$STAGE/src" "$STAGE/resources/weather"

# The real, unmodified files under test — copied fresh, not forked.
cp "$WEATHER_SRC/land.mfb" "$STAGE/src/land.mfb"
cp "$WEATHER_SRC/earth.mfb" "$STAGE/src/earth.mfb"
cp "$WEATHER_RESOURCES/coastline.bin" "$STAGE/resources/weather/coastline.bin"

# The probe's own driver — glue, not a fork of anything in examples/weather.
cp "$SCRIPT_DIR/probe/main.mfb" "$STAGE/src/main.mfb"

cat > "$STAGE/project.json" <<'EOF'
{
  "name": "wind-map-probe",
  "version": "0.1.0",
  "mfb": "1.0",
  "kind": "executable",
  "mode": "app",
  "sources": [
    {
      "root": "src",
      "role": "main",
      "include": ["**/*.mfb"]
    }
  ],
  "resources": [
    {
      "src": "resources/weather/*.bin",
      "dst": "weather/"
    }
  ],
  "entry": "main",
  "targets": ["native"]
}
EOF

"$MFB_BIN" build "$STAGE" >"$STAGE/build.log" 2>&1 || {
  echo "FAIL: wind-map-probe did not build" >&2
  cat "$STAGE/build.log" >&2
  exit 1
}

if [ "$(uname -s)" != "Darwin" ]; then
  echo "skip: wind-map-probe is an --app build, only exercised on macOS here" >&2
  exit 0
fi

EXE="$STAGE/build/wind-map-probe.app/Contents/MacOS/wind-map-probe"

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
