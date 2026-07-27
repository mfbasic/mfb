# plan-32-B: RVV instruction encoder (vsetvli + vector ops → bytes)

Last updated: 2026-07-27
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
  (`:124`), the field packers `r_type` (`:59`), `i_type` (`:63`), `s_type`
  (`:67`), `b_type` (`:74`), `u_type` (`:90`), `j_type` (`:94`), `emit_fp_r`
  (`:436`), `emit_load_fp`/`emit_store_fp` (`:555`,`:570`).
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
  (`src/arch/riscv64/encode/emitter.rs:124`) that dispatches to typed field
  packers (`r_type` `:59`, `i_type` `:63`, `s_type` `:67`, `b_type` `:74`,
  `u_type` `:90`, `j_type` `:94`) and helpers like `emit_fp_r` (`:436`). RISC-V
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

- [x] `encode/operand.rs`: add `vreg(name) -> Result<u8, String>` for `v0`–`v31`
      (named/typed to match the existing `reg`/`freg` decoders, not `Option` — see
      Corrections C2).
- [x] `encode/emitter.rs`: add the `v_type(funct6, vm, vs2, vs1, funct3, vd,
      opcode)` packer (one shape covers `OP-V` **and** unit-stride load/store),
      the `vsetvli`/`vsetivli` words, and the `VTYPE_E64_M1_TA_MA` const (SEW=64,
      LMUL=1, ta, ma).
- [x] Tests: `encode/tests.rs` — `vsetvli`/`vsetivli` encode to the reference
      word (`rvv_ops_encode_to_reference_words`); `vreg` decodes `v0`/`v31`,
      rejects `v32`/non-vector names (`vector_register_names_decode_to_numbers`).

Acceptance: config-instruction words match the reference assembler; register
decode unit tests pass. No existing output changes. **MET** — `cargo test` green;
artifact-gate diffs=0 vs. up-to-date goldens (Corrections C4).
Commit: f55d26e1e

### Phase 2 — float vector ops (OPFVV) + conversions + FMA

- [x] Emit `vfadd/vfsub/vfmul/vfdiv/vfmin/vfmax.vv`, `vfmacc/vfnmsac.vv`,
      `vfsgnjn/vfsgnjx.vv`, `vfsqrt.v`, `vfcvt.rtz.x.f.v`/`vfcvt.x.f.v` (frm)/
      `vfcvt.f.x.v`, and `vmflt/vmfle/vmfeq.vv` via the `v_type` packer + table.
- [x] Tests: exact-word tests for each float mnemonic (SEW=64) vs. the reference
      assembler (rolled into `rvv_ops_encode_to_reference_words`).

Acceptance: every float vector mnemonic encodes to its reference word. **MET**.
Commit: f55d26e1e

### Phase 3 — integer vector ops, mask materialization, splat/extract, load/store

- [x] Emit `vadd/vsub/vand/vor/vxor.vv`, `vrsub.vx`, `vsll/vsra/vsrl.vi` and
      `.vx`, `vmslt/vmsle/vmseq.vv`, `vmerge.vim`, `vmv.v.i`, `vmv.v.x`,
      `vmv.x.s`, `vmv.s.x`, `vslidedown.vi`, `vle64.v`, `vse64.v`.
- [x] ~~`encode/sizing.rs`: register all new mnemonics as 4-byte words.~~ —
      moot: sizing is emitter-derived (bug-341-B3) — `instruction_size` runs a
      throwaway encode and takes the byte length, so a single-word RVV op sizes
      to 4 automatically with **no** table entry. See Corrections C1.
- [x] Tests: exact-word tests for each (`rvv_ops_encode_to_reference_words`);
      a sizing test asserting 4 bytes each (`rvv_ops_are_single_words_and_size_matches`).

Acceptance: every integer/mask/mem vector mnemonic encodes to its reference
word and sizes to 4 bytes; `encode/tests.rs` green. **MET**.
Commit: f55d26e1e

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

## Corrections

- **C1 — sizing is emitter-derived, no table.** `encode/sizing.rs` has had no
  per-mnemonic byte table since bug-341-B3: `instruction_size` runs a throwaway
  `emit_instruction` and returns `text.len()`. Every RVV op emits exactly one
  `emit_word`, so it sizes to 4 with zero sizing.rs change. Phase-3's "register
  all new mnemonics as 4-byte words" is therefore moot; a test
  (`rvv_ops_are_single_words_and_size_matches`) asserts the 4-byte size instead.
- **C2 — representation: one `RvVop` CodeOp + `vop` field, not ~44 variants.**
  The plan implied the RVV mnemonics would each be a `CodeOp` (its tests wrote
  `ci("vfadd.vv", …)`), but `CodeInstruction::new` requires a registered
  `CodeOp`, and adding ~44 would bloat the neutral-MIR vocabulary. Instead a
  single `CodeOp::RvVop` carries the specific mnemonic in a `vop` field, table-
  driven in the emitter (`emit_rv_vop`) — matching the existing `RvBr`/`RvFcmp`
  precedent (sub-op in a `cond`/`cmp` field) and plan §3.4's "table-drive the
  mnemonics". Footprint: one `mirror`-block MirOp variant (`src/arch/ops.rs`,
  `src/target/shared/code/{mir.rs,code_impl.rs}`). `vreg` returns
  `Result<u8,String>` (matching `reg`/`freg`), not `Option`; the negative test
  asserts the error message, not `None`.
- **C3 — `op_v` named `v_type`.** Follows the existing `r_type`/`i_type`/… packer
  naming and takes an extra `opcode` arg so the one function covers `OP-V`
  arithmetic **and** the LOAD-FP/STORE-FP vector load/store (same field layout).
- **C4 — reference assembler + artifact-gate baseline.** No `qemu-riscv64`
  user-mode nor a system `llvm-mc` on PATH, but the host has
  `/Users/justinzaun/local/brew/opt/llvm/bin/llvm-mc` and
  `riscv64-unknown-elf-as -march=rv64gcv`; every golden word was produced by the
  latter and cross-checked with the former. The artifact-gate on clean HEAD
  reports 24 `DIFF`s — all `macos-aarch64` `.ncode`, byte-identical to the 24
  `.ncodesum` goldens already modified-uncommitted in the main tree (stale
  committed goldens, bug-388 lineage), not produced by any code B touches.
  Adopting the main tree's regenerated goldens → **diffs=0** (1476 goldens, 4
  targets), proving B byte-identical. The 24 golden updates are out of plan-32's
  scope and are left to the owning main-tree change.

## Summary

A self-contained, byte-exactly-verifiable encoder for the RVV subset C needs.
Risk is confined to bit-field correctness and neutralized by per-mnemonic
reference-assembler tests. Nothing in the running compiler emits these words
until sub-plan C, so this lands with zero behavioral change.
