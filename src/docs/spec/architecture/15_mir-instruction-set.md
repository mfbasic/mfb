# MIR Instruction Set

The target-neutral machine IR (MIR) is the seam between the per-target code
builder and the backend: a single ISA-independent op vocabulary the backend
lowers *to* and selects *from*. It is the layer every backend plugs into, and
the `-mir` dump is its observable form. [[src/target/shared/code/mir.rs:MirOp]]

> **Status: this layer is under active construction.** MIR is being introduced
> one op-family at a time (the `plan-00-*` series in `planning/mir.md`), and the
> op set, mnemonics, and grouping below track the code *as it is today* — they
> will keep moving as more of the backend is neutralized. Treat the catalog as a
> current snapshot, not a frozen contract. The NIR the code builder derives from
> is `./mfb spec architecture native-ir`; how a backend realizes each MIR op as
> concrete instructions is the backend's own concern (for the in-tree backend,
> `./mfb spec architecture aarch64-instruction-set`).

## Design contract: identity round trip

MIR is introduced *underneath* the existing backend without changing a single
emitted byte. The raise/lower round trip `select ∘ lower_to_mir` is the
**identity** on the instruction stream: every backend op maps to exactly one
`MirOp` and back, and the fused/expand ops re-expand to the exact instruction
sequence the builders emit today. [[src/target/shared/code/mir.rs:lower_to_mir]]
[[src/target/shared/code/mir.rs:select_aarch64]]

The MIR is now the **sole** code path: plan-00-G flipped it on by default and
deleted the legacy `direct` (no-MIR) AArch64 backend. During plans A–F the round
trip was kept the identity and proven byte-identical against the `direct` path
(a differential self-diff) before each op family was neutralized; that gate has
been retired with the `direct` path. A coverage gap is still a *compile* error,
not a test miss — `from_code`/`to_code` are exhaustive matches over the backend
op set, so a missing variant fails the build.
[[src/target/shared/code/mir.rs:from_code]] [[src/target/shared/code/mir.rs:to_code]]

## Instruction model

A `MirInstruction` is `{ op: MirOp, fields: Vec<(&'static str, String)> }` — an
op tag plus an order-independent bag of named string operands. The register
allocator's field-based liveness works unchanged across the seam.
[[src/target/shared/code/mir.rs:MirInstruction]] Two neutral conventions replace
ISA-specific spellings in the field values:

- **`arena_base`** names the arena-state base pointer instead of a pinned
  physical register; a backend realizes it as whatever register or memory slot
  it pins the arena to. The rename is total and reversible because the arena
  base is pinned program-wide. [[src/target/shared/code/mir.rs:ARENA_BASE]]
  [[src/target/shared/code/mir.rs:rename_field_values]]
- Virtual registers (`%vN` integer, `%fN` float) appear in the pre-allocation
  `-mir` dump; the hand-written runtime helpers are shown over their final
  physical-register stream instead (see *The `-mir` dump*).

## Op groups

`MirOp` is one enum built from five groups by the `mir_ops!` macro; the group an
op belongs to decides how a backend maps it.
[[src/target/shared/code/mir.rs:MirOp]]

| Group | Maps to | Mnemonic | Purpose |
|-------|---------|----------|---------|
| **mirror** | one backend op, 1:1 | the op's own mnemonic | shapes whose spelling is already ISA-neutral (universal ALU/move/load/branch, `clz`/`rbit`/`msub`, scalar FP) |
| **renamed** | one backend op, 1:1 | ISA-neutral semantic name | ISA-specific scalar shapes given a portable name; selection still byte-identical |
| **simd** | one backend op, 1:1 | `v128.*` | fixed-width vector lane ops |
| **fused** | *two* instructions | explicit | flagless control flow: a compare/arith setter folded together with the flag-reading branch that consumed it |
| **expand** | *two* instructions | explicit | one structural op a backend realizes as a short fixed sequence |

Mirror, renamed, and simd ops are 1:1 with a backend op (`to_code` returns it).
Fused and expand ops have no single backend op (`to_code` is `None`) — they are
produced by fusing adjacent instruction pairs in `lower_to_mir` and expanded
back byte-for-byte at selection. [[src/target/shared/code/mir.rs:to_code]]

### renamed ops (neutral scalar names)

Each carries the ISA-neutral semantic name instead of an ISA-specific
instruction spelling. The mapping is 1:1, so selection and encoding stay
byte-identical; only the `-mir` mnemonic changes.
[[src/target/shared/code/mir.rs:mir_ops]]

| MIR mnemonic | Meaning |
|--------------|---------|
| `mulhi_s` / `mulhi_u` | signed / unsigned 64×64 → high 64 |
| `addc` | add with carry in/out |
| `rotr` / `rotr_w` | variable rotate-right (64 / 32-bit) |
| `bswap` / `bswap_w` | byte reverse (64 / 32-bit) |
| `f2i_trunc` | f64→i64 toward zero |
| `f2i_floor` / `f2i_ceil` | f64→i64 toward −∞ / +∞ |
| `f2i_nearest` | f64→i64 nearest, ties away |
| `i2f` | signed i64 → f64 |
| `fmov_i2f` / `fmov_f2i` | reinterpret bits between i64 and f64 |
| `call` / `call_indirect` | direct call to a symbol / call via a register |
| `syscall` | trap into the OS |

The ABI register placement (which GPRs carry call args, the syscall number, the
result) is a per-backend detail, deliberately *not* named in the op.

### simd ops (`v128.*`)

The vector vocabulary is a set of fixed-width lane ops in the `v128.*`
namespace. Lanes are `2×f64` / `2×i64` / `16×i8` as the op needs. The lane
*semantics* — NaN propagation of `v128.fmin`/`v128.fmax`, `bsl`/`bit` mask
polarity, round-mode ties, lane-compare all-ones/all-zeros masks — are the
contract every backend must realize against, pinned by an executable
lane-semantics test matrix. [[src/target/shared/code/mir.rs:mir_ops]] Examples:
`v128.load`/`v128.store`, `v128.fadd`/`v128.fmul`/`v128.fma`,
`v128.fmin`/`v128.fmax`, `v128.fcmp_gt`/`v128.icmp_ge`,
`v128.fround_even`/`v128.fround_trunc`, `v128.f2i_nearest`/`v128.i2f`,
`v128.and`/`v128.bsl`/`v128.bit`, `v128.dup_from_gpr`/`v128.umov_to_gpr`.

### fused ops (flagless control flow)

A flag-setter immediately followed by the flag-reading branch that consumed it
is fused into one flagless op, so the MIR carries no compare/branch pair with a
hidden condition-flag dependency. The operands and the branch condition are
carried explicitly; selection expands each back to the two instructions a
backend emits. [[src/target/shared/code/mir.rs:fused_variant]]
[[src/target/shared/code/mir.rs:fused_setter_codeop]]

| MIR mnemonic | Folds |
|--------------|-------|
| `br_cc` / `br_cc_imm` | compare-and-branch (register / immediate rhs) |
| `fbr_cc` / `fbr_cc_zero` | float compare-and-branch (vs register / vs zero) |
| `add_ovf` / `sub_ovf` | explicit-overflow arithmetic plus its overflow-trap branch |
| `syscall_br` | syscall plus its error-check branch |

The branch condition rides in a `cond` field; it marks the split point between
the setter operands and the branch operands. A second or third branch reading
the *same* comparison (a 3-way ordering such as `lo`/`hi` on one compare) carries
a `share` field so the backend emits only the branch, reusing the single shared
comparison. [[src/target/shared/code/mir.rs:FUSED_COND_FIELD]]
[[src/target/shared/code/mir.rs:FUSED_SHARE_FIELD]]

### expand ops (structural)

| MIR mnemonic | Meaning |
|--------------|---------|
| `addr_of` | PC-relative address of a symbol (`dst`, `symbol`) |

`addr_of` is one neutral op for materializing a symbol's address; a backend
realizes it natively as whatever PC-relative sequence it uses (a single
instruction or a short pair). [[src/target/shared/code/mir.rs:fuse_addr_of]]

## Authoring reference

This section is the per-op detail needed to *write* a well-formed MIR
instruction, not just read one.

### Operand grammar

Every operand is a string in the field bag, distinguished only by which field
names it. [[src/target/shared/code/mir.rs:MirInstruction]]

- **Registers.** `%vN` is a virtual integer/GPR value, `%fN` a virtual float
  value (`N` a decimal index), as they appear in the pre-allocation `-mir` dump.
  The hand-written runtime helpers are shown over physical registers instead.
  `arena_base` is the one named pinned operand (the arena-state base pointer).
- **Immediates.** A decimal integer literal as a string, carried in `value`
  (`mov_imm`), `imm` (`add_imm`/`sub_imm`/`sub_sp`/`add_sp`), `shift` (shift-by-
  immediate), `offset` (load/store displacement), or `index` (lane index).
- **Labels.** A `label` op's `name` field *defines* a zero-width branch target;
  a branch's `target` field *references* one.
- **Symbols.** A link symbol (e.g. `_mfb_fn_…`, `_mfb_rt_…`, a data symbol) in
  the `symbol` field (`addr_of`) or a call's `target`.

The field bag is order-independent for every op **except the fused ops**, where
order is significant: the `cond` field marks the boundary between the setter's
operands (before it) and the branch's operands (after it).
[[src/target/shared/code/mir.rs:FUSED_COND_FIELD]]

### Field signatures

Required fields per op (validated before encoding). Renamed and `v128.*` ops
carry the same fields as the shape they rename.
[[src/target/shared/code/code_impl.rs:validate]]

| Op(s) | Required fields | Role |
|-------|-----------------|------|
| `label` | `name` | define a branch target (zero-width) |
| `mov` | `dst`, `src` | register copy |
| `mov_imm` | `dst`, `value` | load an immediate |
| `add` `sub` `and` `orr` `eor` `mul` `sdiv` `udiv` `lslv` `lsrv` `asrv` `mulhi_s` `mulhi_u` `rotr` `rotr_w` | `dst`, `lhs`, `rhs` | binary integer op |
| `mvn` `clz` `rbit` `bswap` `bswap_w` | `dst`, `src` | unary integer op |
| `msub` | `dst`, `lhs`, `rhs`, `minuend` | `dst = minuend − lhs*rhs` |
| `lsl_imm` `lsr_imm` `asr_imm` | `dst`, `src`, `shift` | shift by immediate |
| `add_imm` `sub_imm` | `dst`, `src`, `imm` | add/sub immediate |
| `add_sp` `sub_sp` | `imm` | adjust the stack pointer |
| `cmp` `cmp_imm` | `lhs`, `rhs` | compare (usually fused — see below) |
| `b` | `target` | unconditional branch |
| `branch_self` `ret` `syscall` | *(none)* | self-loop / return / OS trap |
| `call` | `target` | direct call to a symbol |
| `call_indirect` | `register` | call via a register |
| `ldr_u64` `ldr_u32` `ldr_u16` `ldr_u8` `ldr_d` | `dst`, `base`, `offset` | load (width in the mnemonic) |
| `str_u64` `str_u32` `str_u8` `str_d` | `src`, `base`, `offset` | store |
| `fadd_d` `fsub_d` `fmul_d` `fdiv_d` | `dst`, `lhs`, `rhs` | binary f64 op |
| `fmov_d_from_d` `fneg_d` `fabs_d` `fsqrt_d` `i2f` `f2i_trunc` `f2i_floor` `f2i_ceil` `f2i_nearest` `fmov_i2f` `fmov_f2i` | `dst`, `src` | unary f64 / convert / reinterpret |
| `fmadd_d` | `dst`, `addend`, `lhs`, `rhs` | `dst = addend + lhs*rhs` |
| `addr_of` | `dst`, `symbol` | PC-relative symbol address |
| `v128.load` | `dst`, `base`, `offset` | 128-bit vector load |
| `v128.store` | `src`, `base`, `offset` | 128-bit vector store |
| `v128.fadd` `v128.fsub` `v128.fmul` `v128.fdiv` `v128.fmin` `v128.fmax` `v128.fcmp_gt` `v128.fcmp_ge` `v128.fcmp_eq` `v128.add` `v128.sub` `v128.icmp_gt` `v128.icmp_ge` `v128.icmp_eq` `v128.sshl` `v128.ushl` `v128.and` `v128.or` `v128.xor` | `dst`, `lhs`, `rhs` | binary lane op |
| `v128.fma` `v128.fms` `v128.bsl` `v128.bit` | `dst`, `lhs`, `rhs` | binary lane op where `dst` is **also a source** (accumulator / mask) |
| `v128.fabs` `v128.fneg` `v128.fsqrt` `v128.fround_*` `v128.f2i_trunc` `v128.f2i_nearest` `v128.i2f` `v128.neg` `v128.abs` `v128.fcmp_*_zero` | `dst`, `src` | unary lane op |
| `v128.shl_imm` `v128.sshr_imm` `v128.ushr_imm` | `dst`, `src`, `shift` | lane shift by immediate |
| `v128.dup_from_gpr` | `dst`, `src` | broadcast a GPR into all lanes |
| `v128.umov_to_gpr` | `dst`, `src`, `index` | extract lane `index` to a GPR |

The carry/borrow family (signed/unsigned add-with-carry) is mid-reshape from a
flag-based form to explicit `carry_in`/`carry_out` operands and is **not stable**
in the MIR vocabulary yet — author against it only once the `plan-00-*` work
settles. [[src/arch/aarch64/ops.rs:CodeOp]]

### Fused-op layout

A fused op's field bag is `[<setter fields>, cond=<condition>, <branch fields>,
share?]`. The setter fields are those of the compare/arith it folded; the branch
fields are the branch's (`target`). [[src/target/shared/code/mir.rs:lower_to_mir]]

| Op | Setter fields | Branch fields | Notes |
|----|---------------|---------------|-------|
| `br_cc` | `lhs`, `rhs` | `target` | integer compare-and-branch |
| `br_cc_imm` | `lhs`, `rhs` | `target` | `rhs` is an immediate |
| `fbr_cc` | `lhs`, `rhs` | `target` | float compare-and-branch |
| `fbr_cc_zero` | `src` | `target` | float compare vs zero |
| `add_ovf` `sub_ovf` | `dst`, `lhs`, `rhs` | `target` | branch taken on overflow |
| `syscall_br` | *(none)* | `target` | branch taken on syscall error |

An optional `share` field (value `"true"`) marks a branch that reuses the
immediately preceding fused op's comparison rather than recomputing it — used for
multi-way branches on one compare. [[src/target/shared/code/mir.rs:FUSED_SHARE_FIELD]]

### Condition codes

The `cond` field carries the branch condition as one of these values:

`b.eq` `b.ne` `b.ge` `b.lt` `b.gt` `b.le` `b.hi` `b.lo` `b.mi` `b.ls` `b.vc` `b.vs`

Signed orderings use `b.ge`/`b.lt`/`b.gt`/`b.le`; unsigned use
`b.hi`/`b.lo`/`b.ls`; `b.vs`/`b.vc` test overflow (set/clear) for `add_ovf`/
`sub_ovf` and the syscall error check. [[src/arch/aarch64/ops.rs:CodeOp]]

## The `-mir` dump (`mfb build -mir`)

`-mir` is the neutral counterpart to `-ncode`: it serializes the MIR stream
captured **before** register allocation and instruction selection, as stable,
versioned JSON. The whole-module form is deliberately ISA-independent — no
`target` / `arch` field, so diffing a `-mir` dump across targets is identical
where a per-target code dump is not. [[src/target/shared/code/mir.rs:MirPlan]]
[[src/target/shared/code/mod.rs:lower_module_mir_for_platform]]

Builder-emitted functions appear with their pre-allocation MIR (virtual
registers `%vN`/`%fN`); the hand-written runtime helpers, which never pass
through the pre-allocation seam, appear over their final physical-register
stream — so the dump is complete and honest about what is neutral versus not.
[[src/target/shared/code/mir.rs:build_mir_plan]]

Relocations carry their **neutral intent** (`call`, `data_addr_hi`,
`data_addr_lo`, `got_load_hi`, `got_load_lo`) rather than the concrete reloc kind
a backend picks. [[src/target/shared/code/mir.rs:MirRelocation]]
[[src/target/shared/code/types.rs:RelocIntent]]

```json
{
  "format": "mfb-mir",
  "version": 1,
  "project": "demo",
  "entrySymbol": "_mfb_fn_main",
  "functions": [
    {
      "name": "main",
      "symbol": "_mfb_fn_main",
      "returns": "Nothing",
      "params": [],
      "instructions": [
        { "op": "addr_of", "dst": "%v0", "symbol": "_mfb_str_0" },
        { "op": "mov", "dst": "%v1", "src": "%v0" },
        { "op": "call", "symbol": "_mfb_rt_io_io_print" }
      ],
      "relocations": [
        { "from": "_mfb_fn_main", "to": "_mfb_rt_io_io_print",
          "intent": "call", "binding": "global", "library": null }
      ]
    }
  ]
}
```

## See Also

- ./mfb spec architecture native-ir — the NIR the code builder derives from
- ./mfb spec architecture native — the per-target code plan and code generation
- ./mfb spec architecture artifacts — the `-mir` dump among build outputs
- ./mfb spec architecture aarch64-instruction-set — how the in-tree backend realizes MIR ops as concrete instructions
- ./mfb spec memory native-calling-convention — the register/ABI rules a backend realizes
- ./mfb spec linker symbols-and-relocations — how the neutral reloc intents are emitted
