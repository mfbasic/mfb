# AArch64 Instruction Set

Native codegen does not emit textual assembly. It builds a list of
`CodeInstruction` values — one abstract op plus named string fields — and the
AArch64 backend turns each into one or more fixed 32-bit little-endian words.
The op repertoire is the closed `CodeOp` enum; the backend rejects any op it
cannot encode. [[src/arch/aarch64/ops.rs:CodeOp]] This topic specifies that
repertoire and its encodings. The register set, ABI, and which physical
registers each op may name are canonical in
`./mfb spec memory native-calling-convention`; the relocations that patch
`adrp`/`bl`/page-offset words are canonical in
`./mfb spec linker symbols-and-relocations`; the code plan these instructions
populate is `./mfb spec architecture native`.

All integer data ops are **64-bit (X-register)** forms; the loads/stores carry
an explicit access width in the mnemonic, and the floating-point ops are
**double-precision (D-register)**. The encoder writes each word with
`emit_word` (`u32::to_le_bytes`). [[src/arch/aarch64/encode/emitter.rs:emit_instruction]]

## Instruction model

A `CodeInstruction` is `{ op: CodeOp, fields: Vec<(&'static str, String)> }` —
an op tag and an order-independent set of named string operands.
[[src/target/shared/code/types.rs:CodeInstruction]] There is no typed operand
union: every operand (register name, immediate, label, symbol) is a string,
looked up by name at encode time via `field(instruction, "name")`. Construction
is fluent (`CodeInstruction::new(mnemonic).field("dst","x0")…`), and `validate`
checks that each op carries its required field names before encoding.
[[src/target/shared/code/validation.rs:validate]]

Operands are decoded by small helpers:

- `reg` maps a register name to a 0–31 number. `x0..x28`, `x30`/`lr`, and the
  `w*` aliases map to the same numbers; `sp`/`raw_sp`/`x31`/`xzr` all map to
  `31`; `d0..d7` map to `0..7`. There is **no `x18`** (platform-reserved) and
  **no `x29`** in the table. [[src/arch/aarch64/encode/operand.rs:reg]]
- `immediate` parses a `u64`, also accepting `"true"`→1 / `"false"`→0.
  [[src/arch/aarch64/encode/operand.rs:immediate]]
- `shift` parses a 0–63 shift amount. [[src/arch/aarch64/encode/operand.rs:shift]]

Most ops encode to exactly one word; the variable-length ops are covered under
*Multi-word lowerings* below. `instruction_size` computes the byte length ahead
of emission so label and symbol offsets are known before patching.
[[src/arch/aarch64/encode/sizing.rs:instruction_size]]

## Op catalog

Each row gives the op's mnemonic, its required fields, and the base 32-bit word
the encoder ORs operand bits into. Field placement is `Rd = bits[4:0]`,
`Rn = bits[9:5]`, `Rm = bits[20:16]` unless noted. Every encoder lives in
`encode.rs`. [[src/arch/aarch64/encode/emitter.rs:emit_instruction]]

| Op | Mnemonic | Fields | Base word | Notes |
|----|----------|--------|-----------|-------|
| `Label` | `label` | `name` | — | zero-width; records a branch target |
| `Mov` | `mov` | `dst`,`src` | `0xAA0003E0` | `orr Xd,xzr,Xm` |
| `MovImm` | `mov_imm` | `dst`,`value` | `0xD2800000` | `movz`+`movk` chain; multi-word |
| `Add` | `add` | `dst`,`lhs`,`rhs` | `0x8B000000` | reg add |
| `Adds` | `adds` | `dst`,`lhs`,`rhs` | `0xAB000000` | flag-setting add |
| `Sub` | `sub` | `dst`,`lhs`,`rhs` | `0xCB000000` | reg sub |
| `Subs` | `subs` | `dst`,`lhs`,`rhs` | `0xEB000000` | flag-setting sub |
| `And` | `and` | `dst`,`lhs`,`rhs` | `0x8A000000` | bitwise and |
| `Orr` | `orr` | `dst`,`lhs`,`rhs` | `0xAA000000` | bitwise or |
| `Eor` | `eor` | `dst`,`lhs`,`rhs` | `0xCA000000` | bitwise xor |
| `Mvn` | `mvn` | `dst`,`src` | `0xAA2003E0` | `orn Xd,xzr,Xm`, `Rm=bits[20:16]` |
| `Mul` | `mul` | `dst`,`lhs`,`rhs` | `0x9B007C00` | `madd` with `Ra=xzr` |
| `SMulH` | `smulh` | `dst`,`lhs`,`rhs` | `0x9B407C00` | signed high mul |
| `UMulH` | `umulh` | `dst`,`lhs`,`rhs` | `0x9BC07C00` | unsigned high mul |
| `Adc` | `adc` | `dst`,`lhs`,`rhs` | `0x9A000000` | add with carry |
| `Rorv` | `rorv` | `dst`,`lhs`,`rhs` | `0x9AC02C00` | variable rotate-right |
| `SDiv` | `sdiv` | `dst`,`lhs`,`rhs` | `0x9AC00C00` | signed divide |
| `UDiv` | `udiv` | `dst`,`lhs`,`rhs` | `0x9AC00800` | unsigned divide |
| `MSub` | `msub` | `dst`,`lhs`,`rhs`,`minuend` | `0x9B008000` | `Rd=lhs*rhs - minuend`; `minuend=Ra=bits[14:10]` |
| `LslImm` | `lsl_imm` | `dst`,`src`,`shift` | `0xD3400000` | `ubfm`; `immr=(64-s)&63`, `imms=63-s` |
| `LsrImm` | `lsr_imm` | `dst`,`src`,`shift` | `0xD3400000` | `ubfm`; `immr=s`, `imms=63` |
| `AsrImm` | `asr_imm` | `dst`,`src`,`shift` | `0x93400000` | `sbfm`; `immr=s`, `imms=63` |
| `AddImm` | `add_imm` | `dst`,`src`,`imm` | `0x91000000` | `imm12=bits[21:10]`, `sh=bit22`; multi-word |
| `SubImm` | `sub_imm` | `dst`,`src`,`imm` | `0xD1000000` | as `add_imm`; multi-word |
| `SubSp` | `sub_sp` | `imm` | `0xD1000000` | `sub_imm(sp,sp,imm)` |
| `AddSp` | `add_sp` | `imm` | `0x91000000` | `add_imm(sp,sp,imm)` |
| `CmpImm` | `cmp_imm` | `lhs`,`rhs` | `0xF100001F` | `subs xzr,Rn,#imm`; falls back to `mov_imm`+`cmp` |
| `Cmp` | `cmp` | `lhs`,`rhs` | `0xEB00001F` | `subs xzr,Rn,Rm` |
| `BranchEq` | `b.eq` | `target` | `0x54000000` | cond=`0`; `imm19<<5`; label-patched |
| `BranchNe` | `b.ne` | `target` | `0x54000001` | cond=`1` |
| `BranchGe` | `b.ge` | `target` | `0x5400000A` | cond=`a` |
| `BranchLt` | `b.lt` | `target` | `0x5400000B` | cond=`b` |
| `BranchGt` | `b.gt` | `target` | `0x5400000C` | cond=`c` |
| `BranchLe` | `b.le` | `target` | `0x5400000D` | cond=`d` |
| `BranchVc` | `b.vc` | `target` | `0x54000007` | cond=`7` (no overflow) |
| `BranchHi` | `b.hi` | `target` | `0x54000008` | cond=`8` (unsigned >) |
| `BranchLo` | `b.lo` | `target` | `0x54000003` | cond=`3` (unsigned <) |
| `BranchMi` | `b.mi` | `target` | `0x54000004` | cond=`4` (N set; IEEE float `<`) |
| `BranchLs` | `b.ls` | `target` | `0x54000009` | cond=`9` (C clear or Z set; IEEE float `<=`) |
| `Branch` | `b` | `target` | `0x14000000` | unconditional; `imm26`; label-patched |
| `BranchLink` | `bl` | `target` | `0x94000000` | emits a `branch26` relocation, not a label patch |
| `BranchLinkRegister` | `blr` | `register` | `0xD63F0000` | `Rn=bits[9:5]` |
| `BranchSelf` | `branch_self` | — | `0x14000000` | `b .` (offset 0); self-loop |
| `Svc` | `svc` | — | `0xD4000001` | `svc #0` |
| `Ret` | `ret` | — | `0xD65F03C0` | `ret x30` |
| `LdrU64` | `ldr_u64` | `dst`,`base`,`offset` | `0xF9400000` | scale 8; `imm12=off/8`; multi-word fallback |
| `LdrU32` | `ldr_u32` | `dst`,`base`,`offset` | `0xB9400000` | scale 4 |
| `LdrU16` | `ldr_u16` | `dst`,`base`,`offset` | `0x79400000` | scale 2 |
| `LdrU8` | `ldr_u8` | `dst`,`base`,`offset` | `0x39400000` | scale 1 |
| `StrU64` | `str_u64` | `src`,`base`,`offset` | `0xF9000000` | scale 8; `src=Rt` |
| `StrU32` | `str_u32` | `src`,`base`,`offset` | `0xB9000000` | scale 4 |
| `StrU8` | `str_u8` | `src`,`base`,`offset` | `0x39000000` | scale 1 |
| `Adrp` | `adrp` | `dst`,`symbol` | `0x90000000` | emits a `page21` relocation |
| `AddPageOff` | `add_pageoff` | `dst`,`symbol` | `0x91000000`+`(Rd<<5)|Rd` | emits a `pageoff12` relocation |
| `FMovXFromD` | `fmov_x_from_d` | `dst`,`src` | `0x9E660000` | `Xd ← Dn` bits |
| `FMovDFromX` | `fmov_d_from_x` | `dst`,`src` | `0x9E670000` | `Dd ← Xn` bits |
| `FAddD` | `fadd_d` | `dst`,`lhs`,`rhs` | `0x1E602800` | double add |
| `FSubD` | `fsub_d` | `dst`,`lhs`,`rhs` | `0x1E603800` | double sub |
| `FMulD` | `fmul_d` | `dst`,`lhs`,`rhs` | `0x1E600800` | double mul |
| `FDivD` | `fdiv_d` | `dst`,`lhs`,`rhs` | `0x1E601800` | double div |
| `FNegD` | `fneg_d` | `dst`,`src` | `0x1E614000` | double negate |
| `FSqrtD` | `fsqrt_d` | `dst`,`src` | `0x1E61C000` | double sqrt |
| `FCmpD` | `fcmp_d` | `lhs`,`rhs` | `0x1E602000` | `Dm=bits[20:16]`, `Dn=bits[9:5]` |
| `FCmpZeroD` | `fcmp_zero_d` | `src` | `0x1E602000`+`0x8` | `fcmp Dn,#0.0` |
| `SCvtfDFromX` | `scvtf_d_from_x` | `dst`,`src` | `0x9E620000` | signed int → double |
| `FCvtzsXFromD` | `fcvtzs_x_from_d` | `dst`,`src` | `0x9E780000` | double → int, round toward zero |
| `FCvtmsXFromD` | `fcvtms_x_from_d` | `dst`,`src` | `0x9E700000` | double → int, round toward −∞ |
| `FCvtpsXFromD` | `fcvtps_x_from_d` | `dst`,`src` | `0x9E680000` | double → int, round toward +∞ |
| `FCvtasXFromD` | `fcvtas_x_from_d` | `dst`,`src` | `0x9E640000` | double → int, round to nearest, ties away |

Notes on the float ops: the `fmov`/`fcvt`/`scvtf` family places the operand
register in `bits[9:5]` (`src`) and the destination in `bits[4:0]` (`dst`), the
same `(src<<5)|dst` pattern the integer single-source ops use, even though one
side is a D-register and the other an X-register.
[[src/arch/aarch64/encode/emitter.rs:emit_scvtf_d_from_x]] The double-precision set is
exactly: `fadd_d`, `fsub_d`, `fmul_d`, `fdiv_d`, `fneg_d`, `fsqrt_d`, `fcmp_d`,
`fcmp_zero_d`, `fmov_x_from_d`, `fmov_d_from_x`, `scvtf_d_from_x`,
`fcvtzs_x_from_d`, `fcvtms_x_from_d`, `fcvtps_x_from_d`, `fcvtas_x_from_d` —
note `fneg_d`, `fsqrt_d`, `fcmp_d`, and `fcmp_zero_d` are present in addition to
the four arithmetic and five conversion ops.

## NEON vector ops

The vectorized `math::` array overloads (`mfb spec language builtin-functions`
§18.2) process two 64-bit lanes per instruction. These ops take **vector
register** operands named `v0`..`v31` (the `ldr_q`/`str_q` data register also
accepts the `q0`..`q31` spelling); the lane arrangement is fixed by the op —
`.2d` (two i64/f64 lanes) for every numeric op, `.16b` for the bitwise/select
ops `and_v`/`orr_v`/`eor_v`/`bsl_v`/`bit_v`. Field placement matches the scalar
ops: `Vd=bits[4:0]`, `Vn=bits[9:5]`, `Vm=bits[20:16]`.
[[src/arch/aarch64/encode/emitter.rs:emit_v_three_same]]

| Op | Mnemonic | Fields | Base word | Notes |
|----|----------|--------|-----------|-------|
| `LdrQ` | `ldr_q` | `dst`,`base`,`offset` | `0x3DC00000` | 128-bit load; `imm12=off/16` |
| `StrQ` | `str_q` | `src`,`base`,`offset` | `0x3D800000` | 128-bit store; `src=Vt` |
| `FAddV` | `fadd_v` | `dst`,`lhs`,`rhs` | `0x4E60D400` | `fadd .2d` |
| `FSubV` | `fsub_v` | `dst`,`lhs`,`rhs` | `0x4EE0D400` | `fsub .2d` |
| `FMulV` | `fmul_v` | `dst`,`lhs`,`rhs` | `0x6E60DC00` | `fmul .2d` |
| `FDivV` | `fdiv_v` | `dst`,`lhs`,`rhs` | `0x6E60FC00` | `fdiv .2d` |
| `FMlaV` | `fmla_v` | `dst`,`lhs`,`rhs` | `0x4E60CC00` | `dst += lhs*rhs` (Horner) |
| `FMlsV` | `fmls_v` | `dst`,`lhs`,`rhs` | `0x4EE0CC00` | `dst -= lhs*rhs` |
| `FMinV` | `fmin_v` | `dst`,`lhs`,`rhs` | `0x4EE0F400` | `fmin .2d` |
| `FMaxV` | `fmax_v` | `dst`,`lhs`,`rhs` | `0x4E60F400` | `fmax .2d` |
| `FCmGtV` | `fcmgt_v` | `dst`,`lhs`,`rhs` | `0x6EE0E400` | lane mask `lhs>rhs` |
| `FCmGeV` | `fcmge_v` | `dst`,`lhs`,`rhs` | `0x6E60E400` | lane mask `lhs>=rhs` |
| `FCmEqV` | `fcmeq_v` | `dst`,`lhs`,`rhs` | `0x4E60E400` | lane mask `lhs==rhs` |
| `AddV` | `add_v` | `dst`,`lhs`,`rhs` | `0x4EE08400` | integer `add .2d` |
| `SubV` | `sub_v` | `dst`,`lhs`,`rhs` | `0x6EE08400` | integer `sub .2d` |
| `CmGtV` | `cmgt_v` | `dst`,`lhs`,`rhs` | `0x4EE03400` | signed lane mask `lhs>rhs` |
| `CmGeV` | `cmge_v` | `dst`,`lhs`,`rhs` | `0x4EE03C00` | signed lane mask `lhs>=rhs` |
| `CmEqV` | `cmeq_v` | `dst`,`lhs`,`rhs` | `0x6EE08C00` | lane mask `lhs==rhs` |
| `SshlV` | `sshl_v` | `dst`,`lhs`,`rhs` | `0x4EE04400` | signed per-lane variable shift |
| `UshlV` | `ushl_v` | `dst`,`lhs`,`rhs` | `0x6EE04400` | unsigned per-lane variable shift |
| `AndV` | `and_v` | `dst`,`lhs`,`rhs` | `0x4E201C00` | `and .16b` |
| `OrrV` | `orr_v` | `dst`,`lhs`,`rhs` | `0x4EA01C00` | `orr .16b` |
| `EorV` | `eor_v` | `dst`,`lhs`,`rhs` | `0x6E201C00` | `eor .16b` |
| `BslV` | `bsl_v` | `dst`,`lhs`,`rhs` | `0x6E601C00` | bit-select: `dst = (lhs&dst)\|(rhs&~dst)` |
| `BitV` | `bit_v` | `dst`,`lhs`,`rhs` | `0x6EA01C00` | insert-if-true |
| `FAbsV` | `fabs_v` | `dst`,`src` | `0x4EE0F800` | `fabs .2d` |
| `FNegV` | `fneg_v` | `dst`,`src` | `0x6EE0F800` | `fneg .2d` |
| `FSqrtV` | `fsqrt_v` | `dst`,`src` | `0x6EE1F800` | `fsqrt .2d` |
| `FRintpV` | `frintp_v` | `dst`,`src` | `0x4EE18800` | round toward +∞ |
| `FRintmV` | `frintm_v` | `dst`,`src` | `0x4E619800` | round toward −∞ |
| `FRintaV` | `frinta_v` | `dst`,`src` | `0x6E618800` | round nearest, ties away |
| `FRintnV` | `frintn_v` | `dst`,`src` | `0x4E618800` | round nearest, ties even |
| `FRintzV` | `frintz_v` | `dst`,`src` | `0x4EE19800` | round toward zero |
| `FCvtzsV` | `fcvtzs_v` | `dst`,`src` | `0x4EE1B800` | f64 lane → i64, toward zero |
| `FCvtasV` | `fcvtas_v` | `dst`,`src` | `0x4E61C800` | f64 lane → i64, ties away |
| `ScvtfV` | `scvtf_v` | `dst`,`src` | `0x4E61D800` | i64 lane → f64 |
| `NegV` | `neg_v` | `dst`,`src` | `0x6EE0B800` | integer negate `.2d` |
| `AbsV` | `abs_v` | `dst`,`src` | `0x4EE0B800` | integer absolute `.2d` |
| `FCmGtZeroV` | `fcmgt_zero_v` | `dst`,`src` | `0x4EE0C800` | lane mask `src>0.0` |
| `FCmGeZeroV` | `fcmge_zero_v` | `dst`,`src` | `0x6EE0C800` | lane mask `src>=0.0` |
| `FCmEqZeroV` | `fcmeq_zero_v` | `dst`,`src` | `0x4EE0D800` | lane mask `src==0.0` |
| `FCmLtZeroV` | `fcmlt_zero_v` | `dst`,`src` | `0x4EE0E800` | lane mask `src<0.0` |
| `FCmLeZeroV` | `fcmle_zero_v` | `dst`,`src` | `0x6EE0D800` | lane mask `src<=0.0` |
| `ShlV` | `shl_v` | `dst`,`src`,`shift` | `0x4F005400` | left shift; `immhb=64+s`, `s∈0..=63` |
| `SshrV` | `sshr_v` | `dst`,`src`,`shift` | `0x4F000400` | signed right; `immhb=128−s`, `s∈1..=64` |
| `UshrV` | `ushr_v` | `dst`,`src`,`shift` | `0x6F000400` | unsigned right; `immhb=128−s` |
| `DupVFromX` | `dup_v_from_x` | `dst`,`src` | `0x4E080C00` | broadcast GPR into both `.2d` lanes |
| `UmovXFromV` | `umov_x_from_v` | `dst`,`src`,`index` | `0x4E003C00` | `Xd ← Vn.d[index]`; `imm5=8\|(index<<4)` |
| `FMaddD` | `fmadd_d` | `dst`,`addend`,`lhs`,`rhs` | `0x1F400000` | scalar `Dd = Da + Dn*Dm` (one round); `Ra=bits[14:10]` |

The scalar `fmadd_d` (and the `d`-register decoding extended to `d8`..`d31`)
backs the double-double recombination in the Float transcendental kernels, which
also rely on the internal runtime symbol `_mfb_simd_alloc_list(count, typeCode)`
(allocates a tight homogeneous numeric `List`; `mfb spec memory collections`).

Integer `min`/`max`/`mul`/`clz` have no `.2d` form in NEON, so the kernels build
those from `cmgt_v`+`bsl_v` (lane select) and integer shifts instead. A per-lane
error mask (e.g. `fcmlt_zero_v` for a negative `sqrt` lane) is reduced to a GPR
with two `umov_x_from_v` extracts ORed together — there is no horizontal-reduce
op in the set. Shift-immediate ops encode the amount in `immhb=bits[22:16]`. Every
new op's exact word is pinned by `encodes_neon_vector_ops` in `encode.rs`, which
asserts against the system assembler (`as -arch arm64`) output.

## Worked encodings

### `mov_imm` — `movz`/`movk` chain

`emit_mov_imm` always emits a `movz` for the low 16 bits (`0xD2800000`), then a
`movk` (`0xF2800000`) for each of the three higher 16-bit lanes that is nonzero;
a value of `0` emits the single `movz`. The lane index `hw` (0..3) goes in
`bits[22:21]` of the `movk`, the 16-bit chunk in `bits[20:5]`, `Rd` in
`bits[4:0]`. [[src/arch/aarch64/encode/emitter.rs:emit_mov_imm]]

```text
mov_imm x0, #0x1234_0000_5678          ; value = 0x0000_1234_0000_5678

  lane 0 = 0x5678  -> movz x0,#0x5678
    0xD2800000 | (0x5678 << 5) | 0      = 0xD28ACF00
  lane 1 = 0x0000  -> skipped
  lane 2 = 0x1234  -> movk x0,#0x1234,lsl#32
    0xF2800000 | (2 << 21) | (0x1234 << 5) | 0 = 0xF2C24680
```

`wide_imm_word_count` precomputes this length (1 + nonzero high lanes) so the
op's byte size is known before emission. [[src/arch/aarch64/encode/sizing.rs:wide_imm_word_count]]

### `adrp` + `add_pageoff` — symbol address

A symbol's absolute address is materialized in two instructions, each emitting
its own relocation rather than a finished immediate; the linker fills the page
and page-offset bits. [[src/arch/aarch64/encode/emitter.rs:emit_symbol_ref]]

```text
adrp x1, _sym            -> 0x90000000 | 1            = 0x90000001
                            + relocation { page21, target=_sym }
add_pageoff x1, _sym     -> 0x91000000 | (1<<5) | 1   = 0x91000021
                            + relocation { pageoff12, target=_sym }
```

The relocation `binding` is `external` (with the resolved `library`) when the
symbol is an import, else `data`. The base words carry zero in their immediate
fields; the page/offset bits are entirely supplied by the relocation
(`./mfb spec linker symbols-and-relocations`).

### `umulh` — single-word data op

```text
umulh x14, x11, x9       ; dst=14, lhs(Rn)=11, rhs(Rm)=9

  0x9BC07C00 | (9 << 16) | (11 << 5) | 14 = 0x9BC97D6E
```

This exact word (`0x9BC97D6E`) is the value the encoder's unit test asserts.
[[src/arch/aarch64/encode/tests.rs:encodes_umulh_adc_and_rorv]]

## Multi-word lowerings

Several ops expand to more than one word when an immediate or offset will not
fit a single field; `instruction_size` mirrors each expansion so layout stays
exact. [[src/arch/aarch64/encode/sizing.rs:instruction_size]]

- **`add_imm`/`sub_imm`/`add_sp`/`sub_sp`.** A value that fits the 12-bit
  immediate (optionally `<<12`) is one word; otherwise the encoder emits a chain
  of `add`/`sub` chunks accumulating the full value, chaining `Rd` as the source
  of each subsequent chunk. `encode_add_sub_imm` decides the single-word case;
  `next_add_sub_chunk` drives the chain. [[src/arch/aarch64/encode/emitter.rs:emit_add_imm]]
- **`cmp_imm`.** A 12-bit immediate encodes directly as
  `subs xzr,Rn,#imm`; otherwise the value is loaded with `mov_imm` into a
  scratch register (`scratch_excluding`) and compared register-to-register.
  [[src/arch/aarch64/encode/emitter.rs:emit_cmp_imm]]
- **Loads/stores.** Each requires its offset to be a multiple of the access
  width (else an encode error) and, when scaled, ≤ 4095. A larger offset is
  materialized by `add_imm` into a scratch base register, then the access uses
  that base at offset 0. [[src/arch/aarch64/encode/emitter.rs:emit_ldr_u64]]
- **`mov_imm`.** The `movz`/`movk` chain above.

## Labels, branches, and relocations

`Label` instructions are zero-width markers; the encoder records each label's
byte offset within its function. [[src/arch/aarch64/encode/emitter.rs:emit_instruction]]
Conditional branches and the local unconditional `b` are emitted as a
placeholder `0` word plus a deferred `LabelPatch`; after a function's body is
laid out, `patch_labels` computes the PC-relative displacement and rewrites the
word in place — `imm26` (`branch_imm26`) for `b`, `imm19<<5` (`branch_imm19`)
for the conditional set. [[src/arch/aarch64/encode/emitter.rs:patch_labels]] These
intra-function branches never produce relocations.

`bl` is different: it targets a **symbol**, not a label. The encoder emits the
bare `0x94000000` and a `branch26` relocation — `internal` binding if the target
is a function symbol in this image, `external` (with the import library) if it
is an imported symbol; an unresolved target is an encode error.
[[src/arch/aarch64/encode/emitter.rs:emit_bl]] `blr` (indirect call), `ret`, `svc`, and
`branch_self` carry no relocation and no label.

## See Also

* ./mfb spec architecture native — the native code plan these instructions populate
* ./mfb spec memory native-calling-convention — registers, ABI, and clobber sets
* ./mfb spec linker symbols-and-relocations — the relocation kinds that patch `adrp`/`bl`/page-offset words
* ./mfb spec architecture ir — the IR lowered into this instruction stream
