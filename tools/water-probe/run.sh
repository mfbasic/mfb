#!/bin/sh
# run.sh — build and run water-probe, the bug-716 regression check.
#
# This probe exercises the REAL examples/water/src/shapes.mfb and
# toolbar.mfb — staged into a scratch project fresh on every run
# (mktemp -d + cp), never committed copies. If either file regresses, the
# next run stages the regressed file and this probe catches it.
#
# probe/main.mfb (the only committed file here) is probe-only glue: it
# reproduces the exact decision order examples/water/src/main.mfb's
# mouse-down handler uses (onToolbar before shapeAt), calling the real
# onToolbar/buttonAt with a real canvas::Point, exactly as main.mfb does.
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
WATER_SRC="$SCRIPT_DIR/../../examples/water/src"

STAGE=$(mktemp -d)
cleanup() {
  status=$?
  rm -rf "$STAGE"
  exit $status
}
trap cleanup EXIT INT TERM

mkdir -p "$STAGE/src"

# The real, unmodified files under test — copied fresh, not forked.
cp "$WATER_SRC/shapes.mfb" "$STAGE/src/shapes.mfb"
cp "$WATER_SRC/toolbar.mfb" "$STAGE/src/toolbar.mfb"

cp "$SCRIPT_DIR/probe/main.mfb" "$STAGE/src/main.mfb"

cat > "$STAGE/project.json" <<'EOF'
{
  "name": "water-probe",
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
  "entry": "main",
  "targets": ["native"]
}
EOF

"$MFB_BIN" build "$STAGE" >"$STAGE/build.log" 2>&1 || {
  echo "FAIL: water-probe did not build" >&2
  cat "$STAGE/build.log" >&2
  exit 1
}

if [ "$(uname -s)" != "Darwin" ]; then
  echo "skip: water-probe is an --app build, only exercised on macOS here" >&2
  exit 0
fi

EXE="$STAGE/build/water-probe.app/Contents/MacOS/water-probe"

OUT=$(MFB_MACAPP_HEADLESS=1 perl -e '
  my $limit = shift @ARGV;
  my $pid = open(my $fh, "-|");
  if ($pid == 0) { exec($ARGV[0]) or exit 127; }
  local $SIG{ALRM} = sub { kill "KILL", $pid; exit 99; };
  alarm $limit; local $/; my $o = <$fh>; close($fh); print $o;
' 15 "$EXE")

echo "$OUT"

case "$OUT" in
  *"PASS: bug-716"*) exit 0 ;;
  *)
    echo "FAIL: water-probe did not report PASS" >&2
    exit 1
    ;;
esac
