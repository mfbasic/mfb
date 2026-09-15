#!/usr/bin/env bash
# The zlib behaviour probe: how Python's and Node's zlib treat hand-built edge streams
# (python/probe_streams.py) — trailing bytes, header flags, bad trailers, and code sets at
# the edges of zlib's validity rules. It measures the judges, not the MFB decoder: its
# table is what the decoder's strictness rules and the oracle's declared divergences cite.
#
# Usage: tools/oracles/compress/probe.sh
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
WORK=$(mktemp -d "${TMPDIR:-/tmp}/mfb-probe.XXXXXX")
trap 'rm -rf "$WORK"' EXIT

count=$(cd "$HERE/python" && python3 gen.py probe "$WORK/probe.job")
python3 "$HERE/python/oracle.py" probe "$WORK/probe.job" >"$WORK/python.txt"
node "$HERE/node/oracle.mjs" probe "$WORK/probe.job" >"$WORK/node.txt"
for f in python node; do
  n=$(wc -l <"$WORK/$f.txt" | tr -d ' ')
  [ "$n" = "$count" ] || { echo "$f answered $n of $count probe cases" >&2; exit 2; }
done

echo "python zlib $(python3 -c 'import zlib; print(zlib.ZLIB_RUNTIME_VERSION)'), node zlib $(node -e 'console.log(process.versions.zlib)')"
paste -d '|' "$WORK/probe.job.names" "$WORK/python.txt" "$WORK/node.txt" |
  while IFS='|' read -r names py nd; do
    printf '%-34s python: %s\n%-34s node:   %s\n' "$names" "${py#case * }" "" "${nd#case * }"
  done
