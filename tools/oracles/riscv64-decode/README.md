# tools/oracles/riscv64-decode

GNU `objdump` as the oracle for the riscv64 decoder (`src/arch/riscv64/decode`),
which `mfb dbg` uses for `dis`, `finish`, the unwinder and software stepping on
riscv64 Linux. The decoder spells every instruction as `objdump` does with its
default aliases and ABI register names, so `objdump`'s disassembly of real code
is the reference. Xcode's `llvm-objdump` has no riscv64 target, so the corpora
are written on a riscv64 Linux box (2232 has GNU binutils) and the test runs on
the Mac.

- **objdump-corpus.sh** — writes `objdump -d`'s disassembly of each ELF as
  `<pc>\t<bytes>\t<text>` rows: `<bytes>` as `objdump` prints them (8 hex digits
  for a 4-byte instruction, 4 for a 2-byte one), the `# …` comment and the
  `<symbol>` annotations dropped.

      # on box 2232, with the files to read copied there
      sh objdump-corpus.sh /tmp/rv-corpus/libc.txt /lib/riscv64-linux-gnu/libc.so.6
      # on the Mac, with the corpus copied back
      DECODE_CORPUS=/tmp/rv-corpus/libc.txt cargo test --bin mfb \
          decodes_a_corpus_as_objdump_does -- --ignored --nocapture

  The test fails on any row the decoder spells differently from `objdump`, and
  prints the rows it leaves undecoded, by mnemonic.

- **encoder-corpus.sh** — the same rows for the compiler's own instruction
  forms: every operation the riscv64 encoder emits, as the ignored test
  `writes_the_encoder_s_words` writes them to a raw file. The words are wrapped
  in an ELF code section with `objcopy` (so a jump's target prints as `58c`,
  not the `0x58c` `objdump` prints for a raw binary) and read with `-M max`,
  since `objdump` does not decode the vector extension by default; the decoder
  spells vector forms as `-M max` does.

      # on the Mac
      ENCODER_WORDS=/tmp/encoder.bin cargo test --bin mfb \
          writes_the_encoder_s_words -- --ignored
      # on box 2232, with encoder.bin and both scripts copied there
      sh encoder-corpus.sh /tmp/rv-corpus/encoder.txt /tmp/encoder.bin

- **sample-words.py** — picks the rows of `src/arch/riscv64/decode/objdump_words.txt`
  from corpora: up to three operand shapes for each mnemonic, instruction length
  and opcode class, so every compressed encoding that prints as the same base
  mnemonic (`c.addi`, `c.addi16sp`, `c.addi4spn` all print `addi`) is kept.
  `decodes_words_as_objdump_does` checks every row on every test run. Regenerate
  it the same way when the decoder learns new forms:

      python3 sample-words.py /tmp/rv-corpus/*.txt > src/arch/riscv64/decode/objdump_words.txt

Measured with GNU objdump (GNU Binutils for Debian) 2.44 on box 2232, on
2026-10-20, with no row decoded differently from `objdump` and none left
undecoded in any corpus:

| Corpus | Where it comes from | Rows | Differing | Undecoded |
|---|---|---|---|---|
| `debug-glibc.txt` | `examples/debug` built with `mfb build --debug -O0 --target linux-riscv64` | 25,585 | 0 | 0 |
| `libc.txt` | glibc 2.41's `/lib/riscv64-linux-gnu/libc.so.6` on box 2232 | 304,077 | 0 | 0 |
| `ld-musl.txt` | musl's `/lib/ld-musl-riscv64.so.1` (libc and loader in one) from box 2229 | 106,156 | 0 | 0 |
| `encoder.txt` | every operation the riscv64 encoder emits (`encoder-corpus.sh`) | 538 | 0 | 0 |

`objdump` itself left no row of them undecoded (`grep -cE '\t\.(insn|word|short)'`
→ 0 in each), so every row was compared. The four hold 227 distinct mnemonics;
`objdump_words.txt` samples all of them in 671 rows.
