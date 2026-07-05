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
IGNORE='(^|/)(target|tests)/|repository/target/|_runtime_tables\.rs$|/code/private/unicode\.rs$|/src/testutil\.rs$'

# Regenerate the JSON summary from the cached profile (no test re-run).
cargo llvm-cov report \
  --ignore-filename-regex "$IGNORE" \
  --json --output-path target/coverage/coverage.json >/dev/null

FLOOR="$FLOOR" python3 - "$@" <<'PY'
import json, os, sys

floor = float(os.environ["FLOOR"])
with open("target/coverage/coverage.json") as f:
    data = json.load(f)

files = data["data"][0]["files"]
# Optional trailing args: only report files whose path contains one of them.
filters = sys.argv[1:]

rows = []
for entry in files:
    name = entry["filename"]
    if filters and not any(g in name for g in filters):
        continue
    lines = entry["summary"]["lines"]
    pct = lines["percent"]
    covered = lines["covered"]
    total = lines["count"]
    rows.append((pct, name, covered, total))

rows.sort()
below = [r for r in rows if r[0] < floor and r[3] > 0]

# Repo-relative display paths.
cwd = os.getcwd() + "/"
def rel(p):
    return p[len(cwd):] if p.startswith(cwd) else p

if below:
    print(f"Files below {floor:.0f}% line coverage:")
    for pct, name, covered, total in below:
        print(f"  {pct:6.2f}%  ({covered}/{total})  {rel(name)}")
else:
    print(f"All in-scope files >= {floor:.0f}% line coverage.")

overall = data["data"][0]["totals"]["lines"]["percent"]
print(f"\nOverall line coverage: {overall:.2f}%  ({len(rows)} files"
      + (f", {len(filters)} filter(s)" if filters else "") + ")")

sys.exit(1 if below else 0)
PY
