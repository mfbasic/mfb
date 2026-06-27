#!/usr/bin/env bash
# Reads 100000 lines from a file (via stdin) line by line, counting lines and
# bytes.
#
# NOTE: mfb's io::readLine reads one byte per syscall (unbuffered), so reading
# 100k lines is slow (~0.3s) — slower than Python/C here. The runner therefore
# defaults to few iterations.
set -euo pipefail
: "${BENCH_RUNS:=30}"
export BENCH_RUNS
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$here/../runner.sh"

input=/tmp/mfb-bench-io-read.txt
seq 0 99999 > "$input"
BENCH_ARTIFACTS+=("$input")

bench_build_mfb "$here/mfb"
bench_build_c   "$here/c" io-read

mfb_read() { "$MFB_OUT" < "$input"; }
py_read()  { python3 "$here/python/main.py" < "$input"; }
c0_read()  { "$here/c/io-read-O0.out" < "$input"; }
c2_read()  { "$here/c/io-read-O2.out" < "$input"; }

echo "io-read — read 100000 lines from stdin, count lines + bytes:"
time_run "mfb"    mfb_read
time_run "python" py_read
time_run "c -O0"  c0_read
time_run "c -O2"  c2_read
