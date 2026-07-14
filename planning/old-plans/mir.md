# MIR Design Work-Pad

Last updated: 2026-06-29

> **This is not a plan.** It is a scratchpad to figure out *everything we need to know*
> before we could write a real MIR plan and build x86_64 / rv64 backends. It records the
> problem, the constraints, every cross-ISA hazard found, options with leanings, and the
> open questions still to settle. Nothing here is committed design.

## 0. Why

`CodeInstruction.op` is `CodeOp` (`src/arch/aarch64/ops.rs`) — the **AArch64 instruction
set** (`SMulH`, `Rorv`, `Rbit`, `Adc`, `BranchVs`, …). The whole `src/target/shared/code/`
backend imports `crate::arch::aarch64::{abi, ops::CodeOp}` and emits AArch64 directly.
"Shared" means *shared across OS* (linux/macos), **not across ISA**. There is no
target-neutral machine IR between NIR (`src/ir/`) and the AArch64 stream. So a second ISA
is not a plug-in — the entire instruction-selection layer is welded to AArch64.

What *is* already ISA-neutral and survives a MIR: the parser/typechecker, NIR, the
register-allocator **algorithm** (liveness is field-based; `RegisterModel`/`ClassModel`
are traits), and the object/linker split (already per-OS). The MIR is the missing layer
that makes that scaffolding honest.

## 1. Scope & ground rules

- **Primary targets: aarch64, x86_64, rv64.** All 64-bit. The MIR is designed for these
  three; each must map cleanly (or with a *named, bounded* expansion).
- **Assumed ISA baselines (resolved — see §12.3):**
  - **aarch64:** ARMv8-A (NEON + scalar FMA are baseline; what we have today).
  - **x86_64:** **x86-64-v3** (Haswell, 2013) — **FMA3 required** (the ≤1 ULP kernels need
    single-rounding FMA), plus SSE4.1 (`roundpd`), BMI1 (`lzcnt`), AVX2. SIMD stays
    **128-bit**; AVX/AVX2 256-bit is a later optional widening, AVX-512 is not targeted.
  - **rv64:** **RVA20 = RV64GC** — the hard floor. `G` already includes D (f64) **and FMA**
    (`fmadd.d` etc.), so it covers accuracy with no extension. **`Zbb` is an optional
    feature flag** (`clz`/`ror`/`rev8`/`min`/`max` when present, base-G expansion when
    absent — never accuracy-critical). **RVV** (vector) is later/optional; `v128` is
    **scalarized** on rv64 by default (§6).
- **64-bit only.** The language is 64-bit. The MIR assumes 64-bit registers/pointers. A
  future **x86 (32-bit)** target is explicitly *not* a MIR constraint — it would fudge the
  64-bit semantics (register pairs, etc.) entirely inside its own MIR→x86 backend. We do
  not bend the MIR for it.
- **Goal: aarch64 output stays byte-identical** when it is re-routed *through* the MIR
  (the same "bump oracle" discipline plan-03 used). Adding the MIR must not regress the
  one working target.
- The MIR is **post-NIR, pre-encode**, over **virtual registers**; the existing allocator
  runs on it. Physical registers appear only at ABI boundaries.

## 2. Pipeline (target shape)

```
NIR ──lower──> MIR (virtual regs, target-neutral ops)
                 │
                 ├─ register allocation (shared algorithm + per-ISA RegisterModel)
                 │
                 └─ per-ISA instruction selection ──> machine ops ──> encode ──> object (per-OS)
```

`src/target/shared/code/` splits into: **(a)** the genuinely shared NIR→MIR lowering (most
of today's `builder_*.rs` *logic* — value shaping, control flow, the runtime-helper
calls — is ISA-independent; only the final `abi::`/`CodeOp` emission is not), and **(b)**
`src/arch/<isa>/` = MIR→machine selection + encoder + `RegisterModel` + ABI + relocations.

> **plan-00-A decision (implemented):** in Phase A the seam sits **just before
> register allocation**, at the single `CodeBuilder::run_register_allocation`
> chokepoint (`src/target/shared/code/mir.rs`, `builder_codegen_primitives.rs`).
> The lowered builder stream is raised to `MirOp` (`lower_to_mir`) and selected
> straight back (`select_aarch64`) before the **existing** allocator runs on the
> AArch64 stream — i.e. allocation stays *after* selection, so plan A is a pure
> structural insert with **zero allocator change** (allocating on the MIR is a
> later option). The round trip is the identity here (`MirOp` mirrors `CodeOp`
> 1:1), which is what makes `-codegen mir` byte-identical to `-codegen direct`.
> The hand-written runtime helpers (§9) do not pass through the builder, so they
> are not yet MIR; the `-mir` dump shows them as their 1:1 MIR over the final
> physical stream until plan-00-B ports them.

## 3. Value & type model

- MIR value types: **`i64`**, **`f64`**, and **`v128`** (SIMD; see §6). Pointers are `i64`.
  No sub-word *value* type — language `Byte`/`Bool` are `i64` in registers.
- **Memory access widths are explicit**: load/store `8/16/32/64` (zero/sign-extend on
  load). Needed for `Byte`, string/encoding bytes, the collection headers, record fields.
  (Today: `LdrU64/32/16/8`, `StrU64/32/8` — note there is no `StrU16` yet; MIR should have
  all four store widths.)
- Language → MIR: `Integer`/`Fixed` → `i64` (Fixed is Q32.32, ordinary i64 arithmetic);
  `Float` → `f64`; `Byte`/`Bool` → `i64` with 8-bit loads/stores; `String`/collections/
  records → `i64` pointers to flat blocks (layout unchanged, `mfb spec memory`).

## 4. Integer & scalar instruction set (the easy 80%)

These map 1:1 or near-1:1 on all three ISAs. MIR keeps them as semantic ops over vregs;
backends pick encodings and materialize immediates.

- **Move / const:** `mov`, `mov_imm <any i64>` (backend: aarch64 `movz/movk`, x86 `mov`,
  rv64 `lui/addi`/`li`). Immediates are abstract — never pre-encoded in the MIR.
- **ALU:** `add/sub/mul/and/or/xor/not`, `sdiv/udiv`, shifts `shl/shr/sar` (imm + variable),
  `neg`. All universal.
- **"Exotic" int ops — the ones that are *not* universal, flagged for expansion:**
  - `mulhi_s` / `mulhi_u` (64×64→high 64): aarch64 `smulh/umulh`, x86 `imul/mul` (128-bit
    result), rv64 `mulh/mulhu`. Universal but different shapes. *(Used by FNV hash, PCG64
    RNG, fdlibm.)*
  - `clz`: aarch64 `clz`, x86 `lzcnt` (BMI1) / `bsr`, rv64 `clz` (**Zbb** extension) — else
    a loop/expansion. **Decide a baseline ISA feature set (§9).**
  - `rbit` (reverse bits): aarch64 native; x86 **none** (multi-instruction); rv64 `brev8`+
    (Zbb/Zbkb) or expansion. → backend expands.
  - `rev`/`bswap` (byte reverse): aarch64 `rev`, x86 `bswap`, rv64 `rev8` (Zbb). 
  - `rotr` (rotate, var+imm): aarch64 `rorv`/`ror`, x86 `ror`, rv64 `ror` (**Zbb**) — else
    shift-or expansion.
  - `addc` (add with carry-in/out): aarch64 `adc` (carry flag), x86 `adc`, rv64 **no carry
    flag** → carry computed by comparison. *(Used by 128-bit PCG64 add.)* → §5 flagless.
  - `msub` / multiply-add: universal as a 2-op expansion if no fused form.
- **PC-relative address of a symbol** — `addr_of <sym>`: aarch64 `adrp`+`add :lo12:`,
  x86 RIP-relative `lea`, rv64 `auipc`+`addi`. One MIR op; each backend does its 2-instr
  (or 1-instr) sequence. *(Today this is `Adrp`+`AddPageOff` — pure AArch64; must become
  neutral.)*
- **Scalar float:** `fadd/fsub/fmul/fdiv/fsqrt/fabs/fneg/fmadd`, `fmov_i2f`/`fmov_f2i`
  (bit reinterpret), `i2f` (`scvtf`), `f2i_{trunc,floor,ceil,round,nearest}` (the
  `FCvtzs/ms/ps/as` family — distinct rounding modes; x86 `cvttsd2si`+`roundsd`, rv64
  `fcvt.l.d` with rm field). FMA needs **FMA3** on x86 and **F/Zfa** on rv64 — baseline
  decision (§9).

## 5. Control flow — **the single biggest portability problem**

Today control flow is **flag-based**: `Cmp`/`Adds`/`Subs`/`FCmpD` set NZCV, then
`BranchEq/Ne/Ge/Lt/Gt/Le/Hi/Lo/Ls/Mi/Vs/Vc` read it. **RV64 has no condition flags** — it
branches directly on a register comparison (`beq/bne/blt/bge/bltu/bgeu`) or materializes a
boolean (`slt/sltu`). x86 and aarch64 *do* have flags. A flag-based MIR is unbuildable on
rv64.

**Resolution (strong lean): the MIR has no flags. It exposes compare-and-branch and
compare-to-bool, both flagless:**
- `br_<cc> a, b, Ltrue [, Lfalse]` where `<cc>` ∈ {eq, ne, slt, sle, sgt, sge, ult, ule,
  ugt, uge}. Lowers to: aarch64 `cmp; b.cc`; x86 `cmp; jcc`; rv64 native `blt/bge/…`.
- `set_<cc> dst, a, b` (dst = 0/1). aarch64 `cmp; cset`; x86 `cmp; setcc`; rv64 `slt`+.
- **Float compare-and-branch** `fbr_<cc>` with IEEE conditions **including unordered**
  (lt, le, gt, ge, eq, ne, **uno/ord**). Must preserve the plan-17 IEEE semantics (NaN
  comparisons → false; the `b.mi`/`b.ls` conditions plan-17 added). aarch64 `fcmp; b.cc`;
  x86 `ucomisd; jcc` (CF/ZF/PF — the unordered handling differs, pin it with tests); rv64
  `feq/flt/fle` into a reg + branch.
- **Overflow**: today `BranchVs`/`BranchVc` read the V flag (integer overflow trap; the
  finiteness boundary `b.vs` for FP — plan-17). RV64 has no V. The MIR must make overflow
  **explicit**: an `add_ovf`/`sub_ovf`/`mul_ovf` that yields a value **and** a boolean
  overflow vreg (aarch64 `adds; cset vs`; x86 `add; seto`; rv64 compute from sign/compare).
  The FP-overflow path is already the `fabs/fcmp vs +Inf` form (plan-17, plan-16 Piece B) —
  that is flagless-friendly; keep it.

This decision ripples everywhere (every trap check, every loop condition, every IEEE
compare) — it is the load-bearing one. *Aarch64 re-routed through compare-and-branch must
still encode to the same `cmp; b.cc` it emits today (byte-identical goal).* 

> **plan-00-B decision (implemented):** consistent with plan-00-A's seam (the MIR is
> raised/lowered at `run_register_allocation`, builders unchanged), the flagless ops are
> produced by **fusion in `lower_to_mir` and expansion in `select_aarch64`** rather than by
> retargeting `builder_control.rs`. `lower_to_mir` fuses an adjacent flag-setter +
> flag-reading branch into one flagless op — `BrCc`/`BrCcImm` (`cmp`/`cmp_imm` + `b.cc`),
> `FBrCc`/`FBrCcZero` (`fcmp`/`fcmp_zero` + `b.cc`, carrying the plan-17 `b.mi`/`b.ls`/`b.vs`
> conditions verbatim), `AddOvf`/`SubOvf` (`adds`/`subs` + `b.vc`/`b.vs` overflow trap). The
> condition is a `cond` field; the operands are carried, so there is no hidden NZCV
> dependency. A 3-way `cmp; b.lo; b.hi` (string ordering) fuses to an owning op plus a
> `share`-marked op that reuses the comparison — `select_aarch64` emits the shared `cmp`
> once, byte-for-byte. `select_aarch64` expands each fused op back to the exact setter +
> branch, so `-codegen mir` stays byte-identical. The integer-overflow trap's "bool
> consumer" is the fused `*_ovf` op (no builder restructuring needed). **No `set_cc`/
> `fset_cc`**: this backend never materializes a comparison to a 0/1 register (no `cset`);
> every comparison feeds a branch, so compare-to-bool has nothing to neutralize.
>
> **Out of scope (stays flag-based, only in hand-written helpers — `mir.md §9`):** the
> `svc; b.lo` syscall carry check (needs the `syscall` neutralization, §7) and the
> `adds; adc` / `subs; sbc` 128-bit carry chains (the future `addc` op, §4). Both are
> left un-fused (a setter not followed by a flag-*branch*, or a branch not preceded by a
> fusable setter). After this plan the **builder/vreg path is fully flagless** for compare
> + overflow; the helper residue is the §9 helper-porting work.

## 6. SIMD — **the hardest open area**

The big tail of `CodeOp` is NEON: `LdrQ/StrQ`, `FAddV..FMaxV`, `FMlaV/FMlsV`,
`FRintp/m/a/n/zV`, `FCvtzs/asV`, `ScvtfV`, the `Cm*V` lane compares, `Add/Sub/Neg/AbsV`,
`Sshl/Ushl/Shl/Sshr/UshrV`, `And/Orr/Eor/Bsl/BitV`, `DupVFromX`, `UmovXFromV`. These power:
the transcendental kernels (`builder_simd_float_math.rs`, 2-lane f64), the `vector::`
package + array overloads, and (planned) register-native vectors (plan-01-vector).

The three ISAs diverge most here:
- **aarch64 NEON:** fixed 128-bit, rich (`bsl`/`bit` select, `frintm`, lane `umov/dup`).
- **x86_64:** SSE2 (128-bit, 2×f64) baseline; **FMA3** for `fmla`; **SSE4.1** for
  `roundpd` (frint*) and `blendv`/`pblendvb` (bsl/bit). 256-bit needs AVX. Horizontal ops
  and lane moves differ (`movmskpd`, `pshufd`, `extractps`).
- **rv64:** base ISA has **no SIMD**. **RVV** (vector ext) is *length-agnostic* — a totally
  different model (vsetvl, mask registers) — not a fixed-128 map. Many rv64 targets ship
  without RVV.

**Options (unsettled — this is the headline open question §9):**
1. **Fixed-width 128-bit `v128` MIR** (2×f64 / 4×f32 / 16×i8 lanes), semantic lane ops.
   Maps NEON↔SSE2+FMA3+SSE4.1 well; **rv64 scalarizes** `v128` (lower each op to 2× scalar
   f64) for correctness, with RVV as a *later* optional backend. *Lean: this — it keeps the
   hand-tuned kernels portable (rewrite once in `v128`-MIR), and rv64-without-RVV still
   works, just slower.*
2. **Scalarize SIMD in the MIR** (no `v128`), let backends re-vectorize. Loses the kernels'
   hand-tuned lane tricks (plan-03's branch-on-quadrant, BIT-selects) and the vector::
   SIMD; effectively throws away the SIMD work. *Reject.*
3. **Keep SIMD per-ISA** (kernels written N times). No portability for the biggest, most
   accuracy-sensitive code. *Reject.*

If (1): pin the lane semantics precisely (NaN behavior of `fminnm`↔`minpd`, the `bsl` vs
`blendv` mask polarity, rounding-mode ties) with byte/ULP tests — these are the silent-bug
mines. And the `v128` op list must cover exactly what the kernels + vector:: use, no more.

> **plan-00-E decision (implemented):** the 48-op NEON tail of `CodeOp` moved
> from the MIR `mirror` group into a new `simd` group with neutral `v128.*`
> mnemonics (`v128.fadd`, `v128.fma`, `v128.fround_even`, `v128.bsl`,
> `v128.dup_from_gpr`, …) — the round/convert names mirror plan-00-C's scalar
> `f2i_*`/`i2f`. Consistent with plan-00-A's seam, this is a **mnemonic-only**
> neutralization: each `v128` MirOp keeps its variant identity and maps 1:1 to
> its NEON `CodeOp`, so `select_aarch64` and the encoder are untouched and the
> output is byte-identical; only the `-mir` dump changes (no `*_v`/`*_q`
> mnemonic survives). The kernels and `vector::` need no rewrite — they reach
> the `v128` vocabulary through the builder→`run_register_allocation` seam.
> The FP `RegisterModel` class now formally spans the `d`/`v`/`q` views (the
> 128-bit `qN` view of the shared FP file). The lane-semantics contract (§6's
> silent-bug surface — `fmin`/`fmax` NaN propagation, `bsl`/`bit` mask polarity,
> round-mode ties, lane-mask compare patterns) is pinned as a 48-row test matrix
> (`v128_lane_semantics_contract`); the executable golden vectors are the
> unchanged ULP harness (`tools/math-kernels/runtime_ulp.py`) and the
> `func_vector_*` / `func_math_*array*` acceptance fixtures. Validation:
> codegen-selfdiff byte-identical, ULP harness unchanged (exp/log 100% ≤1 ULP),
> acceptance 975/975.

## 7. Calls, ABI, clobbers, the pinned arena register

- **Call** is abstract in the MIR: `call <sym|reg>, args=[vregs], rets=[vregs]`. The backend
  lowers to the per-ISA convention (aarch64 AAPCS x0–x7/v0–v7; x86_64 SysV rdi/rsi/…/xmm0–7;
  rv64 a0–a7/fa0–fa7) and applies the **clobber model** (today `call_clobber_mask` in
  `regalloc/analysis.rs` is AArch64 register numbers + the `_mfb_arena_alloc` special-case).
  Each ISA provides its caller/callee-saved sets + the runtime-helper clobber facts.
- **The pinned arena-state register** is AArch64-specific: `x19` (`ARENA_STATE_REGISTER`)
  holds the arena pointer program-wide, plus `x18` reserved, `x16/x17` IP scratch. This is
  a **per-ISA runtime-ABI decision**: rv64 (32 GPRs) can pin one happily; **x86_64 has only
  16 GPRs** and pinning one for the arena is expensive — it may instead keep the arena base
  in a fixed **TLS slot / memory global** and load it (or pin only under pressure). The MIR
  must not assume a pinned global register — model "arena base" as an *abstract source* the
  backend realizes (pinned reg vs TLS load).
  > **plan-00-D decision (implemented):** the neutral MIR names the operand
  > `arena_base` (`mir::ARENA_BASE`); the realization is a `RegisterModel` query
  > (`Aarch64RegisterModel::arena_base` → the pinned `x19`/`ARENA_STATE_REGISTER`,
  > reserved from allocation). Consistent with plan-00-A's seam, the rename is a
  > fold/expand: `lower_to_mir` renames the realization register to `arena_base`,
  > `select_aarch64` renames it back — identity for codegen, so the allocator
  > sees `x19` exactly as today. A field value equal to the pinned register is
  > unambiguously the arena base (it is reserved program-wide), which makes the
  > rename total and reversible. The `-mir` dump shows `arena_base`, never `x19`.
- **Syscall**: `syscall <nr>, args` — aarch64 `svc #0` (x8=nr), x86_64 `syscall`
  (rax=nr, rdi…), rv64 `ecall` (a7=nr, a0…). The register mapping is per-ISA; the MIR op
  is one.
- **Stack/frame**: MIR has stack slots (spill + locals, already present). Prologue/epilogue,
  frame size, callee-saved save/restore, red-zone — all **per-ISA** (the `finalize_frame`
  logic generalizes; the register lists/encodings do not).

## 8. Symbols & relocations

`CodeRelocation{from,to,kind,binding,library}` carries an **AArch64 `kind` string**
(`branch26`, page21/pageoff12, GOT). MIR should carry a **neutral relocation *intent***:
`{Call, DataAddr, GotLoad, …}` + symbol + binding (internal/import) + library. The backend
(per ISA **and** OS) maps intent → concrete reloc: aarch64 `R_AARCH64_CALL26`/`ADR_PREL`/
`GOT`; x86_64 `R_X86_64_PLT32`/`PC32`/`GOTPCREL`; rv64 `R_RISCV_CALL`/`PCREL_HI20/LO12`.
The object writers are already per-OS (mach-o/elf); they gain per-ISA reloc tables.

> **plan-00-D decision (implemented):** `CodeRelocation.kind` is now a neutral
> `RelocIntent` enum (`Call`, `DataAddrHi`/`DataAddrLo`, `GotLoadHi`/`GotLoadLo`)
> with `binding` kept alongside (it still splits a direct call from an
> import-stub call). The AArch64 intent→kind table lives in
> `src/arch/aarch64/reloc.rs` (`reloc_kind`): `Call→branch26`, both `*Hi→page21`,
> both `*Lo→pageoff12`. To stay byte-identical the concrete realization is
> *not* moved into the linker (it would be churn + risk on the only
> binary-producing path); instead the encoder (`emit_bl`/`emit_symbol_ref`)
> derives the intent from instruction context and runs it through the table, the
> `-ncode` serializer prints the realized kind (so `.ncode` still reads
> `branch26`/…, golden-identical), and `EncodedRelocation`/the per-OS linker are
> untouched. The neutral intent surfaces in the `-mir` dump's new `relocations`
> array (intent *name*: `call`/`data_addr_hi`/…, never an AArch64 kind). Routing
> the linker itself off `RelocIntent` is deferred to the first x86_64/rv64 plan,
> which is when a second per-ISA reloc table actually exists.

## 9. Runtime helpers — MIR vs per-ISA

Today the helpers (`lower_arena_alloc`, `lower_build_error_loc`, `lower_make_error_result`,
the PCG64 RNG fill, `fmod`, the math kernels, …) are **hand-written AArch64 `CodeFunction`s**.
**Decision: long-term, every helper body is MIR — zero hand-written per-ISA helpers.**

- **Port first (pure compute + memory):** arena alloc/free/insert, build_error_loc /
  make_error_result, the RNG fill, `fmod`, **and the math kernels** (the `v128`-MIR from §6
  makes the kernels portable). Straightforward MIR.
- **Port last (the "machine-y" ones — but still MIR):** the entry/`_start` shim,
  `_mfb_shutdown` + signal setup, the syscall stubs, the thread trampoline's register
  setup. These become MIR via the MIR's `syscall` op, the ABI-abstract `call`, and
  `arena_base` (§7) — none of them actually need hand asm once those ops exist.
- **Never a "helper" — it is the backend:** selection, encoder, RegisterModel, ABI/clobber
  tables, relocations, frame prologue/epilogue. That is the only per-ISA code.

This rewrite (hand-AArch64 helpers → MIR) is a real chunk of the cost and worth scoping
early: it is where "the backend is tiny per ISA" is won or lost. Staging is fine; the end
state is all-MIR helpers + a thin per-ISA backend.

> **plan-00-F decision (implemented).** Two parts, both AArch64-byte-identical.
>
> *(a) Vocabulary completion.* The last AArch64-named ops in the helper streams —
> `bl`/`blr`/`svc` — join the MIR `renamed` group as `call`/`call_indirect`/
> `syscall` (1:1 `CodeOp`, neutral mnemonic, byte-identical like plan-00-C/E).
> The macOS syscall error idiom `svc; b.<carry>` — the one flag-reading branch
> plan-00-B deferred to "the syscall neutralization" — fuses into a new flagless
> `syscall_br` op (the `svc` becomes a fusable setter; `select_aarch64` expands it
> back byte-for-byte). After this the helper MIR is **fully neutral and flagless**:
> a `-mir` dump of an entry+arena+RNG+thread+error program names no `svc`/`bl`/
> `blr`/`adrp`/`x19`/NEON/`*_v`/standalone `b.cc` — only neutral ops plus the
> universal `b`/`ret`/`branch_self`/`add_sp`/`sub_sp` and the deferred `adds;addc`
> carry chain (§4's explicit-carry concern, not a "helper" issue).
>
> *(b) Helpers through the seam.* The hand-written helpers don't pass through the
> builder's pre-allocation seam (`run_register_allocation`), so under `-codegen
> mir` they are routed through `lower_to_mir → select_aarch64` (the identity) at
> the plan-assembly chokepoint (`route_function_through_mir`). This brings them
> under the byte-identical gate — the self-diff now exercises the entry sequence,
> the arena allocator, the error path, the PCG64 RNG, the math kernels, and the
> thread trampoline through the MIR (validated: byte-identical except the
> pre-existing `bug-01` union-drop non-determinism; RNG + threads run correctly
> under `-codegen mir`).
>
> **Not done (deliberate, honors the byte-identical constraint).** The helper
> *bodies* still build their stream via the AArch64 `abi::*` emitters using fixed
> physical registers (`x9`/`x10`/`v22`…). They are not rewritten to emit
> `MirInstruction` against vregs, because that is the one thing that cannot stay
> byte-identical (the allocator would re-pin registers) — so true register
> portability belongs to the per-ISA backends (each writes its own register
> placement), exactly as §7 says "the ABI register placement is a backend
> detail." What plan-00-F delivers is the byte-identical half: every helper op is
> now a neutral MIR op, and every helper stream is proven MIR-representable.

## 10. Representation question (the MIR data type)

Today `CodeInstruction{op: CodeOp, fields: Vec<(&str, String)>}` — a typed op + **stringly**
fields. Two routes for the MIR:
- **Reuse the shape** (a neutral `MirOp` enum + the same string-field bag). Lowest churn;
  the allocator's field-based liveness keeps working unchanged; lets aarch64 re-route fast.
  *Lean: start here* — it is the cheapest path to "aarch64 byte-identical through MIR."
- **A typed MIR** (vregs as integers, typed operands, explicit basic blocks/CFG). Cleaner,
  but a bigger rewrite and re-tools the allocator's input. Defer; can evolve into it.
  The CFG already gets rebuilt in `regalloc/analysis.rs`; a typed MIR could carry it.

## 11. What's hard vs easy — summary

- **Easy (1:1):** moves, basic ALU, scalar float arith, loads/stores, direct branches/calls,
  stack slots. ~80% of ops.
- **Mechanical expansion (named, bounded):** mulhi shapes, clz/rbit/rev/rotr/addc on ISAs
  lacking the instruction, immediate materialization, PC-rel address sequences, f→i rounding
  modes.
- **Load-bearing redesign:** **(§5) flags → compare-and-branch** (touches every condition),
  **(§6) SIMD `v128` + per-ISA lane semantics** (touches every kernel + vector::), **(§7)
  the pinned arena register** (x86_64 register pressure), **(§9) helpers → MIR**.
- **Per-ISA, but well-scoped:** encoder, RegisterModel, ABI/clobbers, relocations, frame,
  entry/syscall/signal.

## 12. Open questions to settle before a real plan

1. **SIMD strategy (§6) — ✅ CONFIRMED.** Fixed-width **`v128`** MIR; NEON ↔ SSE2+FMA3+
   SSE4.1 (x86-64-v3); **rv64 scalarizes** `v128` (RVV later). Remaining sub-task (not a
   decision, an implementation gate): pin the exact `v128` op set the kernels + `vector::`
   use, and the lane-semantics test matrix (NaN of `fminnm`↔`minpd`, `bsl`-vs-`blendv` mask
   polarity, rounding-mode ties) — the silent-bug surface.
2. **Flagless model (§5) — ✅ RESOLVED: flagless is the long-term-best, no flags in the
   MIR.** The MIR never exposes a condition-flags register; conditions are
   `br_<cc> a,b,L` (compare-and-branch) and `set_<cc> dst,a,b` (compare-to-bool), and
   traps use explicit-result ops (`add_ovf`/`sub_ovf`/`mul_ovf` → value + overflow vreg;
   the FP-overflow path stays the plan-17 `fabs/fcmp vs +Inf` form). Rationale: modeling a
   flags register in an IR (flag liveness, cross-instruction clobbers, partial updates) is
   a well-known tar pit that LLVM/Cranelift/etc. deliberately avoid; flagless is the
   lowest common denominator that fits rv64 *and* maps to flag-ISAs with **no loss** —
   aarch64/x86 lower `br_cc` to the exact `cmp; b.cc`/`cmp; jcc` they emit today, so the
   byte-identical gate holds. Implementation sub-tasks (not decisions): prove the
   AArch64-byte-identical lowering, and pin x86 unordered-FP (`ucomisd` CF/ZF/PF) against a
   test vector.
3. **Baseline ISA feature set — ✅ RESOLVED (see §1).** x86_64 = **x86-64-v3** (Haswell,
   FMA3 required, SSE4.1/BMI1, 128-bit SIMD, no AVX-512). rv64 = **RVA20 / RV64GC** (base
   `G` already has f64 + FMA; **`Zbb` optional** feature-flag with base-G expansion; RVV
   later, `v128` scalarized by default). aarch64 = ARMv8-A (unchanged). Consequence: the
   only baseline-forced extension is **x86 FMA3** (accuracy); everything else
   (clz/rotr/rev/round) is native-when-present, expanded-when-absent.
4. **Pinned arena register (§7) — ✅ RESOLVED: abstract in the MIR; each ISA pins or TLS.**
   The MIR models the arena base as an **abstract source** (`arena_base` — a value the
   backend realizes), never a named global register. Each backend chooses: **pin a register**
   (aarch64 `x19`, rv64 — 32 GPRs, cheap) or **load from a fixed TLS/memory location**
   (x86_64 — 16 GPRs, pinning is too costly). MIR code that uses the arena just references
   `arena_base`; the realization is a per-ISA RegisterModel/ABI detail.
5. **Helpers → MIR scope (§9) — ✅ RESOLVED: long-term, *all* runtime helpers move to MIR.**
   The target is zero hand-written per-ISA helper bodies — arena, error builders, RNG,
   `fmod`, the math kernels, **and** the entry shim / shutdown+signal setup / thread
   trampoline, all expressed in MIR (the MIR's `syscall`, ABI-abstract `call`, and
   `arena_base` ops make even the "machine-y" ones portable). The only irreducibly per-ISA
   code is the **backend itself** (selection, encoder, RegisterModel, ABI/clobber tables,
   relocations, frame prologue/epilogue) — not "helpers." Staging is allowed (port the
   pure-compute helpers first; the entry/syscall glue last), but the end state is
   all-MIR.
6. **MIR representation (§10):** reuse the `op+string-fields` shape first (lean) vs typed MIR.
7. **Validation discipline — ✅ RESOLVED: a flag-gated dual path (the plan-03 pattern).**
   The MIR backend is added **behind a build flag** (e.g. `-codegen mir`, default
   `direct` = today's no-MIR AArch64 path), exactly as plan-03 shipped `-regalloc
   linear-scan` alongside the byte-identical `bump` oracle. Both paths coexist; the gate is
   that **`-codegen mir` produces byte-identical AArch64** to `-codegen direct` across the
   full acceptance suite (a self-diff oracle, no new ISA needed). Only once that holds is
   the MIR path made default and **the `direct` (no-MIR) path removed**. New ISAs
   (x86_64/rv64) are then validated by the full runtime suite + the ULP harness on a real
   or emulated (QEMU-user) target — still need a QEMU/CI story for those, but the *MIR
   rewrite itself* is de-risked entirely on AArch64 before any new ISA exists.

## 12a. Tooling: `-mir` dump (a required deliverable)

`mfb build -mir` — the **neutral counterpart to the existing `-ncode`**. Same ASM-as-JSON
serializer (`format: "mfb-mir"`), but it emits the **MIR**: neutral ops
(`addr_of`/`br_slt`/`add_ovf`/`call`/`arena_base`/`v128.*`), **virtual registers**
(`%v5`/`%f3`), and **no `target`/`arch`** (it is ISA-independent — diff it across targets
and it is identical; diff `-ncode` and it is not). It is produced *before* register
allocation and instruction selection.

This lands **with** the MIR layer (Phase 1 below) — it is how the MIR is inspected and how
the byte-identical gate is debugged. Reuse the `-ncode` JSON writer on the `MirOp` stream;
the two flags then sit side by side: `-mir` = what the backends consume, `-ncode` = what a
backend produced.

## 13. Rough sequencing (for the eventual plan, not now)

1. Define `MirOp` (reuse the instruction shape); lower NIR→MIR; aarch64 MIR→machine, **all
   behind `-codegen mir`** (default stays `direct`). Prove `-codegen mir` is
   **byte-identical** to `direct` across the full acceptance suite (the self-diff gate that
   de-risks everything).
2. Port the runtime helpers to MIR (still byte-identical under `-codegen mir`).
3. Settle + implement the `v128` SIMD layer (still byte-identical under `-codegen mir`).
4. **Flip the default to MIR and delete the `direct` (no-MIR) path** once 1–3 hold green.
   From here the AArch64 backend *is* MIR→machine; there is one path.
5. Add **x86_64** (x86-64-v3, SysV/linux first): selection + encoder + ABI + relocs +
   per-ISA shims + QEMU/CI. The flagless + addressing + immediate + FMA3 work pays off here.
6. Add **rv64** (RVA20 / RV64GC, Zbb-optional): selection + encoder + ABI + relocs + shims;
   `v128` scalarized (RVV later).

The whole bet: do steps 1–4 with **zero aarch64 output change** (a flag you can diff against
itself), so the entire risk of the rewrite is paid down on a target we already trust, before
any new ISA exists.
