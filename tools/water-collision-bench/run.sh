#!/bin/sh
# run.sh — build and run water-collision-bench, the bug-717 regression +
# timing probe.
#
# This probe exercises the REAL examples/water/src/shapes.mfb — staged into
# a scratch project fresh on every run (mktemp -d + cp), never a committed
# copy. If shapes.mfb regresses, the next run stages the regressed file and
# this probe catches it.
#
# probe/main.mfb (the only committed file here) is:
#   - oracleContactWith: a FROZEN pre-fix contactWith, kept deliberately as a
#     fixed reference point (see README "The frozen oracle") — this one copy
#     is legitimate to keep static, since it exists to answer "does the real,
#     current contactWith still agree with the pre-fix behavior", not to
#     track the real file.
#   - the comparison/timing driver (probe-only glue).
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

# The real, unmodified file under test — copied fresh, not forked. It imports
# canvas/color (for its drawing section), which is why the scratch project
# below is "mode": "app" — this probe never calls a mode-gated canvas
# function, only builds Shape/Contact values and calls contactWith and
# friends, so no app::setMode call is needed at runtime.
cp "$WATER_SRC/shapes.mfb" "$STAGE/src/shapes.mfb"

cp "$SCRIPT_DIR/probe/main.mfb" "$STAGE/src/main.mfb"

cat > "$STAGE/project.json" <<'EOF'
{
  "name": "water-collision-bench",
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
  echo "FAIL: water-collision-bench did not build" >&2
  cat "$STAGE/build.log" >&2
  exit 1
}

if [ "$(uname -s)" = "Darwin" ]; then
  EXE="$STAGE/build/water-collision-bench.app/Contents/MacOS/water-collision-bench"
  OUT=$(MFB_MACAPP_HEADLESS=1 perl -e '
    my $limit = shift @ARGV;
    my $pid = open(my $fh, "-|");
    if ($pid == 0) { exec($ARGV[0]) or exit 127; }
    local $SIG{ALRM} = sub { kill "KILL", $pid; exit 99; };
    alarm $limit; local $/; my $o = <$fh>; close($fh); print $o;
  ' 30 "$EXE")
else
  echo "skip: water-collision-bench is an --app build, only exercised on macOS here" >&2
  exit 0
fi

echo "$OUT"

case "$OUT" in
  *"PASS: water-collision-bench"*) exit 0 ;;
  *)
    echo "FAIL: water-collision-bench found a mismatch or did not report PASS" >&2
    exit 1
    ;;
esac
