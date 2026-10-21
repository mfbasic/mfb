#!/bin/sh
# Write the corpus of the compiler's own instruction forms: objdump's text for
# the raw words `writes_the_encoder_s_words` wrote (every operation the riscv64
# encoder emits), in the format of objdump-corpus.sh.
#
#   encoder-corpus.sh <out> <words.bin>
#
# Run it on a riscv64 Linux box with GNU binutils. The words are wrapped in an
# ELF code section first, so objdump prints a jump's target as it does in a
# program (`58c`, not the `0x58c` it prints for a raw binary), and `-M max`
# turns on the vector extension, which objdump does not decode by default.
set -eu
if [ $# -ne 2 ]; then
  echo "usage: $0 <out> <words.bin>" >&2
  exit 2
fi
out=$1 words=$2
object=$(mktemp)
trap 'rm -f "$object"' EXIT
objcopy -I binary -O elf64-littleriscv -B riscv \
  --rename-section .data=.text,contents,code,alloc,load,readonly "$words" "$object"
OBJDUMP_ARGS='-M max' sh "$(dirname "$0")/objdump-corpus.sh" "$out" "$object"
