# plan-00-C — Neutral Addressing, Immediates & Exotic Integer Ops

Last updated: 2026-06-29

Neutralize the remaining AArch64-specific *scalar* shapes so the MIR's integer/address
vocabulary is ISA-independent: PC-relative addressing, immediate materialization, and the
"exotic" integer ops that are not 1:1 across ISAs (`mir.md §4`).

Depends on plan-00-A/B. Stays AArch64-**byte-identical** under `-codegen mir`.

## 1. Goal

- **`addr_of <sym>`** replaces `Adrp` + `AddPageOff`. One MIR op; AArch64 selects the
  `adrp; add :lo12:` pair (x86 `lea` RIP-rel, rv64 `auipc; addi` later).
- **`mov_imm <any i64>`** — immediates are abstract in the MIR; the backend materializes
  (AArch64 `movz/movk` chain). No pre-encoded immediate forms (`AddImm`/`SubImm`/`CmpImm`
  keep their *small* immediate operands, but large constants flow as `mov_imm`).
- **Semantic exotic ops**, each lowering to AArch64 today and expandable elsewhere later:
  `mulhi_s`/`mulhi_u` (`smulh`/`umulh`), `clz`, `rbit`, `rev`/`bswap` (`rev`), `rotr`
  (`rorv`/`ror`), `addc` (`adc`), `msub`. These are the `mir.md §4` "mechanical expansion"
  set; here they get neutral names + the AArch64 lowering.
- **`f2i_<mode>`** names for the `FCvtzs/ms/ps/as` rounding-mode family (trunc/floor/ceil/
  nearest) and `i2f` for `scvtf`; bit-reinterpret `fmov_i2f`/`fmov_f2i`.

### Non-goals

- No expansion code for absent instructions yet (that is the per-ISA backends, plans H/I) —
  AArch64 has all of these natively, so this plan only renames+reshapes, byte-identically.
- SIMD ops are plan-00-E; helpers are plan-00-F.

## 2. Current State

`CodeOp` carries `Adrp`/`AddPageOff` (AArch64 PC-rel), `MovImm` (already abstract-ish —
the encoder emits `movz/movk`), `SMulH`/`UMulH`/`Clz`/`Rbit`/`RevW`/`RevX`/`Rorv`/`Adc`/
`MSub`, and the `FCvt*`/`SCvtf` family. The PCG64 RNG (`adc`, `umulh`, `rorv`), FNV hash
(`umulh`), and fdlibm (`clz`, bit ops) are the consumers.

## 3. Design

NIR→MIR emits the neutral ops; the AArch64 selector maps each back to its instruction(s)
byte-identically. `addr_of` is the only structural change (one MIR op → two AArch64 ops) —
selection emits the same `adrp; add_pageoff` pair the builders do today, with the same
relocations (relocation neutralization is plan-00-D).

## 4. Phases

1. `addr_of` (+ AArch64 `adrp; add_pageoff` selection) — retarget the symbol-address sites.
2. `mov_imm` abstract-immediate convention + the `movz/movk` materialization stays in the
   AArch64 encoder.
3. Neutral exotic-int ops (`mulhi_*`/`clz`/`rbit`/`rev`/`rotr`/`addc`/`msub`) + AArch64
   selection; retarget RNG/FNV/fdlibm emit sites.
4. `f2i_<mode>`/`i2f`/bit-reinterpret renames + selection.
5. Byte-identical gate.

## 5. Validation

- Suite **byte-identical** under `-codegen mir`. The RNG (`math::rand` reproducibility),
  the FNV map hashing, and fdlibm accuracy tests are the load-bearing checks (these are the
  exotic-op consumers).
- `-mir` dumps show neutral ops (no `adrp`/`smulh`/`rorv` mnemonics).

## Summary

The "rename + reshape the scalar ops to semantic, ISA-neutral forms" plan. Mostly
mechanical and byte-identical on AArch64; its value is that the MIR's integer/address
vocabulary stops naming AArch64 instructions, so the x86_64/rv64 backends have a clean,
semantic surface to select from (and to *expand* where they lack a native op).
