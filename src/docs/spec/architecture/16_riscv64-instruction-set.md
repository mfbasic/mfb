# RISC-V Instruction Set

The `linux-riscv64` backend is the third ISA after AArch64 and x86-64, and the
one that most exercises the neutrality of the target-neutral MIR: RISC-V has **no
condition flags** (so a flagless compare-and-branch is the only way one lowers at
all) and **no 128-bit vector file** on the base target (so `v128` ops scalarize
to `2× f64`). It consumes the **same neutral MIR** the shared builders and helpers
produce, plugging in through the backend trait with no edits to the other two
paths. [[src/arch/riscv64/mod.rs:1]] [[src/arch/riscv64/backend.rs:Riscv64Backend]]
This topic specifies the RISC-V op repertoire, its encodings, and how selection
turns each neutral op into RISC-V machine code. The shared neutral MIR is
`./mfb spec architecture mir-instruction-set`; the register set and ABI roles are
canonical in `./mfb spec memory native-calling-convention`; the relocation kinds
are canonical in `./mfb spec linker symbols-and-relocations`.

The base ISA is **RV64GC** (RVA20 — RV64IMAFDC) under the Linux **lp64d** ABI:
64-bit registers, hardware integer multiply/divide (`M`), atomics (`A`), and
single/double floating point (`F`/`D`). The encoder emits only **fixed 32-bit
little-endian words**; it never emits the compressed 16-bit (`C`) forms, even
though the target permits them. Two extensions the codegen would benefit from are
**not** used: there is no bit-manipulation (`Zbb`) — `clz`, bit/byte reversal, and
rotate are synthesized from base-ISA sequences — and there is no vector (`V`)
extension, so the neutral `v128` vocabulary is realized on memory, not registers.
Hardware fused multiply-add lives in the base `D` extension, so the ≤1-ULP math
kernels (`./mfb spec architecture math-kernels`) hold natively.
[[src/arch/riscv64/encode/mod.rs:1]] [[src/arch/riscv64/encode/emitter.rs:emit_instruction]]

## Instruction model

Like the other backends, this one does not emit textual assembly. It selects the
neutral MIR into a list of `CodeInstruction` values — an abstract op tag plus a
bag of named string operands — and encodes each into one or more 32-bit words.
Every operand (register name, immediate, label, symbol) is a string, looked up by
field name at encode time. The op repertoire is the shared closed `CodeOp` enum
plus a handful of RISC-V-specific ops (below); the encoder rejects any op it does
not encode with an explicit error rather than miscompiling.
[[src/arch/riscv64/encode/emitter.rs:emit_instruction]]
[[src/arch/riscv64/encode/operand.rs:field]]

Encoding is a **two-pass** walk, identical in shape to the AArch64 encoder. The
first pass assigns each text symbol its byte offset using a sizing function; the
second pass records `label` offsets, emits the bytes, and then patches every
intra-function branch displacement. The sizing function **must** return exactly
the byte count the emitter produces for the same instruction, or layout would
drift; the two share the immediate-materialization and bit-manipulation length
tables so they cannot disagree. [[src/arch/riscv64/encode/mod.rs:encode]]
[[src/arch/riscv64/encode/sizing.rs:instruction_size]]

Because RISC-V has fixed-width instructions but a tighter branch reach than
AArch64 (a native conditional branch reaches only ±4 KiB versus ±1 MiB), the
flagless compare-and-branch is **always** emitted in an 8-byte long form (an
inverted short branch over an unconditional `jal`) so its size is deterministic
and it reaches ±1 MiB — no branch-relaxation pass is needed.
[[src/arch/riscv64/encode/mod.rs:1]]

Operands are decoded by small helpers. Integer registers are named by their lp64d
ABI roles (`zero`, `ra`, `sp`, `gp`, `tp`, `t0`–`t6`, `s0`/`fp`, `s1`–`s11`,
`a0`–`a7`) and float registers likewise (`ft0`–`ft11`, `fs0`–`fs11`,
`fa0`–`fa7`), each decoded to its 0–31 number; immediates parse a `u64` (also
accepting `"true"`→1 / `"false"`→0); a shift immediate is validated into 0–63.
[[src/arch/riscv64/encode/operand.rs:reg]] [[src/arch/riscv64/encode/operand.rs:freg]]
[[src/arch/riscv64/encode/operand.rs:shift]]

## Register model

The register model answers the same questions the ISA-neutral linear-scan
allocator asks, over the RISC-V file of 32 GPRs and 32 FP registers named by
their lp64d ABI roles. [[src/arch/riscv64/regmodel.rs:Riscv64RegisterModel]]

- **32 GPRs.** `zero` (`x0`) is the hardware zero; `ra` the link register; `sp`
  the stack pointer; `gp`/`tp` fixed; `t0`–`t6` temporaries; `s0`–`s11`
  callee-saved; `a0`–`a7` argument/return. RISC-V's 32+32 registers are generous,
  so the allocatable pool is large and the allocator rarely spills.
- **Pinned and reserved GPRs (why they are not allocatable):** `zero`/`ra`/`sp`/
  `gp`/`tp` (fixed roles); `a0`–`a7` (ABI argument/return/syscall-arg, placed
  physically at call boundaries by selection); `t0`–`t2` (reserved lowering
  scratch for immediate materialization, overflow detection, and the float-compare
  boolean); `s0` (frame pointer); `s11` (**pinned `arena_base`** — the RISC-V
  analog of AArch64's pinned `x19`, initialized once by the program entry); `s10`
  (realizes the closure-environment role token); and `s2` (realizes the
  worker current-thread role token). The remaining GPRs — `t3`–`t6` and
  `s1`/`s3`–`s9` — form the integer allocatable set.
  [[src/arch/riscv64/regmodel.rs:INT_ALLOCATABLE]]
  [[src/arch/riscv64/regmodel.rs:ARENA_BASE_REGISTER]]
- **32 FP registers.** An FP virtual register carries a single `f64` (there is no
  128-bit file). `ft0`–`ft2` are reserved lowering scratch (float-compare staging
  and the three FP lanes a scalarized fused-multiply-add needs) and `fa0`–`fa7`
  are the FP argument/return bank placed at call boundaries; the allocatable FP
  set is `ft3`–`ft11` and `fs0`–`fs11`. [[src/arch/riscv64/regmodel.rs:FP_ALLOCATABLE]]
- **Caller-saved (volatile):** `ra`, `t0`–`t6`, `a0`–`a7` for GPRs; `ft0`–`ft11`,
  `fa0`–`fa7` for FP. **Callee-saved:** `s0`–`s11` and `fs0`–`fs11`.
  [[src/arch/riscv64/regmodel.rs:Riscv64RegisterModel]]
- **Spill slots** are 16 bytes (keeping every offset 16-aligned, matching the
  shared frame math); an integer spill uses `sd`/`ld`, an FP spill stores a single
  `f64` with `fsd`/`fld`. [[src/arch/riscv64/regmodel.rs:spill_slot_bytes]]

## Op catalog

Each neutral op maps to one RISC-V instruction, or — where the base ISA has no
single form — to a short deterministic sequence. The single-instruction cases:

| Op | RISC-V | Notes |
|----|--------|-------|
| `mov` | `addi rd,rs,0` (`mv`) | register copy |
| `mov_imm` | `li` sequence | `lui`/`addi`/`slli` chain; see *Immediate materialization* |
| `add` `sub` `and` `orr` `eor` | `add` `sub` `and` `or` `xor` | R-type integer |
| `mul` | `mul` | `M` extension |
| `smulh` `umulh` | `mulh` `mulhu` | signed / unsigned high 64 |
| `sdiv` `udiv` | `div` `divu` | |
| `rv.slt` `rv.sltu` | `slt` `sltu` | set-less-than (signed / unsigned) |
| `lslv` `lsrv` `asrv` | `sll` `srl` `sra` | variable shifts |
| `lsl_imm` `lsr_imm` `asr_imm` | `slli` `srli` `srai` | shift by immediate |
| `mvn` | `xori rd,rs,-1` (`not`) | |
| `sxtw` | `addiw rd,rs,0` (`sext.w`) | sign-extend low 32 bits |
| `msub` | `mul; sub` | `dst = minuend − lhs*rhs` |
| `add_imm` `sub_imm` | `addi` (or `li; add`/`sub`) | wide immediates stage through `t0` |
| `add_sp` `sub_sp` | `addi sp,sp,±imm` | stack adjust |
| `ldr_u64` `ldr_u32` `ldr_u16` `ldr_u8` | `ld` `lwu` `lhu` `lbu` | zero-extending loads |
| `str_u64` `str_u32` `str_u8` | `sd` `sw` `sb` | stores |
| `ldr_d` `str_d` | `fld` `fsd` | double load/store |
| `b` | `jal zero,label` | unconditional; patched |
| `bl` | `auipc ra; jalr ra` (`call`) | see *Calls, labels, relocations* |
| `blr` | `jalr ra,0(rs)` | indirect call |
| `branch_self` | `jal zero,0` | self-loop |
| `svc` | `ecall` | OS trap |
| `ret` | `jalr zero,0(ra)` | |
| `adrp` | `auipc rd,%pcrel_hi` | high half of a PC-relative address |
| `add_pageoff` | `addi rd,rd,%pcrel_lo` (or `ld` for a GOT symbol) | low half |

Field placement follows the RISC-V formats (`rd = bits[11:7]`, `rs1 = bits[19:15]`,
`rs2 = bits[24:20]`, funct3/funct7 selecting the operation), decoded once per op.
[[src/arch/riscv64/encode/emitter.rs:emit_instruction]]

### Floating point (scalar double)

All FP ops are double-precision. `fadd.d`/`fsub.d`/`fmul.d`/`fdiv.d` are the
R-type arithmetic; `fmin.d`/`fmax.d` back `fminnm_d`/`fmaxnm_d` and implement the
IEEE-number semantics (a finite operand wins over a NaN), matching AArch64's
`fminnm`/`fmaxnm`. The sign-manipulation trio maps to the `fsgnj` family: register
move (`fmov_d_from_d` → `fmv.d`), negate (`fneg_d` → `fsgnjn.d`), and absolute
value (`fabs_d` → `fsgnjx.d`). `fsqrt.d` computes the square root. Bit reinterprets
between a GPR and an FP register are `fmv.x.d`/`fmv.d.x`; the integer↔double
conversions are `fcvt.d.l` (signed i64 → double) and `fcvt.l.d` (double → i64)
with the rounding mode selecting toward-zero, toward −∞, toward +∞, or
nearest-ties-away. A float compare (`rv.fcmp`) is `feq.d`/`flt.d`/`fle.d` writing
a 0/1 GPR. [[src/arch/riscv64/encode/emitter.rs:emit_fp_r]]
[[src/arch/riscv64/encode/emitter.rs:emit_fcvt_l_d]]

The scalar fused multiply-add family is the one place RISC-V's native
instructions line up **1:1 by name** with the neutral MIR, needing none of the
result-renaming the other two backends do: `fmadd_d`→`fmadd.d`, `fmsub_d`→`fmsub.d`,
`fnmsub_d`→`fnmsub.d`, `fnmadd_d`→`fnmadd.d`, all single-rounding R4-type
instructions with `rs1=lhs`, `rs2=rhs`, `rs3=addend`.
[[src/arch/riscv64/encode/emitter.rs:emit_instruction]]

### Base-ISA synthesized ops (no `Zbb`)

RV64GC has no `clz`/`ctz`/`rev8`/`brev8`/`ror`, so five ops expand to base-ISA
sequences whose exact length is computed from shared level tables (so the sizing
pass and the emitter agree byte-for-byte):
[[src/arch/riscv64/encode/sizing.rs:instruction_size]]

- **`rorv` / `rorv_w`** — rotate-right by a variable amount as
  `(x >> s) | (x << (64−s))`, using the ISA's 6-bit shift-amount masking so a
  `−s` gives `(64−s)&63`. The 32-bit form uses word shifts (`srlw`/`sllw`) and
  zero-extends the low 32 bits. [[src/arch/riscv64/encode/emitter.rs:emit_instruction]]
- **`clz`** — smears the highest set bit down to bit 0, then `64 − popcount`
  (a SWAR popcount) counts the leading zeros (64 when the input is zero).
  [[src/arch/riscv64/encode/emitter.rs:emit_clz]]
- **`rbit` / `rev_x` / `rev_w`** — bit reversal, 64-bit byte reversal, and
  32-bit byte reversal, each a chain of parallel masked swaps at successive
  granularities, finishing with a 32-bit half swap.
  [[src/arch/riscv64/encode/emitter.rs:emit_reversal]]

### Explicit-carry arithmetic

RISC-V has no carry flag, so the explicit-carry ops carry the carry as a **value**.
`add_carry` computes `dst = lhs + rhs + carry_in` and derives `carry_out` from two
`sltu` comparisons; `sub_borrow` is the mirror using `sub`/`sltu`. Each expands to
a fixed 7-instruction sequence (correct even when the carry operands are `zero`),
so its size is deterministic. [[src/arch/riscv64/encode/emitter.rs:emit_add_carry]]

## Encoding

The emitter models the standard RISC-V instruction formats as small functions,
each packing its operand and function-select fields into a 32-bit word written
little-endian: [[src/arch/riscv64/encode/emitter.rs:emit_instruction]]

- **R-type** — `funct7 | rs2 | rs1 | funct3 | rd | opcode` (register-register ALU,
  and the FP ops via the `OP_FP` opcode).
- **I-type** — a signed 12-bit immediate in the top field (`addi`, loads, `jalr`,
  the shift-immediates, `ecall`).
- **S-type** — the store immediate split across two fields.
- **B-type** — the conditional-branch immediate, scrambled across the word
  (used inside the long-form compare-and-branch).
- **U-type** — a 20-bit upper immediate (`lui`, `auipc`).
- **J-type** — the `jal` immediate (used for `b`, `branch_self`, and the long-form
  branch's inner jump).
- **R4-type** — the four-register fused multiply-add form (`rs3 | fmt | rs2 | rs1 |
  rm | rd | opcode`).

Because both the loads/stores and the immediate adds carry only a signed 12-bit
field, an out-of-range offset or immediate is materialized into a scratch register
first. A load or store whose displacement exceeds ±2047 stages the address through
`t0` (or, for a load whose destination differs from its base, through the
destination register itself, avoiding a clobber of a live lane in a scalarized
vector sequence). [[src/arch/riscv64/encode/emitter.rs:emit_load]]

### Immediate materialization

`mov_imm` (and every wide-immediate fallback) builds a 64-bit constant through the
standard `li` expansion: a `lui`+`addi` for a 32-bit-representable value, else a
recursive `lui/addi/slli 12/addi` chain that reconstructs the exact bit pattern
(at most ~8 words). The expansion is a single shared routine used by both the
emitter (to produce the words) and the sizing pass (to count them), so the two
passes always agree. [[src/arch/riscv64/encode/sizing.rs:li_steps]]
[[src/arch/riscv64/encode/emitter.rs:emit_li]]

## MIR → RISC-V selection

Selection has two jobs, like the other backends: expand the flagless fused ops,
and remap the residual physical registers the neutral MIR still carries at ABI
boundaries. [[src/arch/riscv64/select.rs:select_riscv64]]

### Flagless control flow

RISC-V has no condition flags, so every compare-and-branch is self-contained.

- **Integer compare-and-branch** (`br_cc`/`br_cc_imm`) lowers to one native
  compare-and-branch, emitted as `rv.br` — the 8-byte inverted-short-branch-over-`jal`
  long form. Because RISC-V provides only `beq`/`bne`, signed `blt`/`bge`, and
  unsigned `bltu`/`bgeu`, the relations `>`/`<=`/`>`(unsigned)/`<=`(unsigned) are
  produced by **swapping the two operands** (e.g. `lhs > rhs` becomes `rhs < lhs`).
  A non-zero immediate rhs is materialized into `t0`; a zero rhs uses the hardware
  `zero` register directly. [[src/arch/riscv64/select.rs:int_branch]]
  [[src/arch/riscv64/select.rs:expand_fused]]
- **Float compare-and-branch** (`fbr_cc`/`fbr_cc_zero`) emits `feq.d`/`flt.d`/`fle.d`
  into a scratch GPR (ordered — NaN yields 0), then a branch that tests it. Each
  relation reproduces the exact ordered-only or unordered-including truth set the
  AArch64 float branch carries; the ordered-only relations branch on true, the
  unordered-including ones branch on the ordered complement being false, and the
  finiteness checks compare each operand with itself to detect a NaN. A compare
  against zero first materializes `+0.0` into an FP scratch via `fmv.d.x zero`.
  [[src/arch/riscv64/select.rs:float_branch]]
- **Overflow-checked arithmetic** (`add_ovf`/`sub_ovf`) computes the sum/difference
  and detects signed overflow with a sign-comparison sequence (`eor`/`mvn`/`and`),
  then branches on the detection word's sign bit — writing the destination
  **before** the branch, since the branch jumps away on the no-overflow path.
  [[src/arch/riscv64/select.rs:expand_fused]]
- A bare (non-fused) integer compare whose flag-reading branch is not adjacent
  (fusion missed it) is handled specially: the compare snapshots its left operand
  into `gp` — a register the codegen never otherwise uses and which survives across
  calls — and keeps the right operand by name; the later standalone branch
  re-derives a native `rv.br` from that saved state. The saved compare is
  invalidated by an intervening label or a redefinition of the right-operand
  register. [[src/arch/riscv64/select.rs:select_riscv64]]
- **`addr_of`** expands to the `auipc; addi` pair (the `adrp`+`add_pageoff` ops the
  encoder realizes PC-relative), so a re-lowering pass re-fuses it and the
  expansion is a fixed point. [[src/arch/riscv64/select.rs:select_riscv64]]

### ABI register remapping

The neutral MIR still carries AArch64 physical names at ABI boundaries and in the
hand-written helpers. Unlike x86-64 — where arguments, returns, and syscall
arguments occupy three disjoint register files, forcing a control-flow role
analysis — RISC-V reuses the `a0`–`a7` bank for arguments **and** results **and**
syscall arguments, exactly as AArch64 reuses `x0`–`x7`. So the remap is a simple
**positional** substitution, `xN → aN`, with a handful of fixed cases: `x8` → `a7`
(the syscall-number register), the link register token → `ra`, the zero token →
`zero`, `sp` → `sp`, the caller/callee scratch `x9`–`x29` to a fixed home table,
and the AArch64 FP `dN` bank to the RISC-V FP ABI roles (`d0`–`d7` → `fa0`–`fa7`,
`d8`–`d15` → `fs0`–`fs7`). A role token is first realized to its AArch64 spelling
and then positionally remapped, keeping the mapping total.
[[src/arch/riscv64/select.rs:remap_riscv_abi]] [[src/arch/riscv64/select.rs:map_scratch_register]]

The calling convention is lp64d: integer arguments/returns in `a0`–`a7`, FP in
`fa0`–`fa7`, the syscall number in `a7` with `ecall` trapping into the kernel.
The return address is held in `ra` (a register), not pushed by the call, so a
16-aligned frame keeps `sp` 16-aligned at call sites with **no** extra padding —
like AArch64, unlike x86-64. The full ABI role and clobber tables are canonical in
`./mfb spec memory native-calling-convention`.
[[src/arch/riscv64/backend.rs:frame_call_padding]]

### No native SIMD — `v128` scalarization

RV64GC has no 128-bit register file, so the neutral `v128` vocabulary (used by the
transcendental math kernels and the `vector::` package) cannot live in a register.
Instead each `v128` op is **scalarized** onto a memory-slot region in the
per-thread arena state: every distinct vector value in a function is assigned a
16-byte slot (via linear-scan reuse, so the region size is the peak concurrent
count, not the value count), and each op materializes the slot base into `t2`,
loads its operands' two `f64`/`i64` lanes into the reserved scratch, computes the
two scalar results, and stores them back. The region is bounded to 128 slots so
the largest lane offset stays within the 12-bit load/store immediate. The slots
are **per-thread** (addressed off the pinned `s11`), so concurrent threads running
vector kernels no longer corrupt each other; they are non-reentrant within a
thread, which holds because the kernels are inlined straight-line leaf code.
Correctness is preserved and the native `D`-extension FMA keeps the ≤1-ULP kernel
contract. [[src/arch/riscv64/v128.rs:scalarize_v128]]
[[src/arch/riscv64/v128.rs:build_slot_map]] [[src/arch/riscv64/v128.rs:SLOT_COUNT]]

## Calls, labels, and relocations

The neutral layer carries a relocation *intent* — what a reference means — which
the RISC-V backend realizes into concrete kinds. Every RISC-V PC-relative
reference is a **pair** of instructions: a hi20 that materializes the upper 20 bits
with `auipc`, and a lo12 that adds or loads the low 12 bits. So a data address or
GOT load splits into `*Hi`/`*Lo` kinds (`riscv_pcrel_hi20`/`riscv_pcrel_lo12` for
in-image data, `riscv_got_hi20`/`riscv_got_lo12` for an imported symbol whose GOT
slot holds the resolved address), exactly as AArch64 splits into page and
page-offset relocations. A call is the single `auipc ra; jalr ra` pair the linker
patches as one unit (`riscv_call`), so it needs only one relocation recorded at
the `auipc`. [[src/arch/riscv64/reloc.rs:reloc_kind]]
[[src/arch/riscv64/encode/emitter.rs:emit_call]]

`label` instructions are zero-width markers whose byte offsets are recorded during
the second pass. Intra-function branches (`b`, `branch_self`, and the inner `jal`
of a compare-and-branch) are emitted as a placeholder `jal` word and patched once
the function's label offsets are known; the patcher preserves the destination-
register field and re-encodes the J immediate. A displacement beyond ±1 MiB is a
hard error rather than a silent truncation, and duplicate label names in one
function are rejected. [[src/arch/riscv64/encode/emitter.rs:patch_labels]]
[[src/arch/riscv64/encode/mod.rs:encode]]

The console build emits one executable per libc world — a glibc build and a musl
build — since the two share every kernel struct layout the codegen bakes in;
app (GTK) mode is not supported on this target. [[src/target/linux_riscv64/mod.rs:write_executable]]

## See Also

* ./mfb spec architecture aarch64-instruction-set — the sibling AArch64 op catalog
* ./mfb spec architecture x86_64-instruction-set — the sibling x86-64 backend
* ./mfb spec architecture mir-instruction-set — the shared neutral MIR this backend selects from
* ./mfb spec architecture native — the native code plan these instructions populate
* ./mfb spec architecture math-kernels — the ≤1-ULP kernels the base-`D` FMA backs
* ./mfb spec memory native-calling-convention — registers, ABI, and clobber sets
* ./mfb spec linker symbols-and-relocations — the relocation kinds that patch `auipc`/`jalr` pairs
* ./mfb spec linker linux-riscv64 — the riscv64 ELF backend that links this emitted code
