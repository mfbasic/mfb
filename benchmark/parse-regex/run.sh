#!/usr/bin/env bash
# Uses regex::findAll to count digit runs in a string. mfb vs Python only — C's
# POSIX regex isn't a like-for-like comparison. The input is generated once into
# a temp file so the benchmark times matching, not input construction.
#
# NOTE: the MFBASIC-source regex engine is very slow (superlinear in input
# length), so the input is only 200 numbers. Override iterations with BENCH_RUNS.
set -euo pipefail
: "${BENCH_RUNS:=100}"
export BENCH_RUNS
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$here/../runner.sh"

input=/tmp/mfb-bench-parse-regex.txt
python3 -c "
with open('$input', 'w') as f:
    f.write(' '.join(str(i) for i in range(200)))
"
BENCH_ARTIFACTS+=("$input")

bench_build_mfb "$here/mfb"

echo "parse-regex — regex::findAll digit runs in 200 numbers:"
time_run "mfb"    "$MFB_OUT"
time_run "python" python3 "$here/python/main.py"
