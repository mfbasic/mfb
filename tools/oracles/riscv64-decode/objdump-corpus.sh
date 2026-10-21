#!/bin/sh
# Write GNU objdump's disassembly of each ELF's code as `<pc>\t<bytes>\t<text>`
# rows, the corpus format the riscv64 decoder's oracle test reads.
#
#   objdump-corpus.sh <out> <file>...
#
# Run it on a riscv64 Linux box (objdump must know riscv64). <bytes> is the
# instruction as objdump prints it: 8 hex digits for a 4-byte instruction, 4
# for a 2-byte one, most significant first. <text> is the mnemonic, a tab and
# the operands, as objdump spells them with its default aliases and ABI register
# names; the `# …` comment and every `<symbol>` annotation are dropped.
# Rows objdump itself cannot decode (`.insn`, `.word`, `.short`) are kept: the
# test skips them.
set -eu
if [ $# -lt 2 ]; then
  echo "usage: $0 <out> <file>..." >&2
  exit 2
fi
out=$1
shift
: > "$out"
for file in "$@"; do
  objdump -d -w "$file" \
    | perl -ne 'if (/^\s*([0-9a-f]+):\t([0-9a-f]+)\s+\t(.*?)\s*$/) { my ($a, $w, $t) = ($1, $2, $3); $t =~ s/\s*#.*$//; $t =~ s/\s*<[^>]*>//g; print "$a\t$w\t$t\n"; }' \
    >> "$out"
  echo "$file: $(wc -l < "$out" | tr -d ' ') rows so far" >&2
done
