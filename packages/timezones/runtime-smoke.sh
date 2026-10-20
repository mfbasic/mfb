#!/usr/bin/env bash
#
# runtime-smoke.sh — prove the package links. Build an executable that imports
# ONLY `io` and `timezones` against the compiled .mfp, run it, and check what it
# printed.
#
# The package's own tests run inside the package. This is the only check that a
# consumer can read a zoned string, hold the datetime::DateTime the package hands
# back, pass its fields back in, name an exported enum member, and write it
# again. Reading those fields needs the consumer's own `IMPORT datetime`: imports
# are not transitive, and a package does not export the builtin types it uses
# (`mfb spec language modules-and-packages`; bug-466, bug-750).
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
printf '%s\n' '{"name":"timezones_runtime_smoke","version":"0.1.0","mfb":"1.0","kind":"executable","sources":[{"root":"src","role":"main","include":["**/*.mfb"]}],"entry":"main","packages":[{"name":"timezones","version":"=1.0.0","source":"file:packages/timezones.mfp","direct":true,"requiredBy":[]}]}' > "$WORK/project.json"
cat > "$WORK/src/main.mfb" <<'MFB'
IMPORT datetime
IMPORT io
IMPORT timezones

FUNC main() AS Nothing
  LET z = timezones::parseIso("2026-07-15T09:00:00.000-04:00[America/New_York]")
  io::print(timezones::toIso(z.dateTime, z.name))
  LET again = timezones::civil(z.dateTime.date, z.dateTime.time, z.name)
  io::print(timezones::toIso(again, 9, z.name))
  LET skipped = timezones::civil(datetime::date(2026, 3, 8), datetime::time(2, 30, 0, 0), "America/New_York", timezones::Disambiguation.LATER)
  io::print(timezones::toIso(skipped, z.name))
  io::print(timezones::tzdbVersion())
END FUNC
MFB
"$MFB" build -q "$WORK"
(cd "$WORK" && "$WORK/build/timezones_runtime_smoke.out" > "$WORK/output.txt")

lines="$(wc -l < "$WORK/output.txt" | tr -d ' ')"
if [ "$lines" != 4 ]; then
  fail "expected 4 lines of output, got $lines:
$(cat "$WORK/output.txt")"
fi
line1="$(sed -n '1p' "$WORK/output.txt")"
line2="$(sed -n '2p' "$WORK/output.txt")"
line3="$(sed -n '3p' "$WORK/output.txt")"
line4="$(sed -n '4p' "$WORK/output.txt")"

# Read and written back unchanged, zone and all.
if [ "$line1" != "2026-07-15T09:00:00.000-04:00[America/New_York]" ]; then
  fail "parseIso -> toIso did not reproduce the input: $line1"
fi

# The same wall clock read again in the zone, written with every nanosecond.
if [ "$line2" != "2026-07-15T09:00:00.000000000-04:00[America/New_York]" ]; then
  fail "civil -> toIso(9) is wrong: $line2"
fi

# A skipped reading resolved Later moves back by the gap: 02:30 becomes 01:30 EST.
if [ "$line3" != "2026-03-08T01:30:00.000-05:00[America/New_York]" ]; then
  fail "civil with Disambiguation.LATER is wrong: $line3"
fi

# The release the package was built from.
if [ "$line4" != "2026e" ]; then
  fail "tzdbVersion() is not 2026e: $line4"
fi

echo "timezones runtime smoke passed"
