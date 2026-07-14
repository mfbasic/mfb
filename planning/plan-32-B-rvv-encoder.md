# plan-32-B: RVV instruction encoder (vsetvli + vector ops → bytes)

Last updated: 2026-07-08
Effort: medium (1h–2h)
Depends on: nothing (independent of plan-32-A; both are consumed by C)

Add the RISC-V Vector encoding layer to the riscv64 emitter: the `OP-V`
(opcode `0x57`) instruction formats, `vsetvli`/`vsetivli` configuration, and the
concrete vector mnemonics sub-plan C will select (arithmetic, FMA, min/max,
sqrt/abs/neg, float↔int conversion, integer add/sub/shift/bitwise, mask
compares, mask→lane-vector materialization, splat, element extract, and 128-bit
vector load/store). This sub-plan adds **encoding + unit tests only** — no
selection change — so it is safe to land alone and verifiable against a
reference assembler.

The single behavioral outcome: each new vector `CodeInstruction` mnemonic
encodes to the exact 32-bit word a reference RISC-V assembler (`clang
-march=rv64gcv` / `llvm-mc`) produces for the same operands.

References:

- `src/arch/riscv64/encode/emitter.rs` — `emit_instruction` mnemonic match
  (`:119`), the `r_type`/`i_type`/`s_type`/`u_type` field packers (`:57`–`:85`),
  `emit_fp_r` (`:401`), `emit_load_fp`/`emit_store_fp` (`:502`,`:511`).
- `src/arch/riscv64/encode/sizing.rs` — per-mnemonic byte-size table (every new
  op is a single 4-byte word, so sizing is uniform).
- `src/arch/riscv64/encode/operand.rs` — register-name → number decoding (needs
  `v0`–`v31`).
- `src/arch/riscv64/encode/tests.rs` — the encoding-test pattern to mirror.
- The RISC-V "V" Vector Extension spec v1.0 (instruction formats §5, opcode
  `OP-V`=`1010111`; `vsetvli` §6). Cross-check every word with `llvm-mc
  -triple=riscv64 -mattr=+v --show-encoding`.

## 1. Goal

- A `v0`–`v31` vector register class decodable by
  `encode/operand.rs`.
- Encoders for the vector formats used by C:
  - **Config:** `vsetvli rd, rs1, vtypei` and `vsetivli rd, uimm, vtypei`
    (SEW=64, LMUL=1, ta/ma) — the pass configures `SEW=64, vl=2` once per kernel
    region.
  - **OPFVV** (float vector-vector): `vfadd/vfsub/vfmul/vfdiv/vfmin/vfmax`,
    `vfmacc/vfnmsac` (FMA), `vfsgnjn/vfsgnjx` (neg/abs), `vfsqrt.v`,
    `vmflt/vmfle/vmfeq` (mask compares), `vfcvt.*.x.f`/`vfcvt.f.x` conversions.
  - **OPIVV / OPIVX / OPIVI** (integer): `vadd/vsub/vand/vor/vxor` (vv),
    `vrsub.vx` (neg), `vsll/vsra/vsrl` (`.vi` immediate + `.vx`),
    `vmslt/vmsle/vmseq` (mask compares), `vmerge.vim`/`vmv.v.i`/`vmv.v.x`
    (lane-mask materialization + splat).
  - **OPMVV / OPMVX:** `vmv.x.s` (extract element 0 → GPR), `vmv.s.x`,
    `vslidedown.vi` (reach element 1 for `UmovXFromV` index 1).
  - **Vector load/store:** `vle64.v`/`vse64.v` (unit-stride, for `LdrQ`/`StrQ`
    16-byte moves and vector spill/reload).
- Every new mnemonic sized as one 4-byte word in `encode/sizing.rs`.
- Encoding unit tests in `encode/tests.rs` asserting exact words vs. `llvm-mc`.

### Non-goals (explicit constraints)

- **No selection wiring.** `select_riscv64` and the dual-path lowering are
  untouched here; nothing emits these mnemonics yet, so all real output is
  byte-identical.
- No compressed vector encodings, no LMUL≠1, no segment/indexed/strided
  loads — only what C needs (SEW=64, LMUL=1, unit-stride).
- No change to existing GPR/FP encoders.

## 2. Current State

- The emitter is a flat mnemonic `match` in `emit_instruction`
  (`src/arch/riscv64/encode/emitter.rs:119`) that dispatches to typed field
  packers (`r_type` `:57`, `i_type` `:61`, `s_type` `:65`, `b_type` `:72`,
  `u_type` `:81`, `j_type` `:85`) and helpers like `emit_fp_r` (`:401`). RISC-V
  words are fixed 32-bit; `sizing.rs` maps each mnemonic to its byte length
  (mostly 4, with multi-word base-ISA expansions).
- Register names decode via `encode/operand.rs`; today only `x*`/ABI-int names
  and `f*` names are recognized — **no `v*` vector registers**.
- Encoding tests (`encode/tests.rs`) assert exact 32-bit words, the pattern this
  sub-plan extends.
- There is no `OP-V` support anywhere; this is all new but isolated to the
  encoder.

## 3. Design Overview

Add one new instruction family to the existing flat emitter, isolated to new
mnemonics so nothing existing shifts:

1. **Vector register decode** — extend `operand.rs` to map `v0`–`v31` → 0–31
   (a distinct namespace; a `vector_reg(name)` helper).
2. **Format packers** — add `vsetvli`/`vsetivli` packers and a generic
   `op_v(funct6, vm, vs2, vs1_or_rs1_or_imm, funct3, vd)` packer for the
   `OP-V` (`0x57`) major opcode. All vector arithmetic is this one 32-bit shape
   with varying `funct6`/`funct3`/`vm`; encode it once and table-drive the
   mnemonics.
3. **`vtype` immediate** — a helper computing the `vtypei` field from
   `(SEW, LMUL, ta, ma)`; C only needs `SEW=64, LMUL=1, ta, ma`.
4. **Mnemonic table** — each new mnemonic maps to `(funct6, funct3, vm)` and its
   operand roles (`vd`/`vs2`/`vs1`/`rs1`/`imm`/`mask`). `vm` (mask-enable bit)
   distinguishes masked (`vmerge.vim`, masked splat) from unmasked forms.
5. **Sizing** — every new mnemonic is 4 bytes; add them to the `sizing.rs`
   table so the code-plan byte layout is correct.

**Risk concentrates on encoding correctness** — the `funct6`/`funct3`/`vm`/
`vtype` bit fields are easy to get subtly wrong. Mitigation: every mnemonic gets
a unit test asserting the exact word from `llvm-mc -mattr=+v --show-encoding`,
and the test module lists the reference command so the golden words are
reproducible. This is the whole point of landing the encoder before selection —
it is verifiable in isolation, byte-exactly, without a running program.

**Rejected alternative:** emitting textual `.insn`/assembly and shelling out to
an assembler at build time — the backend is a self-contained encoder (no
external assembler dependency, per plan-99); vector ops follow suit.

## Compatibility / Format Impact

- **Changed:** new vector mnemonics recognized by the emitter and sizer; `v*`
  register names decodable. All additive.
- **Unchanged:** every existing mnemonic's encoding and size; no other backend.

## Phases

### Phase 1 — vector register decode + format packers + vtype

The primitives, unit-tested, with no mnemonics wired yet.

- [ ] `encode/operand.rs`: add `vector_reg(name) -> Option<u8>` for `v0`–`v31`.
- [ ] `encode/emitter.rs`: add `op_v(funct6, vm, vs2, vs1, funct3, vd)` packer
      for opcode `0x57`, plus `vsetvli`/`vsetivli` packers and a `vtype_i64_m1`
      helper (SEW=64, LMUL=1, ta, ma).
- [ ] Tests: `encode/tests.rs` — `vsetvli`/`vsetivli` with SEW=64,LMUL=1 encode
      to the `llvm-mc` reference word; `vector_reg` decodes `v0`/`v31`, rejects
      `v32`.

Acceptance: config-instruction words match `llvm-mc`; register decode unit tests
pass. No existing output changes.
Commit: —

### Phase 2 — float vector ops (OPFVV) + conversions + FMA

- [ ] Emit `vfadd/vfsub/vfmul/vfdiv/vfmin/vfmax.vv`, `vfmacc/vfnmsac.vv`,
      `vfsgnjn/vfsgnjx.vv`, `vfsqrt.v`, `vfcvt.rtz.x.f.v`/`vfcvt.x.f.v` (frm)/
      `vfcvt.f.x.v`, and `vmflt/vmfle/vmfeq.vv` via the `op_v` packer + table.
- [ ] Tests: exact-word tests for each float mnemonic (SEW=64) vs. `llvm-mc`.

Acceptance: every float vector mnemonic encodes to its reference word.
Commit: —

### Phase 3 — integer vector ops, mask materialization, splat/extract, load/store

- [ ] Emit `vadd/vsub/vand/vor/vxor.vv`, `vrsub.vx`, `vsll/vsra/vsrl.vi` and
      `.vx`, `vmslt/vmsle/vmseq.vv`, `vmerge.vim`, `vmv.v.i`, `vmv.v.x`,
      `vmv.x.s`, `vslidedown.vi`, `vle64.v`, `vse64.v`.
- [ ] `encode/sizing.rs`: register all new mnemonics as 4-byte words.
- [ ] Tests: exact-word tests for each; a sizing test asserting 4 bytes each.

Acceptance: every integer/mask/mem vector mnemonic encodes to its reference
word and sizes to 4 bytes; `encode/tests.rs` green.
Commit: —

## Validation Plan

- Tests: per-mnemonic exact-word encoding tests in `encode/tests.rs`, each
  annotated with the `llvm-mc -triple=riscv64 -mattr=+v --show-encoding`
  command that produced its golden word (so they are reproducible, not
  hand-guessed). Negative: `vector_reg("v32")`→None.
- Runtime proof: N/A this sub-plan (nothing selects these yet) — the proof is
  the reference-assembler match. A one-off sanity check: assemble a handful of
  the emitted words with `llvm-mc` on the host and diff.
- Doc sync: none yet (encoder internals).
- Acceptance: `cargo test` green; `scripts/artifact-gate.sh` byte-identical for
  all targets (no selection emits these mnemonics yet).

## Open Decisions

- **`vfcvt` for ties-away (`FCvtasV`)** — RVV float→int uses the dynamic `frm`;
  ties-to-max-magnitude is `frm=RMM`. Recommend encoding the conversion with an
  explicit `frm` set/restore around the op (or a static-rounding vcvt variant if
  available) so the result matches AArch64 `fcvtas` bit-for-bit. Verify against
  the ULP harness in D. (§1)
- **`vmv.x.s` for lane index 1 (`UmovXFromV`)** — recommend `vslidedown.vi vt,
  vs, 1; vmv.x.s rd, vt` (extract element 1). Confirm no cheaper path. (§1)

## Summary

A self-contained, byte-exactly-verifiable encoder for the RVV subset C needs.
Risk is confined to bit-field correctness and neutralized by per-mnemonic
reference-assembler tests. Nothing in the running compiler emits these words
until sub-plan C, so this lands with zero behavioral change.
