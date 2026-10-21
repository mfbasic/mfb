# tools/oracles/aarch64-decode

lldb as the oracle for the AArch64 decoder (`src/arch/aarch64/decode`), which
`mfb dbg` uses for `dis`. The decoder spells every instruction as lldb does on
macOS, so lldb's own disassembly of real code is the reference.

- **lldb-corpus.sh** — writes lldb's disassembly of each image's `__TEXT.__text`
  as `<pc>\t<word>\t<text>` rows (comments after `;` dropped). Needs macOS on
  Apple silicon and lldb.

      zsh tools/oracles/aarch64-decode/lldb-corpus.sh /tmp/sys.txt /bin/ls \
          dyld libsystem_c.dylib libsystem_kernel.dylib libsystem_m.dylib \
          libcorecrypto.dylib libdispatch.dylib
      DECODE_CORPUS=/tmp/sys.txt cargo test -p mfb --bin mfb \
          decodes_a_corpus_as_lldb_does -- --ignored --nocapture

  The test fails on any word the decoder spells differently from lldb, and
  prints the words it leaves undecoded, by mnemonic. The images reachable from
  a host binary are those `image list` shows for it; `/usr/bin/security`
  reaches `CoreFoundation` and `Security`, and an MFBASIC program built with
  `mfb build` is its own host (`<out> build/app.out app.out`).

`src/arch/aarch64/decode/lldb_words.txt` is a sample of such corpora — up to
three operand shapes per mnemonic — that `decodes_words_as_lldb_does` checks on
every test run. Regenerate it the same way when the decoder learns new forms.

Measured with lldb-1703.0.236.103 on macOS 15.7.7, with no word decoded
differently from lldb in any of these corpora:

| Corpus | Words | Undecoded |
|---|---|---|
| `/bin/ls`: libsystem_platform, _kernel, _c, _malloc, _pthread, libdyld, libsystem_m, libcorecrypto, libdispatch, libxpc, libsystem_trace, _info | 655,885 | 245: SVE and SME, and 19 of rarer extensions (MTE, `cpy`/`set`, by-element dot products, `fcmla`, `luti2`, generic `msr`/`msrr`) |
| `/usr/bin/security`: CoreFoundation, Security, libsqlite3; `/bin/ls`: dyld; `/usr/bin/sqlite3`: libz | 1,945,804 | 5: SVE |
| five MFBASIC examples (`life`, `snake`, `money-demo`, `fractals`, `sorts`) | 1,868,623 | 0 |

Word counts leave out lldb's `.long` rows (data in the text section).

## Sweeping beyond the sample

`lldb_words.txt` is a sample; the decoder's rarer arms (scalar and vector forms,
the system instructions, every refusal of a malformed word) are held to lldb by
`src/arch/aarch64/decode/tests/swept_words.txt`, built in four steps:

    python3 tools/oracles/aarch64-decode/sweep.py flips src/arch/aarch64/decode/lldb_words.txt /tmp/flips.s
    python3 tools/oracles/aarch64-decode/sweep.py system /tmp/system.s
    clang -arch arm64 -o /tmp/flips_host /tmp/flips.s -Wl,-e,_main     # a host binary of `.long` words
    zsh tools/oracles/aarch64-decode/lldb-corpus.sh /tmp/flips.txt /tmp/flips_host flips_host
    cat /tmp/flips.txt /tmp/system.txt > /tmp/all.txt
    LONG_SHIFT=21 LONG_MASK=63 python3 tools/oracles/aarch64-decode/pick.py /tmp/all.txt /tmp/cand.txt 1 \
        src/arch/aarch64/decode/lldb_words.txt

`pick.py` keeps one row per mnemonic and operand shape the sample lacks. Then
`DECODE_CORPUS=/tmp/cand.txt cargo test -p mfb --bin mfb lists_a_sweeps_disagreements_with_lldb
-- --ignored --nocapture` prints every row the decoder spells differently (its
refusals of words lldb knows, and its acceptance of words lldb refuses, included);
drop those rows and what is left is a `swept_words.txt` that
`decodes_swept_words_as_lldb_does` holds the decoder to. The rows dropped are the
extensions the decoder does not decode, system registers lldb names that the
decoder prints as `S<op0>_<op1>_C<n>_C<m>_<op2>`, and encodings with a
should-be-zero field set, which the decoder accepts and LLVM refuses.
