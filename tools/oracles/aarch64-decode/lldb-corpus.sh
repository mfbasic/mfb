#!/bin/zsh
# Write lldb's disassembly of each image's __TEXT.__text as `<pc>\t<word>\t<text>`
# rows, the corpus format the AArch64 decoder's oracle test reads.
#
#   lldb-corpus.sh <out> <host-binary> <image>...
#
# <host-binary> is a program that loads the images (the dyld shared cache is
# read from the host, so `/bin/ls` reaches libSystem's dylibs and dyld); an
# <image> is a name `image list` shows, e.g. libsystem_c.dylib, or the host
# binary's own file name.
set -eu
if (( $# < 3 )); then
  echo "usage: $0 <out> <host-binary> <image>..." >&2
  exit 2
fi
out=$1 host=$2
shift 2
: > "$out"
for image in "$@"; do
  range=$(lldb -b -o "image dump sections $image" "$host" 2>/dev/null \
    | grep -E "__TEXT\.__text$" | head -1 | grep -oE '\[0x[0-9a-f]+-0x[0-9a-f]+\)' | tr -d '[)')
  if [[ -z $range ]]; then
    echo "$image: no __TEXT.__text in $host" >&2
    continue
  fi
  start=${range%-*} end=${range#*-}
  lldb -b -o "dis -b --force -s $start -c $(( (end - start) / 4 ))" "$host" 2>/dev/null \
    | perl -ne 'if (/(0x[0-9a-f]+)\]?(?: <[^>]*>)?:\s+0x([0-9a-f]{8})\s+(.*?)\s*$/ || /^\s*(?:->\s*)?(0x[0-9a-f]+):\s+0x([0-9a-f]{8})\s+(.*?)\s*$/) { my ($a, $w, $t) = ($1, $2, $3); $t =~ s/\s*;.*$//; print "$a\t$w\t$t\n"; }' \
    >> "$out"
  echo "$image: $(wc -l < "$out" | tr -d ' ') rows so far" >&2
done
