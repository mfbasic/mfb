#!/usr/bin/env bash
#
# runtime-smoke.sh — prove the package links. Build an executable that imports
# ONLY `io` and `timezones` against the compiled .mfp, run it, and check what it
# printed.
#
# The package's own tests run inside the package. This is the only check that a
# consumer who never writes `IMPORT datetime` can read a zoned string, hold the
# datetime::DateTime the package hands back, pass its fields back in, and write
# it again.
#
#     ./runtime-smoke.sh [path/to/mfb]

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
MFB="${1:-$ROOT/target/release/mfb}"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# `set -e` does not abort on a failing `[[ ]]` under the bash 3.2 that macOS
# ships, so every assertion says so itself. Never write a bare `[[ ]]` here.
fail() { echo "FAIL: $*" >&2; exit 1; }

"$MFB" build -q "$ROOT/packages/timezones"
mkdir -p "$WORK/src" "$WORK/packages"
cp "$ROOT/packages/timezones/timezones.mfp" "$WORK/packages/timezones.mfp"
printf '%s\n' '{"name":"timezones_runtime_smoke","version":"0.1.0","mfb":"1.0","kind":"executable","sources":[{"root":"src","role":"main","include":["**/*.mfb"]}],"entry":"main","packages":[{"name":"timezones","version":"=0.1.0","source":"file:packages/timezones.mfp"}]}' > "$WORK/project.json"
cat > "$WORK/src/main.mfb" <<'MFB'
IMPORT io
IMPORT timezones

SUB main()
  LET z = timezones::parseIso("2026-07-15T09:00:00.000-04:00[America/New_York]")
  io::print(timezones::toIso(z.dateTime, z.name))
  LET again = timezones::civil(z.dateTime.date, z.dateTime.time, z.name)
  io::print(timezones::toIso(again, 9, z.name))
END SUB
MFB
"$MFB" build -q "$WORK"
(cd "$WORK" && "$WORK/build/timezones_runtime_smoke.out" > "$WORK/output.txt")

lines="$(wc -l < "$WORK/output.txt" | tr -d ' ')"
if [ "$lines" != 2 ]; then
  fail "expected 2 lines of output, got $lines:
$(cat "$WORK/output.txt")"
fi
line1="$(sed -n '1p' "$WORK/output.txt")"
line2="$(sed -n '2p' "$WORK/output.txt")"

# Read and written back unchanged, zone and all.
if [ "$line1" != "2026-07-15T09:00:00.000-04:00[America/New_York]" ]; then
  fail "parseIso -> toIso did not reproduce the input: $line1"
fi

# The same wall clock read again in the zone, written with every nanosecond.
if [ "$line2" != "2026-07-15T09:00:00.000000000-04:00[America/New_York]" ]; then
  fail "civil -> toIso(9) is wrong: $line2"
fi

echo "timezones runtime smoke passed"
