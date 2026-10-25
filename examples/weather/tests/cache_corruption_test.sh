#!/bin/sh
# cache_corruption_test.sh — regression test for bug-711.
#
# Proves that a cached GRIB file which passes the magic-byte check but does
# not decode is treated as this cycle's attempt failing (not an unrecoverable
# error), and that the corrupt file is deleted so it is not retried forever.
#
# Builds a small headless driver project (no `canvas`/`app` dependency, so it
# needs no display) out of verbatim copies of examples/weather/src/fetch.mfb and
# grib.mfb, plus a driver main.mfb that exercises attemptCycle directly
# against two synthetic cycles that can never collide with a real GFS cycle:
#
#   - a "corrupt" cycle: a GRIB-magic-prefixed but undecodable file is planted
#     at the real (shared) cache path attemptCycle/cachePath would compute.
#   - a "valid" cycle: this test's own committed fixture
#     (tests/fixtures/valid-analysis.grb2 — a real, previously-fetched GFS
#     analysis hour) is planted at its cache path.
#   - a "zero-length" cycle: a file whose only message is "GRIB", edition 2
#     and a declared message length of 0. decodeMessage once accepted that as
#     an empty field of length 0, so decodeFile never advanced and appended
#     fields until allocation failed (about 20 seconds and 32 GB later on a
#     96 GB machine). It must be refused by the decoder itself, with a "grib:"
#     error, like any other corrupt file. The driver also runs under a time
#     limit, so a regression on a smaller machine fails rather than stalls.
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
WEATHER_SRC="$SCRIPT_DIR/../src"
FIXTURE="$SCRIPT_DIR/fixtures/valid-analysis.grb2"

WORK=$(mktemp -d)
cleanup() {
  status=$?
  rm -rf "$WORK"
  # Best-effort: remove anything this test planted in the shared cache
  # directory, whether or not the fix already deleted it.
  if [ -n "${CORRUPT_PATH:-}" ]; then rm -f "$CORRUPT_PATH"; fi
  if [ -n "${VALID_PATH:-}" ]; then rm -f "$VALID_PATH"; fi
  if [ -n "${ZERO_PATH:-}" ]; then rm -f "$ZERO_PATH"; fi
  exit $status
}
trap cleanup EXIT INT TERM

mkdir -p "$WORK/src"
cp "$WEATHER_SRC/fetch.mfb" "$WORK/src/fetch.mfb"
cp "$WEATHER_SRC/grib.mfb" "$WORK/src/grib.mfb"

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
IMPORT strings

TYPE Winds
  east AS Field
  north AS Field
  east850 AS Field
  north850 AS Field
  east250 AS Field
  north250 AS Field
  temperature AS Field
  precipitation AS Field
  label AS String
END TYPE

FUNC emptyField AS Field
  RETURN Field[ni := 0, nj := 0, lat1 := 0.0, lon1 := 0.0, dlat := 0.0, dlon := 0.0, category := 0, parameter := 0, surface := 0, height := 0, interval := FALSE, values := []]
END FUNC

FUNC main AS Nothing
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

  LET zeroCycle AS Cycle = Cycle[day := "19990101", hour := "12", caption := "bug711-zero-length", seconds := 43200]
  LET zeroPath AS String = cachePath(zeroCycle, 0)
  LET zeroBytes AS List OF Byte = [71, 82, 73, 66, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
  fs::writeBytesAtomic(zeroPath, zeroBytes)
  LET zeroAttempt AS Attempt = attemptCycle(zeroCycle, 0)
  io::print("ZERO_PROBLEM_EMPTY=" & toString(zeroAttempt.problem = ""))
  io::print("ZERO_REFUSED_BY_DECODER=" & toString(strings::startsWith(zeroAttempt.problem, "grib:")))
  io::print("ZERO_FILE_GONE=" & toString(NOT fs::fileExists(zeroPath)))
END FUNC
EOF

# Compute the two paths this test plants (and must clean up) the same way
# cachePath does, without needing a second run: gfs-<day>-<hour>-<hourTag>-<suffix>.
TMPDIR_RESOLVED=${TMPDIR:-/tmp}
CACHE_DIR="${TMPDIR_RESOLVED%/}/mfb-weather-gfs"
CORRUPT_PATH="$CACHE_DIR/gfs-19990101-00-f000-wind-wind850-wind250-temp-rain-0p50.grb2"
VALID_PATH="$CACHE_DIR/gfs-19990101-06-f000-wind-wind850-wind250-temp-rain-0p50.grb2"
ZERO_PATH="$CACHE_DIR/gfs-19990101-12-f000-wind-wind850-wind250-temp-rain-0p50.grb2"

"$MFB_BIN" build "$WORK" >"$WORK/build.log" 2>&1 || {
  echo "FAIL: driver did not build" >&2
  cat "$WORK/build.log" >&2
  exit 1
}

# Run the driver with a 30-second limit: a decoder that stops advancing loops
# forever rather than failing.
"$WORK/build/bug711check.out" >"$WORK/out.txt" 2>&1 &
DRIVER=$!
WAITED=0
while kill -0 "$DRIVER" 2>/dev/null; do
  if [ "$WAITED" -ge 30 ]; then
    kill -9 "$DRIVER" 2>/dev/null || true
    echo "FAIL: the driver did not finish within 30 seconds" >&2
    cat "$WORK/out.txt" >&2
    exit 1
  fi
  sleep 1
  WAITED=$((WAITED + 1))
done
wait "$DRIVER" || {
  echo "FAIL: the driver exited with an error" >&2
  cat "$WORK/out.txt" >&2
  exit 1
}
OUT=$(cat "$WORK/out.txt")
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
# The zero-length cycle: rejected like any other undecodable file, and deleted.
check ZERO_PROBLEM_EMPTY FALSE
check ZERO_REFUSED_BY_DECODER TRUE
check ZERO_FILE_GONE TRUE

if [ "$FAIL" -ne 0 ]; then
  echo "FAIL: bug-711 regression test failed" >&2
  exit 1
fi

echo "PASS: bug-711 — a corrupt cached file (including one whose message declares a length of 0) is treated as this cycle's attempt failing and is deleted; a valid cached file still decodes and is kept"
