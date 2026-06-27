#!/usr/bin/env bash
# Parses a 2000-row CSV of integers and sums the cells. mfb vs Python only —
# C has no standard-library CSV parser. The input is generated once into a temp
# file so the benchmark times parsing, not input construction.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$here/../runner.sh"

input=/tmp/mfb-bench-parse-csv.csv
python3 -c "
with open('$input', 'w') as f:
    for i in range(2000):
        f.write('%d,%d,%d\n' % (i, i + 1, i + 2))
"
BENCH_ARTIFACTS+=("$input")

bench_build_mfb "$here/mfb"

echo "parse-csv — parse a 2000-row CSV and sum cells:"
time_run "mfb"    "$MFB_OUT"
time_run "python" python3 "$here/python/main.py"
