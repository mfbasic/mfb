# plan-174-B: riscv64 instruction decoder

Last updated: 2026-10-12
Effort: large (estimate: 3h–1d)
Depends on: plan-174-A

Adds `src/arch/riscv64/decode`, the riscv64 counterpart of
`src/arch/aarch64/decode`: a function from instruction bytes and a `pc` to the
instruction's text, length, and where control goes next. `mfb dbg` needs it on
riscv64 for `dis`, for finding a function's returns (`finish`), for unwinding
(`bt`) and, because the riscv64 kernel cannot single-step, for `step`. The
outcome: every instruction in the compiler's own riscv64 output and in the two
boxes' C libraries decodes to the text GNU `objdump` prints for it.

This sub-plan adds the decoder and its tests only. Nothing calls it yet.

References:

- `src/arch/aarch64/decode/mod.rs:decode`, `Instruction`, `Flow`: the precedent.
- `src/arch/aarch64/decode/tests.rs`, `lldb_words.txt` and
  `tools/oracles/aarch64-decode/`: the precedent for an oracle corpus.
- `src/arch/riscv64/encode/emitter.rs`: every form the compiler emits.
- The RISC-V unprivileged specification, for the encodings.

## Prerequisites

The prerequisites are stated once in plan-174-A. In addition:

| Must be true | Command | Status |
|---|---|---|
| plan-174-A is complete | `ls planning/plan-174-A-*` → no match (archived to `planning/completed/`) | NOT MET (2026-10-12: plan-174-A is not started) |
| Box 2232 has GNU `objdump` | `ssh -p 2232 test@127.0.0.1 'objdump --version \| head -1'` → a version line | MET (2026-10-12) |

If plan-174-A is not complete, this plan cannot start.

> **NOTE: the Status column is a snapshot; the Command column is the truth.**
> Re-run every command before you continue. If you stop, report all of them.

## 1. Goal

- `crate::arch::riscv64::decode::decode(bytes: &[u8], pc: u64) ->
  Option<Instruction>` and `length(bytes: &[u8]) -> usize` exist.
- Every instruction of `examples/debug` built for `linux-riscv64`, of
  `/lib/riscv64-linux-gnu/libc.so.6` on box 2232 and of
  `/lib/ld-musl-riscv64.so.1` on box 2229 decodes, and its text equals
  `objdump`'s.
- Every form `src/arch/riscv64/encode/emitter.rs` emits decodes.

### Non-goals (explicit constraints)

- No change to the encoder, the selector or any compiler output.
- No use from `src/debugger` (plan-174-C and D).
- Extensions outside the measured population (§2) are not decoded: `decode`
  answers `None` for them and `length` still answers their size. Vector (`V`)
  forms beyond the 47 the encoder emits are in that group.
- The decoder is not a second source of encodings for the compiler. It is read
  only by the debugger.

## 2. Current State

`src/arch/riscv64/` holds `backend.rs`, `mod.rs`, `regmodel.rs`, `reloc.rs`,
`select.rs`, `v128.rs` and `encode/` (`emitter.rs`, `relax.rs`, `sizing.rs`,
`operand.rs`, `mod.rs`, `tests.rs`). It has no decoder
(`find src -iname '*decode*'` → only `src/arch/aarch64/decode`).

The AArch64 decoder is 3,314 lines of code and 792 of tests
(`wc -l src/arch/aarch64/decode/*.rs`), split by instruction class, with
`lldb_words.txt` (930 rows) checked on every test run and an ignored
whole-corpus test.

The compiler emits fixed 4-byte riscv64 instructions: base integer, `M`, `F`/`D`
and a vector subset. It emits no compressed instruction and no `Zb*`
(`emitter.rs` comments at the rotate and bit-reverse arms: "Base-ISA … (no
Zbb)"). The C libraries are RV64GC, so most of their instructions are 2 bytes.

### Measured populations

| What | Count | Command |
|---|---|---|
| Distinct mnemonics in `examples/debug` for riscv64, aliases off | 47 | on 2232: `objdump -d --no-show-raw-insn -M no-aliases debug-glibc.out \| awk '{print $2}' \| grep -E '^[a-z]' \| sort -u \| wc -l` → 47 |
| Words `objdump` could not decode in that file | 0 | `objdump -d debug-glibc.out \| grep -c '\.insn\|unknown\|\.word'` → 0 |
| Distinct mnemonics in glibc 2.41 `libc.so.6`, aliases off | 166 | same pipeline on `/lib/riscv64-linux-gnu/libc.so.6` → 166 |
| Vector mnemonics the encoder emits | 47 | `grep -oE '"v[a-z0-9.]+"' src/arch/riscv64/encode/emitter.rs \| sort -u \| wc -l` → 47 |
| Mnemonics the encoder accepts in all | 131 | `grep -oE '^\s*"[a-z][a-z0-9_.]*" *(\\\|\|=>)' src/arch/riscv64/encode/emitter.rs \| grep -oE '"[^"]+"' \| sort -u \| wc -l` → 131 |

The 166 are RV64I, `M`, `A` (`lr`/`sc`/`amo*`), `F`, `D`, `C`, `Zicsr`
(`csrrs`, `csrrw`, `csrrsi`) and `Zifencei` (`fence`). The most frequent are
compressed: `c.mv` 27,407, `c.ldsp` 25,237, `c.li` 21,648, `c.sdsp` 21,400.

### Verified properties

- **Instruction length is in the first two bits.** A 16-bit parcel whose low two
  bits are not `11` is a 2-byte instruction; otherwise the instruction is 4
  bytes. Both riscv64 boxes implement `C` (`grep isa /proc/cpuinfo` shows `zca`
  and `zcd` on each).
- **`objdump` is available as the oracle on box 2232** and reads the compiler's
  executables (the disassembly of `_mfb_fn_isPrime` in plan-174-C §2 is its
  output). Xcode's `llvm-objdump` has no riscv64 target (`can't find target:
  unable to get target for 'riscv64--'`), so the oracle runs on the box.
- **A compiled function's shape**, from `objdump` of `_mfb_fn_isPrime`: prologue
  `addi sp,sp,-208` then `sd ra,0(sp)`; calls as `auipc ra,0xf` then
  `jalr -260(ra)`; epilogue `ld ra,0(sp)`, `addi sp,sp,208`, `ret`.

## 3. Design Overview

The module mirrors the AArch64 one in shape so `mfb dbg` treats both alike:

```rust
pub(crate) struct Instruction {
    pub(crate) length: usize,            // 2 or 4
    pub(crate) text: String,             // as objdump prints it, comment dropped
    pub(crate) flow: Flow,
    pub(crate) target: Option<u64>,      // a direct branch or jump's destination
    pub(crate) value: Option<u64>,       // what `li` sets
    pub(crate) page: Option<(u8, u64)>,  // auipc: register and the address it forms
    pub(crate) page_offset: Option<(u8, i64)>, // addi/ld/jalr through that register
    pub(crate) effect: StackEffect,      // for the unwinder, see plan-174-C
}

pub(crate) enum Flow {
    Next,                                 // falls through
    Branch,                               // conditional: `target` or the next
    Jump,                                 // unconditional direct: `target`
    Indirect { register: u8, offset: i64 }, // jalr / c.jr / c.jalr: (reg + offset) & !1
    Call,                                 // jal/jalr that links; `Indirect` data in `indirect`
    Return,                               // jalr x0, 0(ra) and c.jr ra
    Trap,                                 // ecall, ebreak, c.ebreak
    AtomicStart,                          // lr.w / lr.d
}
```

`Flow` is richer than AArch64's because plan-174-D computes the next `pc` from
it: the kernel has no single-step on riscv64.

Text follows GNU `objdump` with its default aliases (`li`, `mv`, `ret`, `j`,
`jr`, `beqz`, `bnez`, `nop`, `sext.w`, `neg`, `not`, and the `c.`-less spelling
`objdump` gives compressed forms by default), ABI register names, and its
operand spelling (`sd ra,0(sp)`, `jalr -260(ra)`). The decoder prints the
mnemonic, one tab, then the operands, exactly as `objdump --no-show-raw-insn`
does; the debugger pads the mnemonic itself (plan-174-C).

**Correctness risk** is alias selection and compressed-form expansion: a wrong
immediate decode in a `c.` form gives a wrong `sp` adjustment to the unwinder
and a wrong next `pc` to the stepper. The oracle corpus bounds it: the same
bytes go through `objdump` and through `decode`, and the test compares text for
every row.

**Byte-identity is not this sub-plan's gate.** It adds new code with no caller;
no compiler output changes. The gate is the oracle comparison.

Probed before writing: the mnemonic populations of §2, the absence of undecoded
words in the compiler's output, and the function shape above.

Rejected alternatives:

- **Decode only what the compiler emits.** `step`, `bt` and `finish` run through
  C library code (a breakpoint on `exit`, a backtrace from a system call), which
  is compressed. A decoder without `C` cannot find instruction boundaries there.
- **Print canonical (alias-free) mnemonics.** Developers read `objdump` output
  and the alias forms (`ret`, `li`, `mv`); the AArch64 decoder likewise follows
  its platform tool, lldb.
- **Reuse the encoder tables in reverse.** The encoder is keyed by the
  compiler's AArch64-flavoured operation names (`add_imm`, `ldr_u64`), not by
  RISC-V mnemonics, and covers a quarter of what the libraries use.

## 4. Detailed Design

Files under `src/arch/riscv64/decode/`:

- `mod.rs`: `Instruction`, `Flow`, `decode`, `length`, register-name tables
  (`x1` → `ra` … `x31` → `t6`; `f0` → `ft0` … as `objdump` names them), field
  extractors for the R/I/S/B/U/J formats.
- `integer.rs`: opcodes `LOAD`, `OP-IMM`, `AUIPC`, `OP-IMM-32`, `STORE`, `OP`,
  `LUI`, `OP-32`, `BRANCH`, `JALR`, `JAL`, `SYSTEM` (`ecall`, `ebreak`, the
  `csr*` forms), `MISC-MEM` (`fence`); `M` lives in `OP`/`OP-32`.
- `atomic.rs`: opcode `AMO`: `lr`, `sc`, `amoswap`, `amoadd`, `amoxor`,
  `amoand`, `amoor`, `amomin(u)`, `amomax(u)`, each `.w`/`.d` with the
  `.aq`/`.rl`/`.aqrl` suffix `objdump` prints.
- `float.rs`: `LOAD-FP`, `STORE-FP`, `OP-FP`, `MADD`/`MSUB`/`NMSUB`/`NMADD`,
  with the rounding-mode operand `objdump` prints when it is not dynamic.
- `compressed.rs`: quadrants 0, 1, 2 of RV64C, each expanded to its base form
  for `flow`, `target`, `value` and `effect`, and printed as `objdump` prints
  it.
- `vector.rs`: the 47 forms `emitter.rs` emits (`vsetvli`, `vsetivli`, the
  `v*.vv`/`.vx`/`.vi` arithmetic, `vle64.v`, `vse64.v`, `vmv.*`, `vmerge.vim`,
  `vslidedown.vi`, the `vf*` and `vm*` forms).
- `tests.rs`, `objdump_words.txt`.

`decode` reads `length(bytes)` first; with fewer bytes than that it answers
`None`. A 4-byte word whose opcode or function fields match nothing answers
`None`; the debugger prints such a word as `.word` (4 bytes) or `.short` (2).

The oracle lives in `tools/oracles/riscv64-decode/`:

- `objdump-corpus.sh <out> <file>...`: run on a riscv64 Linux box; for each ELF
  writes `<pc>\t<bytes>\t<text>` rows from `objdump -d` with the trailing
  `# …` comment and the `<symbol>` annotation dropped.
- `README.md`: how to run it and the measured results table, as
  `tools/oracles/aarch64-decode/README.md` has.

`objdump_words.txt` is a sample of those corpora, up to three operand shapes per
mnemonic, checked on every test run by `decodes_words_as_objdump_does`. An
ignored test, `decodes_a_corpus_as_objdump_does`, takes `DECODE_CORPUS=<file>`
and fails on any row whose text differs, printing undecoded rows by mnemonic.

## Compatibility / Format Impact

None. No emitted byte and no command output changes.

## Phases

> **NOTE: keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work. Fill the phase's `Commit:` line when it lands. An unticked
> box means NOT DONE.

### Phase 1 — the oracle and its corpora

Lands the tool and the reference data before any decoder code, so every later
phase has something to be checked against.

- [ ] Add `tools/oracles/riscv64-decode/objdump-corpus.sh` and `README.md`.
- [ ] On box 2232, write corpora for `/tmp/debug-glibc.out` (built with
      `mfb build --debug -O0 --target linux-riscv64 examples/debug`) and
      `/lib/riscv64-linux-gnu/libc.so.6`; copy `/lib/ld-musl-riscv64.so.1` from
      box 2229 to 2232 and write its corpus too. Keep them in `/tmp/rv-corpus/`.
- [ ] Add `src/arch/riscv64/decode/objdump_words.txt`: up to three operand
      shapes per mnemonic drawn from the three corpora.

Acceptance: the sample covers every mnemonic the corpora contain.
  Check: `cut -f3 /tmp/rv-corpus/*.txt | awk '{print $1}' | sort -u | wc -l`
  equals `cut -f3 src/arch/riscv64/decode/objdump_words.txt | awk '{print $1}'
  | sort -u | wc -l` (est. 5 min, mostly `objdump` of libc on box 2232).
Commit: —

### Phase 2 — base integer, `M`, `A`, system

- [ ] Add `decode/mod.rs`, `integer.rs`, `atomic.rs`; declare `mod decode` in
      `src/arch/riscv64/mod.rs`.
- [ ] Tests: `decodes_words_as_objdump_does` over the rows of
      `objdump_words.txt` whose instruction is 4 bytes and not `F`/`D`/`V`;
      `flow_and_targets` for `beq`, `jal`, `jalr`, `ret`, `ecall`, `lr.w`.

Acceptance: every 4-byte integer, `M`, `A` and system row decodes to `objdump`'s
text.
  Check: `cargo test --bin mfb riscv64::decode` → 0 failed (est. 2 min).
Commit: —

### Phase 3 — compressed

- [ ] Add `compressed.rs`: all RV64C forms in the corpora (`c.addi4spn`, `c.ld`,
      `c.sd`, `c.lw`, `c.sw`, `c.fld`, `c.fsd`, `c.addi`, `c.addiw`, `c.li`,
      `c.addi16sp`, `c.lui`, `c.srli`, `c.srai`, `c.andi`, `c.sub`, `c.xor`,
      `c.or`, `c.and`, `c.subw`, `c.addw`, `c.j`, `c.beqz`, `c.bnez`, `c.slli`,
      `c.ldsp`, `c.lwsp`, `c.fldsp`, `c.jr`, `c.mv`, `c.ebreak`, `c.jalr`,
      `c.add`, `c.sdsp`, `c.swsp`, `c.fsdsp`, `c.nop`, `c.unimp`).
- [ ] Tests: the 2-byte rows of `objdump_words.txt`; `length` for both sizes;
      `flow_and_targets` for `c.j`, `c.beqz`, `c.jr ra` (a return), `c.jalr`.

Acceptance: every 2-byte row decodes to `objdump`'s text with length 2.
  Check: `cargo test --bin mfb riscv64::decode` → 0 failed (est. 2 min).
Commit: —

### Phase 4 — `F`/`D`, the vector subset, and the whole corpora

- [ ] Add `float.rs` and `vector.rs`.
- [ ] Tests: `decodes_every_form_the_encoder_emits`, which encodes each of the
      131 operations through `src/arch/riscv64/encode` (as
      `encode/tests.rs` builds them) and asserts `decode` answers `Some` with
      the same length; the remaining rows of `objdump_words.txt`; the ignored
      `decodes_a_corpus_as_objdump_does`.
- [ ] Record in `tools/oracles/riscv64-decode/README.md` the rows and undecoded
      counts for the three corpora, with the `objdump` version.

Acceptance: no row of the three corpora decodes to different text, and none is
undecoded.
  Check: for each corpus, `DECODE_CORPUS=/tmp/rv-corpus/<name>.txt cargo test
  --bin mfb decodes_a_corpus_as_objdump_does -- --ignored --nocapture` →
  `0 differing, 0 undecoded` (est. 5 min for the three; the corpora are copied
  to the Mac, where the test runs).
Commit: —

## Validation Plan

- Tests: `src/arch/riscv64/decode/tests.rs` — the sampled rows on every run, the
  encoder round trip, flow and target cases, and the ignored corpus test.
- Coverage check: `grep -c . src/arch/riscv64/decode/objdump_words.txt` is at
  least the mnemonic count of Phase 1, so every mnemonic is in the always-run
  test, not only in the ignored one.
- Runtime proof: none here. The decoder has no caller until plan-174-C; its
  proof is the corpus comparison against `objdump`.
- Doc sync: `tools/oracles/riscv64-decode/README.md` (new). No spec topic
  changes: the decoder is internal to the debugger and reaches no observable
  contract until plan-174-D, which updates `mfb spec tooling debugger`.
- Final gate: plan-174-E, once for all of plan-174.

## Decisions

- Registers are shown by ABI name only — decided by the owner, 2026-10-12.
  The decoder prints `ra`, `sp`, `a0`, never `x1`, `x2`, `x10`, which is also
  `objdump`'s default. (§3)

## Corrections

## Summary

The risk is in the compressed forms and `objdump`'s alias rules, and the corpus
comparison measures both directly. The compiler, its encoder and its output are
untouched.
