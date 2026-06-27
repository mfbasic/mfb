#!/usr/bin/env bash
# Parses a JSON object with a 5000-element array. mfb vs Python only — C has no
# standard-library JSON parser. The input is generated once into a temp file so
# the benchmark times parsing, not input construction.
#
# NOTE: json::parse builds its tree on mfb's collections, which makes parsing a
# 5000-element array slow (~0.6s), so the runner defaults to few iterations.
set -euo pipefail
: "${BENCH_RUNS:=30}"
export BENCH_RUNS
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$here/../runner.sh"

input=/tmp/mfb-bench-parse-json.json
python3 -c "
nums = ','.join(str(i) for i in range(5000))
with open('$input', 'w') as f:
    f.write('{\"nums\":[' + nums + '],\"tail\":5000}')
"
BENCH_ARTIFACTS+=("$input")

bench_build_mfb "$here/mfb"

echo "parse-json — parse a JSON object with a 5000-element array:"
time_run "mfb"    "$MFB_OUT"
time_run "python" python3 "$here/python/main.py"
