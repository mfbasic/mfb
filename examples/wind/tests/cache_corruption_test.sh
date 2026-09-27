#!/bin/sh
# cache_corruption_test.sh — regression test for bug-711.
#
# Proves that a cached GRIB file which passes the magic-byte check but does
# not decode is treated as this cycle's attempt failing (not an unrecoverable
# error), and that the corrupt file is deleted so it is not retried forever.
#
# Builds a small headless driver project (no `canvas`/`app` dependency, so it
# needs no display) out of verbatim copies of examples/wind/src/fetch.mfb and
# grib.mfb, plus a driver main.mfb that exercises attemptCycle directly
# against two synthetic cycles that can never collide with a real GFS cycle:
#
#   - a "corrupt" cycle: a GRIB-magic-prefixed but undecodable file is planted
#     at the real (shared) cache path attemptCycle/cachePath would compute.
#   - a "valid" cycle: this test's own committed fixture
#     (tests/fixtures/valid-analysis.grb2 — a real, previously-fetched GFS
#     analysis hour) is planted at its cache path.
#
# No network access is required or attempted: cycleData only calls
# http::read when no cache file exists, and both synthetic cycles always have
# one planted before attemptCycle runs.
#
# Usage: cache_corruption_test.sh <path-to-mfb-binary>

set -eu

MFB_BIN="${1:-}"
if [ -z "$MFB_BIN" ]; then
  echo "usage: cache_corruption_test.sh <path-to-mfb-binary>" >&2
  exit 2
fi
MFB_BIN=$(cd "$(dirname "$MFB_BIN")" && pwd)/$(basename "$MFB_BIN")

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
WIND_SRC="$SCRIPT_DIR/../src"
FIXTURE="$SCRIPT_DIR/fixtures/valid-analysis.grb2"

WORK=$(mktemp -d)
cleanup() {
  status=$?
  rm -rf "$WORK"
  # Best-effort: remove anything this test planted in the shared cache
  # directory, whether or not the fix already deleted it.
  if [ -n "${CORRUPT_PATH:-}" ]; then rm -f "$CORRUPT_PATH"; fi
  if [ -n "${VALID_PATH:-}" ]; then rm -f "$VALID_PATH"; fi
  exit $status
}
trap cleanup EXIT INT TERM

mkdir -p "$WORK/src"
cp "$WIND_SRC/fetch.mfb" "$WORK/src/fetch.mfb"
cp "$WIND_SRC/grib.mfb" "$WORK/src/grib.mfb"

cat > "$WORK/project.json" <<'EOF'
{
  "name": "bug711check",
  "version": "0.1.0",
  "mfb": "1.0",
  "kind": "executable",
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

# A stand-in TYPE Winds mirroring flow.mfb's real one (same field names/
# types), so fetch.mfb's Winds[...] constructor resolves without pulling in
# flow.mfb's canvas/color imports (which would force app mode and block a
# headless build). emptyField mirrors main.mfb's own helper, which fetch.mfb's
# emptyWinds() calls.
cat > "$WORK/src/main.mfb" <<EOF
IMPORT collections
IMPORT fs
IMPORT io

TYPE Winds
  east AS Field
  north AS Field
  temperature AS Field
  precipitation AS Field
  label AS String
END TYPE

FUNC emptyField AS Field
  RETURN Field[ni := 0, nj := 0, lat1 := 0.0, lon1 := 0.0, dlat := 0.0, dlon := 0.0, category := 0, parameter := 0, surface := 0, height := 0, interval := FALSE, values := []]
END FUNC

SUB main
  LET corruptCycle AS Cycle = Cycle[day := "19990101", hour := "00", caption := "bug711-corrupt", seconds := 0]
  LET validCycle AS Cycle = Cycle[day := "19990101", hour := "06", caption := "bug711-valid", seconds := 21600]

  LET corruptPath AS String = cachePath(corruptCycle, 0)
  LET validPath AS String = cachePath(validCycle, 0)

  MUT corruptBytes AS List OF Byte = [71, 82, 73, 66]
  FOR i = 0 TO 1999
    corruptBytes = collections::append(corruptBytes, toByte((i * 7919) MOD 256))
  NEXT
  fs::writeBytesAtomic(corruptPath, corruptBytes)

  LET fixture AS List OF Byte = fs::readBytes("$FIXTURE")
  fs::writeBytesAtomic(validPath, fixture)

  LET corruptAttempt AS Attempt = attemptCycle(corruptCycle, 0)
  io::print("CORRUPT_PROBLEM_EMPTY=" & toString(corruptAttempt.problem = ""))
  io::print("CORRUPT_FILE_GONE=" & toString(NOT fs::fileExists(corruptPath)))

  LET validAttempt AS Attempt = attemptCycle(validCycle, 0)
  io::print("VALID_PROBLEM_EMPTY=" & toString(validAttempt.problem = ""))
  io::print("VALID_LABEL_OK=" & toString(validAttempt.winds.label = "GFS surface weather  bug711-valid"))
  io::print("VALID_FILE_KEPT=" & toString(fs::fileExists(validPath)))
END SUB
EOF

# Compute the two paths this test plants (and must clean up) the same way
# cachePath does, without needing a second run: gfs-<day>-<hour>-<hourTag>-<suffix>.
TMPDIR_RESOLVED=${TMPDIR:-/tmp}
CACHE_DIR="${TMPDIR_RESOLVED%/}/mfb-wind-gfs"
CORRUPT_PATH="$CACHE_DIR/gfs-19990101-00-f000-wind-temp-rain-0p50.grb2"
VALID_PATH="$CACHE_DIR/gfs-19990101-06-f000-wind-temp-rain-0p50.grb2"

"$MFB_BIN" build "$WORK" >"$WORK/build.log" 2>&1 || {
  echo "FAIL: driver did not build" >&2
  cat "$WORK/build.log" >&2
  exit 1
}

OUT=$("$WORK/build/bug711check.out")
echo "$OUT"

FAIL=0
check() {
  case "$OUT" in
    *"$1=$2"*) ;;
    *) echo "FAIL: expected $1=$2" >&2; FAIL=1 ;;
  esac
}

# The corrupt cycle: attemptCycle must report a problem (the decode failed,
# not silently succeeded) and the poisoned cache file must be gone.
check CORRUPT_PROBLEM_EMPTY FALSE
check CORRUPT_FILE_GONE TRUE
# The valid cycle: attemptCycle must succeed, decode the real winds, and
# leave the (good) cache file in place.
check VALID_PROBLEM_EMPTY TRUE
check VALID_LABEL_OK TRUE
check VALID_FILE_KEPT TRUE

if [ "$FAIL" -ne 0 ]; then
  echo "FAIL: bug-711 regression test failed" >&2
  exit 1
fi

echo "PASS: bug-711 — a corrupt cached file is treated as this cycle's attempt failing and is deleted; a valid cached file still decodes and is kept"
