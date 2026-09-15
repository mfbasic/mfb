#!/usr/bin/env bash
# Throughput of the `compress` builtin beside Python's zlib. See README.md.
#
# Usage: tools/compress-bench/run.sh <path-to-mfb> [op ...]
# Env:   OPT_LEVELS (default "1 3")
set -euo pipefail
[ $# -ge 1 ] || { echo "usage: run.sh <path-to-mfb> [op ...]" >&2; exit 2; }
exec python3 "$(dirname "$0")/bench.py" "$@"
