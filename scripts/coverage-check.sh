#!/usr/bin/env sh
# Per-file coverage gate. Reads the profile left by scripts/coverage.sh (run it
# first) and prints every in-scope source file below the FLOOR, exiting non-zero
# if any fall short. This is the local equivalent of the CI per-file gate; the
# global floor is enforced separately by `cargo llvm-cov report
# --fail-under-lines`.
#
# FLOOR defaults to 95 (the plan-12 bar); override with e.g. FLOOR=0 to only
# print a report without gating during bring-up.
set -eu

cd "$(dirname "$0")/.."

FLOOR="${FLOOR:-95}"
. ./scripts/coverage-common.sh

# Regenerate the JSON summary from the cached profile (no test re-run).
cargo llvm-cov report $PKG_FLAGS \
  --ignore-filename-regex "$IGNORE" \
  --json --output-path target/coverage/coverage.json >/dev/null

FLOOR="$FLOOR" python3 - "$@" <<'PY'
import json, os, sys

floor = float(os.environ["FLOOR"])
with open("target/coverage/coverage.json") as f:
    data = json.load(f)

cwd = os.getcwd() + "/"
def rel(p):
    return p[len(cwd):] if p.startswith(cwd) else p

# Documented exceptions: files exempt from the per-file gate (Tier-C/D lines
# covered only by the integration harness). Parsed from coverage-exceptions.txt.
exceptions = set()
exc_path = "scripts/coverage-exceptions.txt"
if os.path.exists(exc_path):
    with open(exc_path) as f:
        for line in f:
            line = line.split("#", 1)[0].strip()
            if line:
                exceptions.add(line)

files = data["data"][0]["files"]
# Optional trailing args: only report files whose path contains one of them.
filters = sys.argv[1:]

rows = []
for entry in files:
    name = entry["filename"]
    if filters and not any(g in name for g in filters):
        continue
    lines = entry["summary"]["lines"]
    rows.append((lines["percent"], name, lines["covered"], lines["count"]))

rows.sort()
below = [r for r in rows if r[0] < floor and r[3] > 0 and rel(r[1]) not in exceptions]
excused = [r for r in rows if r[0] < floor and r[3] > 0 and rel(r[1]) in exceptions]

if below:
    print(f"Files below {floor:.0f}% line coverage (GATE FAILURE):")
    for pct, name, covered, total in below:
        print(f"  {pct:6.2f}%  ({covered}/{total})  {rel(name)}")
else:
    print(f"All non-excepted files >= {floor:.0f}% line coverage.")

if excused:
    print(f"\nDocumented exceptions below {floor:.0f}% (integration-covered):")
    for pct, name, covered, total in excused:
        print(f"  {pct:6.2f}%  ({covered}/{total})  {rel(name)}")

overall = data["data"][0]["totals"]["lines"]["percent"]
print(f"\nOverall line coverage: {overall:.2f}%  ({len(rows)} files"
      + (f", {len(filters)} filter(s)" if filters else "")
      + f", {len(excused)} excepted)")

sys.exit(1 if below else 0)
PY
