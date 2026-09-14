#!/bin/sh
# run.sh <browser.out> <url> <runs> — time <runs> page loads back to back and print the
# median (see README.md).
set -eu
if [ "$#" -ne 3 ]; then
  echo "usage: $0 <browser.out> <url> <runs>" >&2
  exit 2
fi
prog=$1
url=$2
runs=$3
here=$(cd "$(dirname "$0")" && pwd)
times=""
i=1
while [ "$i" -le "$runs" ]; do
  line=$(expect -f "$here/load.exp" "$prog" "$url")
  echo "run $i $line"
  ms=$(printf '%s\n' "$line" | sed -n 's/^load_ms=\([0-9][0-9]*\) .*/\1/p')
  if [ -n "$ms" ]; then
    times="$times $ms"
  fi
  i=$((i + 1))
done
count=$(printf '%s\n' $times | grep -c . || true)
if [ "$count" -eq 0 ]; then
  echo "median load_ms=- over 0 runs"
  exit 1
fi
median=$(printf '%s\n' $times | sort -n | awk '{a[NR]=$1} END {if (NR % 2) print a[(NR+1)/2]; else print int((a[NR/2] + a[NR/2+1]) / 2)}')
echo "median load_ms=$median over $count runs"
