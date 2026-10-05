# plan-171-C: Logical immediates and shorter AArch64 constant materialization

Last updated: 2026-10-04
Effort: large (3h–1d)
Depends on: plan-171-B

Every backend applies a constant mask the same way: `mov_imm m, K` then a
register `and`/`orr`/`eor`. The source carries 274 such adjacent pairs
(`rg -U -P --count-matches 'abi::move_immediate\((?:[^()]|\([^()]*\))*?\)\)?[,;]?\s*\n\s*[^\n]*abi::(and|or|exclusive_or)_registers\(' src`,
summed → 274: 84 in `src/codegen/builtins`, 10 in `src/codegen/runtime`, 58 in
`src/target`), and the committed goldens show the shape in every program
(8–168 adjacent `mov_imm` → logic pairs per golden, plan-171-A §2.2's census
script). On AArch64, `mov_imm` of a 64-bit mask is up to four instructions
(`movz` + three `movk`; there is no `movn` and no `orr`-immediate form), so
`x AND 0x5555555555555555` is five instructions where one `and x, x, #imm`
suffices. x86-64 materializes every constant with a 10-byte `movabs`; riscv64
with `li` (1–8 instructions).

This sub-plan adds the immediate logical forms on all three targets — AArch64
`and`/`orr`/`eor #bitmask`, x86-64 `and`/`or`/`xor r/m64, imm32` (sign-extended),
riscv64 `andi`/`ori`/`xori` (12-bit signed) — through ops, MIR, encoders,
decoder, the optimizer's pure-op tables, a MIR combining row, and the builders;
and gives AArch64 `mov_imm` the `movn` and `orr xd, xzr, #bitmask` encodings.

Outcome: a logical operation with an encodable constant operand is one
instruction on every target; a non-encodable constant keeps today's
`mov_imm` + register sequence; all program outputs are unchanged.

References: plan-171-A's list, plus
- `src/arch/aarch64/encode/emitter.rs` (`emit_mov_imm`, the `and`/`orr`/`eor` arms), `sizing.rs:checked_imm12`, `encode_add_sub_imm`.
- `src/arch/x86_64/encode/emitter.rs:alu3`, `enc_alu_imm32`, `enc_mov_imm64`.
- `src/arch/riscv64/encode/emitter.rs:emit_r`, `emit_li`; `sizing.rs:li_steps`.
- `src/optimizer/opt2/constant_folding.rs`, `knownbits.rs`, `lvn.rs:numberable_op`, `plans/mark.rs` (`removable_op`, `flag_preserving`).
- `src/codegen/builtins/bits/func_pop_count.rs` (the `POPCOUNT_MASK_5555` site), `func_band.rs`.

## Prerequisites

See plan-171-A §Prerequisites. In addition: plan-171-B complete
(`ls planning/plan-171-B-* 2>/dev/null` → no match).

## 1. Goal

- New neutral ops `and_imm`, `orr_imm`, `eor_imm` (`dst`, `src`, `imm`), encoded
  natively by all three backends for encodable immediates.
- `Backend::logical_immediate_encodable(op, value) -> bool`: AArch64 =
  bitmask-encodable (64-bit `N:immr:imms`; never 0 or all-ones); x86-64 = fits
  `i32` sign-extended; riscv64 = fits 12-bit signed.
- Every producer asks that question first; an encoder receiving a non-encodable
  immediate returns an encode error (a bug detector, never a materialization
  path — no encoder-private scratch is introduced, which keeps x86 free of new
  `push`/`pop` scratch that plan-169-C is removing).
- AArch64 `mov_imm` picks the shortest of: `movz`+`movk` chain, `movn`+`movk`
  chain, single `orr xd, xzr, #bitmask`.

### Non-goals

- No change to any value or trap; logical ops never trap.
- No `tst`/`ands` form (Open Decision 1).
- x86 `mov_imm` shortening (`mov r32, imm32`, `mov r/m64, simm32`) is sub-plan G
  (it is an x86 size-encoding change of the same class as rel8).

## 2. Current State

- No immediate logical op: `rg -cE 'AndImm|OrrImm|EorImm' src/arch/ops.rs` → 0;
  the only immediate ALU ops are `add_imm`, `sub_imm` and the shift immediates.
- AArch64 `and`/`orr`/`eor`/`mvn` are shifted-register with shift 0. `mov_imm`
  always starts `movz` and adds non-zero `movk`s (up to 4 words): `-1` is four
  instructions. No bitmask helper exists anywhere
  in the encoder; the decoder already decodes the logical-immediate class and the
  `mov` bitmask alias (`src/arch/aarch64/decode/integer.rs:bitmask`,
  `logical_immediate`, `f62f27418`).
- x86: `alu3` emits `mov dst, lhs; op dst, rhs` (register only); `mov_imm` is
  always `enc_mov_imm64` (10 bytes).
- riscv64: the only I-type logical is `xori rd, rs, -1` for `mvn`; `and`/`orr`/`eor`
  are `emit_r`.
- The MIR optimizer's pure-ALU knowledge is whitelists keyed on op
  (`removable_op` in `opt2/plans/mark.rs`, `lvn::numberable_op`, the constant
  folder's `fold_one`, `knownbits`). A new op absent from them is treated as an
  unknown barrier — correct but it blocks folding; Phase 2 adds it.
- Sub-plan B's bit-test matcher sees `mov_imm 2^k; and t, r, m`; once this plan
  rewrites that to `and_imm t, r, #2^k`, the matcher must accept it (Phase 4).

### Verified properties

- *AArch64 bitmask immediates cannot encode 0 or all-ones* — architectural
  (DecodeBitMasks); the property test in Phase 2 proves the encoder agrees.
- *Every x86 logical with an `imm32` sign-extends to 64 bits* — architectural;
  Phase 2's encoder test covers `0x7fffffff`, `-1`, `0x80000000` (rejected).
- *How many emitted `mov_imm`+logic pairs have an encodable constant, per target*
  — UNMEASURED; Phase 1.

## 3. Design Overview

Layers: (1) ops + encodability query + encoders + decoder (inert);
(2) optimizer tables learn the ops (inert — nothing produces them);
(3) producers: a Level-1 MIR row "Logical-immediate combining" (catches user
code like `bits::band(x, 255)` and anything the builders still emit), and the
builder sweep (the 274 source sites); (4) AArch64 `mov_imm` encodings (Level 0,
encoder).

**Correctness risk:** the bitmask encoder (a wrong `N:immr:imms` silently
produces a different mask). It is fully testable: there are exactly 5334 valid
64-bit bitmask immediates, and Phase 2 round-trips all of them through
encode → decode → DecodeBitMasks. **Design uncertainty:** none material.

**Gate class:** behavior-changing for Phases 3–5. Expected diffs: every target's
goldens wherever an encodable mask exists (essentially all — the census shows
mask pairs in every golden), plus every aarch64 golden for `mov_imm` (Phase 5).

**Rejected alternatives.**
- *Encoder-side materialization of non-encodable immediates into a scratch
  register* (the `cmp_imm` precedent on AArch64) — x86 has no free scratch (four
  allocatable GPRs, `INT_ALLOCATABLE = ["r10","r11","r12","r14"]`; existing
  expansions push/pop), and riscv64's encoder scratch `t0` is overwritten by
  spill code between instructions (`.ai/arch-abi.md`, bug-749). Producers decide
  instead.
- *Selection-time rewrite in `select_*`* — selection is 1:1 per instruction and
  cannot see that `m` has a single use; the MIR row has the SSA overlay for that.

## 4. Detailed Design

### 4.1 AArch64 bitmask immediate

`src/arch/aarch64/encode/bitmask.rs`: `encode_logical_immediate(value: u64) ->
Option<(n, immr, imms)>` — find the smallest element size e ∈ {2,4,…,64} whose
repetition yields `value`, require the element to be a rotated run of ones,
compute rotation and run length (standard LLVM `processLogicalImmediate`
algorithm). The decoder side exists: `decode/integer.rs:bitmask` (the ARM
`DecodeBitMasks`) and `logical_immediate`, which prints the class and the `mov`
(bitmask) alias as lldb does.

### 4.2 Producers

- **MIR row** (`src/optimizer/opt2/logimm.rs`, level 1, stage MIR): for
  `and|orr|eor d, a, b` where one operand's only reaching definition is
  `mov_imm m, K` (SSA overlay) and `m` has no other use, and
  `logical_immediate_encodable` → `and_imm d, other, #K`; the `mov_imm` is left
  for DCE. Placed after `knownbits` and the value-numbering rows (so they still
  see the register form) and before `dce`.
- **Builders:** `CodeBuilder::emit_logical_immediate(op, dst, src, value)` and
  the numeric-`Vregs` twin for hand-written helpers: emit `*_imm` when
  encodable, else today's `move_immediate` + `*_registers`.

### 4.3 AArch64 `mov_imm`

In `emit_mov_imm`/`sizing`: candidates `movz` chain (count non-zero halfwords),
`movn` chain (count non-0xffff halfwords of `!value`), `orr xd, xzr, #K` when
bitmask-encodable; emit the shortest, ties to `movz` (fewest golden diffs).

## Phases

> **NOTE — keep the checkboxes current** (same rules as plan-171-A).

### Phase 1 — measure

- [ ] Extend `census.py` with: `mov_imm`+logic pairs split by encodable/not per
      target (it knows the three rules); `mov_imm` instruction-count histogram on
      aarch64 (how many need 3–4 words; how many would be 1 with `movn`/`orr`).
- [ ] Record both into §2; count multi-bit `and t, r, m; cmp t, 0; b.eq/ne` tests
      for Open Decision 1.

Acceptance: §2 has no UNMEASURED.
  Check: `grep -c UNMEASURED planning/plan-171-C-*.md` → 0 (est. 15 min).
Commit: —

### Phase 2 — ops, encoders, decoder, optimizer tables (byte-identical)

- [ ] `src/arch/ops.rs` (`AndImm`, `OrrImm`, `EorImm`), `mir.rs` mirror group,
      `mir.rs:Backend::logical_immediate_encodable` + the three impls
      (`src/arch/*/backend.rs`).
- [ ] AArch64: `bitmask.rs` (§4.1); encoder arms; sizing.
- [ ] Decoder round-trip table rows for each new encoder mnemonic (the class and
      the `mov` bitmask alias already decode, `f62f27418`).
- [ ] x86: `mov dst, src; and|or|xor dst, imm32` (`81 /4|/1|/6`, imm8 `83` form
      deferred to G); encode error on non-`i32`.
- [ ] riscv64: `andi`/`ori`/`xori` (I-type funct3 7/6/4); encode error outside 12-bit.
- [ ] `opt2/plans/mark.rs` (`removable_op`, `flag_preserving`: AArch64 preserves,
      x86 does not), `lvn::numberable_op`, `constant_folding::fold_one`
      (`and_imm` of a known constant folds), `knownbits` (an `and_imm` mask is a
      known-zero fact, exactly like `and` with a `mov_imm`-fed mask).
- [ ] Tests: the 5334-immediate property test (encode∘decode identity and
      `decode::integer::bitmask(encode(v)) == v`, plus 0/all-ones/`0x1234…` rejected);
      x86/rv64 boundary tests; folder/knownbits unit tests for the new ops.

Acceptance: all round trips pass; no emitted byte changes.
  Check: `cargo test --bin mfb bitmask aarch64::decode x86_64::encode riscv64::encode opt2::` → pass (est. 5 min);
  `bash scripts/artifact-gate.sh target/release/mfb bits` → `0 diff(s)` (est. 1 min).
Commit: —

### Phase 3 — MIR combining row

- [ ] `src/optimizer/opt2/logimm.rs` (§4.2) + catalog row "Logical-immediate
      combining" (level 1, stage MIR) + stats counter; wire into `optimize_mir`.
- [ ] Extend sub-plan B's bit-test matcher (`opt2/cbranch.rs`) to accept
      `and_imm t, r, #2^k` (all three backends), with unit tests.
- [ ] Fixtures: `tests/rt-behavior/bits/logical-immediates` (valid) — `band`/
      `bor`/`bxor` with encodable masks on each target's boundaries (0xff,
      0x5555555555555555, 0x7fffffff, 2047, -2048, 0xf0f0…), with non-encodable
      ones (0x123456789, 0), and with a variable operand; prints results.
      `tests/syntax/bits/logical-immediates-invalid` is not applicable — no new
      user surface; recorded here as the reason.
- [ ] `rt_codegen_forms`: `and_imm`/`orr_imm`/`eor_imm` in all three targets' dumps
      for the encodable cases, `mov_imm`+`and` for the rest.
- [ ] `rt_regalloc_levels_agree`: add the fixture.
- [ ] Golden re-baseline (all targets; procedure as plan-171-A Phase 3).
- [ ] Runtime: macOS acceptance; `linux-runtime-proof.sh` on 2223, 2227, 2229;
      fixture on Windows box 2230.

Acceptance: encodable masks are one instruction on each target; outputs unchanged.
  Check: `cargo test --release --test rt_codegen_forms logical` → pass (est. 3 min);
  the three `linux-runtime-proof.sh` runs → 0 mismatches (est. 25–60 min each).
Commit: —

### Phase 4 — code-review sweep (this form)

- [ ] Convert the 274 `move_immediate` → `*_registers` source sites to
      `emit_logical_immediate` (grep above; re-run it after → 0, or each survivor
      listed with its reason, e.g. the mask register is reused afterwards).
- [ ] Census diff per target; read the 20 functions with the most remaining
      non-encodable masks and decide per site whether a different mask spelling
      is encodable (e.g. `and x, #0xffffffff` vs. a 32-bit `uxtw` — note, do not
      add forms here). "Makes sense" = fewer instructions on that path with the
      same value for every input.
- [ ] Golden re-baseline for the sweep's diffs (same procedure).

Acceptance: the grep returns 0 or only justified survivors; census shows the
pair count fell to the non-encodable remainder.
  Check: the `rg -U -P` command from the header → the justified count (est. 1 min).
Commit: —

### Phase 5 — AArch64 `mov_imm` (`movn`, `orr`)

- [ ] `emit_mov_imm` + sizing (§4.3); decoder already prints `mov` aliases for
      `movn`/`orr` (verify rows exist; add if not).
- [ ] Tests: `0`, `-1`, `0xffff_ffff_0000_ffff`, `0x5555…`, `0x00ff_00ff_…`,
      `i64::MIN`, a value where `movz` and `movn` tie.
- [ ] Golden re-baseline (aarch64 only); runtime: acceptance + 2223.
- [ ] Benchmark on macOS host (`run.sh 10`, `rank.py`; `bits`, `crypto`,
      `compress` rows).

Acceptance: every aarch64 `mov_imm` is ≤ the old instruction count and `-1` is one instruction.
  Check: `cargo test --bin mfb aarch64::encode::tests::mov_imm` → pass (est. 2 min); census `mov_imm` histogram has no 4-word `-1`.
Commit: —

### Phase 6 — spec and docs

- [ ] `aarch64-instruction-set` (logical immediates, worked bitmask encoding,
      `mov_imm` strategy), `x86_64-instruction-set`, `riscv64-instruction-set`,
      `mir-instruction-set` (three mirror ops; the encodability query).
- [ ] `.ai/codegen-invariants.md`: "a non-encodable immediate reaching an encoder
      is a producer bug; ask `logical_immediate_encodable` first".

Acceptance: specs render cleanly.
  Check: `cargo test --bin mfb spec` → pass (est. 2 min).
Commit: —

## Validation Plan

- Tests: bitmask property test, encoder/decoder units, folder/knownbits units,
  `logimm` row units, `rt_codegen_forms`, the bits fixture, levels-agree.
- Coverage: `bits` byte-identity fixture calls `band`/`bor`/`bxor`
  (`grep -n 'bits::b' tests/byte-identity/bits/src/main.mfb` before trusting a green gate).
- Runtime proof: Phase 3's five targets; Phase 5's aarch64 runs.
- Doc sync: Phase 6; `mfb man optimizations` lists the new row.
- Final gate (once): `cargo test --no-fail-fast` to a log, `EXIT=0`, 0 `failures:` (est. 60–90 min).

## Open Decisions

1. `tst`/`ands` with immediate for multi-bit mask tests — **recommend deferring**
   unless Phase 1 finds such tests among the 20 hottest functions; then add a
   `tst_imm` neutral op in a follow-up under the same flags-liveness proof as B.
2. MIR-row placement — **recommend after value numbering, before DCE** (§4.2) vs.
   at selection; the SSA single-use proof exists only in the MIR seam.

## Corrections

<Filled in during execution.>

## Summary

The bitmask encoder is the one place a bug silently changes a value; it is
covered exhaustively. Producers, not encoders, decide encodability, so no
scratch register is introduced on any target.
