# MFBASIC Register Allocator Plan

Last updated: 2026-06-28

This plan replaces the native backend's bump-and-reset register model with a real
register allocator over a virtual-register layer: an integer class and a
floating-point class, liveness so loop-carried values stay resident across
iterations, and spilling so register pressure no longer fails compilation. The
single behavioral outcome a correct implementation produces: **identical program
output and error behavior to today, with hot float and integer loops keeping their
live values in registers — `Float` arithmetic running as `fadd d, d, d` on resident
FP registers with no per-statement stack spill and no FP↔GPR `fmov` round-trip —
and the standing "break nested expressions into LETs" limitation removed.**

This is the structural Phase 3 of `plan-01-float-codegen`, split out into its own
plan because — unlike that plan's Phases 1, 2, and 3a — it is not a self-contained
pass: it is a backend-wide architectural change. plan-01 Phase 3a (block-local
store-to-load forwarding) has already landed and is complementary; this plan
subsumes the rest of plan-01 Phase 3.

It complements:

- `./mfb spec architecture native` (`src/spec/architecture/06_native.md` — the
  value/stack-slot model this plan rewrites) and `…/13_native-ir.md`.
- `./mfb spec memory native-calling-convention` and `…/runtime-helper-abi`
  (`src/spec/memory/**` — the per-helper register clobber sets the allocator must
  honor; the build input for which registers survive a `bl`).
- `.ai/compiler.md` § "Native Codegen Register Lifetimes" (the canonical statement
  of the clobber hazard this allocator must encode as a constraint).

## 1. Goal

- Introduce a virtual-register abstraction so lowerings stop naming physical
  registers, and a coloring pass that assigns physical registers (or spill slots)
  after the whole function is lowered.
- Two register classes: **integer** (GPRs) and **floating-point** (`d`/`v`),
  so a `Float` value can live in a `d`-register across an expression and across a
  loop body instead of as a bit pattern in a GPR / stack slot.
- **Liveness-driven residency:** loop-carried locals (accumulators, induction-
  derived values) stay in callee-saved registers across iterations rather than
  being `ldr`/`str`'d on every use.
- **Spilling:** when pressure exceeds the file, spill to stack slots — removing the
  current hard failure ("allocator has no spilling; break nested exprs into LETs").
- **Honor the runtime-helper clobber model:** any value live across a `bl` to a
  runtime helper must be in a callee-saved register or spilled; the allocator must
  never color a value into a register the callee clobbers and assume it survives.
- **Be target-parametric, not AArch64-welded.** The allocator *algorithm* (liveness,
  interval construction, linear-scan, spilling) must be ISA-neutral and driven by a
  target register description, so the planned x86_64 backend reuses the core instead
  of forcing a second allocator. See §5 (Target portability).
- Byte-identical program output and identical error codes/trigger points; `.run`,
  `.ir`, `.ast` goldens unchanged (native-code goldens — `.ncode`/`.nobj`/`.hex` —
  are regenerated as the expected instruction-selection change).

### Non-goals (explicit constraints)

- **No language-surface, value/copy/move/freeze, layout, or ABI change.** A `Float`
  is still an 8-byte f64 in every stored location (escaping local, record field,
  collection element, thread-transfer buffer); the call ABI (argument/return
  registers, callee-saved set, stack alignment) is unchanged. Register residency is
  a function-internal concern: a value materializes to/from its existing 8-byte
  little-endian form at every memory boundary exactly as today.
- **No change to error behavior.** `ErrFloatInf`/`ErrFloatNan`/`ErrFloatDomain`/
  `ErrOverflow`, TRAP routing, scope-drop ordering, and the owned-value zero-init
  at entry must be preserved bit-for-bit in observable effect.
- **`Fixed` determinism untouched.** No change to the Q32.32 lowering or its
  cross-target bit-identity.
- **Do not weaken the clobber model.** The allocator encodes the
  `.ai/compiler.md` register-lifetime rules; it does not assume any helper's
  scratch list is a stable contract beyond what the callee's source proves.
- **No new instruction semantics.** This plan is instruction *selection/assignment*
  only (it may use already-supported instructions like FP moves and spills); FMA
  adoption and hardware-op changes are `plan-02`, not here.

## 2. Current State

The backend has **no virtual-register layer**. Lowerings emit physical registers
directly:

- `allocate_register` (`builder_codegen_primitives.rs:4`) returns
  `abi::temporary_register(self.next_register)` — a concrete name (`x8`, `x9`, …) —
  and bumps a counter. It **fails** when a single expression exceeds the temporary
  budget (the "break nested exprs into LETs" rule).
- `reset_temporary_registers` (`:24`) sets `next_register = 8` at the top of nearly
  every statement (`builder_numeric.rs:112,273,…`), so no temporary survives a
  statement boundary. Values that must persist are locals in stack slots
  (`allocate_stack_object`, `:50`), `ldr`/`str`'d on every use.
- `ValueResult.location` is a physical register name (`String`). There is no FP
  register class: a `Float` holds its f64 bit pattern in a GPR / stack slot, and
  every float op round-trips through `fmov` (`emit_float_binary`,
  `builder_numeric.rs:847`).
- Callee-saved tracking exists in embryo: `mark_register_used` /
  `used_callee_saved` (`builder_codegen_primitives.rs:16`) records callee-saved
  GPRs touched, and `finalize_frame` (`codegen_utils.rs:342`) emits the
  save/restore and shifts stack offsets. This is the hook the allocator extends to
  the FP class.

Hundreds of call sites across every `builder_*.rs` use `allocate_register` and
literal register names, so introducing virtuals is a broad but mechanical change.

### 2.1 What already landed (do not redo)

- **plan-01 Phase 1:** the finiteness check is a single shift+compare
  (`emit_float_result_check`, `builder_math.rs`), one scratch register.
- **plan-01 Phase 3a:** block-local store-to-load forwarding
  (`peephole::forward_stores_to_loads`, run before `finalize_frame`) turns
  defensive reloads into register moves (~17–21% on `float-leibniz`/`float-nbody`).
  This allocator makes that pass largely redundant for the spills it cannot remove
  at the source; keep it (it is cheap and still catches integer cases) until the
  allocator demonstrably subsumes it, then re-evaluate.

## 3. Design Overview

The risk is a silent miscompile, so the design is staged so that **each stage keeps
all 976 acceptance tests green and is independently revertible**, and the
behavior-changing coloring lands only after the plumbing is proven by an
identity transform.

- **Stage A — virtual-register layer + `BumpAndReset` strategy.** Add a
  virtual-register type; make `allocate_register` mint virtuals so lowerings never
  name a physical register again. Replace the inline bump numbering with the **first
  `AllocationStrategy` implementation, `BumpAndReset`**, which reproduces today's
  exact physical assignment so output is **byte-identical** (the Stage A acceptance
  bar, and what proves the whole vreg-layer plumbing). This is not throwaway
  scaffolding: it validates the trait interface before `LinearScan` exists, moves
  *all* register assignment out of the lowerings (so Stage B is a drop-in "add
  `LinearScan`, flip the default" with zero lowering churn), and leaves a permanent
  `-regalloc bump` reference/oracle for differential debugging.
  - **Key implication:** the points where `reset_temporary_registers` is called
    today become **explicit region-boundary markers** in the instruction stream.
    `BumpAndReset` consumes them to replay the `x8`-restart numbering; `LinearScan`
    (Stage B) ignores them and allocates by liveness. Locals remain explicit stack
    slots (`allocate_stack_object`) — not a register-model decision — so they are
    unchanged, which is part of how Stage A stays byte-identical.
- **Stage B — liveness + linear-scan (integer class).** Compute live ranges over the
  lowered stream and color with a linear-scan allocator for the integer class only,
  reusing registers by liveness instead of bump-and-reset. Spill on pressure. FP
  values still travel as today. This is where loop-carried integer values become
  resident and the "break into LETs" failure disappears.
- **Stage C — floating-point register class.** Give `Float` values an FP virtual
  class colored to `d0–d7`/`d8–d15`; rewrite `emit_float_binary` and the float
  loads/stores to operate on FP registers, materializing to memory only at
  boundaries. This removes the `fmov` round-trips and the per-statement float
  spills — the bulk of the float-loop win.
- **Stage D — loop-carried residency + callee-saved FP.** Range-splitting / loop
  awareness so accumulators stay in callee-saved `x19–x28` / `d8–d15` across
  iterations; extend `finalize_frame` to save/restore used callee-saved FP.

Correctness concentrates in the clobber model (§4.3) and spill correctness (§4.4).

## 4. Detailed Design

### 4.1 Virtual registers

A `VReg(u32, RegClass)` with `RegClass ∈ {Int, Fp}`. `allocate_register` returns an
`Int` vreg; a new `allocate_fp_register` returns an `Fp` vreg. Physical registers
that the ABI pins (argument/return registers `x0..x7`/`d0..d7`, `sp`, `x19` frame
base, the runtime scratch `x16`/`x17`) are represented as **pre-colored** vregs so
the allocator respects fixed assignments at call boundaries and helper interfaces.
Instruction fields carry vregs; a final rewrite substitutes physical names.

### 4.2 Liveness and the pluggable allocation strategy

Liveness is **strategy-independent shared infrastructure**: build basic blocks from
the instruction stream (split at labels/branches), model loop back-edges (so a value
live across the back-edge is live for the whole loop, keeping accumulators
resident), and compute live-in/live-out by backward dataflow. This runs once,
regardless of which allocation method is selected.

The **allocation method itself is a swappable module** behind an
`AllocationStrategy` trait, so linear-scan ships first and graph-coloring (or any
other method) can be added later without touching the rest of the backend:

```text
trait AllocationStrategy {
    // Consumes: the function's vregs, CFG + liveness, and the per-ISA
    // RegisterModel (§5). Produces: a vreg -> (physical | spill-slot) assignment
    // plus the spill/reload/move insertions, honoring class constraints, pinned
    // and tied operands, and clobber sets.
    fn allocate(&self, func: &FnBody, live: &Liveness, regs: &dyn RegisterModel)
        -> Allocation;
}
```

- `BumpAndReset` (Stage A, the reference impl): replays the legacy per-statement
  bump numbering from region-boundary markers, ignoring liveness — byte-identical to
  the pre-allocator backend. Kept permanently as the `-regalloc bump` oracle.
- `LinearScan` (Stage B, the V1 *real* allocator, becomes the default): one scan per
  register class over live intervals; spill the interval with the furthest next-use
  on pressure (§4.4).
- `GraphColoring` (future): builds an interference graph from the same `Liveness`
  and colors it; slots in as a second `AllocationStrategy` impl with no change to
  liveness, the `RegisterModel`, or the rewrite pass.

Both strategies consume the *same* shared liveness and emit the *same* `Allocation`
shape, so the surrounding pipeline is method-agnostic.

**CLI selection.** A `-regalloc <name>` build flag (value-taking, mirroring
`-target`; parsed in `src/cli/build.rs` alongside the other flags and threaded
through `BuildOptions`) selects the strategy, defaulting to `linear-scan`. Unknown
names are a build error listing the available strategies. This makes alternative
allocators A/B-testable on the `benchmark/` suite without a rebuild-per-method and
keeps the default stable.

### 4.3 Clobber model (correctness-critical)

Every `bl`/`blr` to a runtime helper or MFBASIC function is a clobber point. The
allocator must treat all caller-saved registers (and, per `.ai/compiler.md`, the
specific helper scratch sets — e.g. `_mfb_arena_alloc` destroying
`x9/x10/x14/x15/x16/x20–x28`) as dead across the call: a value live across it must
be in a callee-saved register or spilled. This is encoded as clobber operands on the
call instruction so liveness/coloring see it. The existing manual spills around
helper calls (the "store before bl, reload after" pattern) become redundant once the
allocator enforces this and can be removed lowering-by-lowering — but only after the
allocator provably honors the clobber set (a differential `.run` golden check per
removal).

### 4.4 Spilling

On pressure, spill the interval with the furthest next-use to a stack slot
(`allocate_stack_object`), inserting reload before each use and store after each def.
Spill of an `Fp` vreg uses the existing 8-byte slot (`str`/`ldr` of the FP register).
This is what removes the "break nested exprs into LETs" failure; add a regression
test that previously required a manual LET split and now compiles directly.

### 4.5 finalize_frame interaction

`finalize_frame` already saves/restores used callee-saved GPRs and shifts stack
offsets. Extend it to save/restore used callee-saved FP registers (`d8–d15`, lower
64 bits per the AArch64 PCS) via the same `used_callee_saved` mechanism generalized
to both classes. Spill-slot allocation must occur before offset finalization, as
today.

### 4.6 SIMD / vector register interaction (critical)

The `math::` transcendental kernels are NEON code that runs **inlined into the
calling user function** (`lower_simd_float_binary_scalar`, `emit_pow_scalar`, …,
each calling `reset_temporary_registers`). On AArch64 the scalar FP registers and
the NEON vector registers are **one physical file** — `d_n` is the low 64 bits of
`v_n` — so the allocator's FP class and the kernels' vector usage share registers
and the allocator must model this, not treat "scalar `d`" and "vector `v`" as
independent.

Two facts make this tractable, and one is load-bearing:

- **The kernels already avoid `v8–v15`.** Audited register use across
  `builder_simd_float_math.rs` / `builder_pow.rs` / `builder_simd_math.rs` is
  exactly `v0–v7` and `v16–v31` — never `v8–v15`. That is the deliberate
  caller-saved/callee-saved split: `v8–v15`'s low 64 bits are callee-saved by the
  PCS, so the kernels stay off them. **Consequence:** a loop-carried `Float` the
  allocator parks in `d8–d15` *survives* an inlined kernel call untouched. The
  Stage D plan to hold accumulators in `d8–d15` is therefore safe across
  `math::sin`/`exp`/`pow`/… in a loop body — the common and important case.
- **Treat each inlined kernel (and every vector op) as a clobber region over the
  caller-saved FP/vector set** `{v0–v7, v16–v31}` (= `{d0–d7, d16–d31}`). The
  allocator must spill any live scalar `Float` held in those registers across the
  kernel, exactly as it spills caller-saved GPRs across a `bl`. With `d8–d15`
  reserved for live-across-call values and `d0–d7` for short-lived temporaries, this
  falls out of the normal clobber handling (§4.3) once FP clobbers are modeled.
- **Do not virtualize the kernels.** They are hand-tuned, ULP-validated fixed-`v`
  sequences (`plan-01-libm-kernels`); rewriting them to virtual vector registers
  would risk their accuracy and is out of scope. They keep their fixed registers;
  the allocator only needs their clobber set, not control of their internals.

So the allocator has **one FP/SIMD register class** with sub-register aliasing
(`d_n ⊂ v_n`), an allocatable scratch range of `d0–d7`, a callee-saved range of
`d8–d15`, and `d16–d31` modeled as caller-saved and kernel-clobbered. Add a
clobber-safety test: a loop that keeps an accumulator live across a `math::` call
and verifies the result past the small-input threshold.

## 5. Target Portability (future x86_64)

Today the entire native backend is AArch64-only: `src/arch/` contains just
`aarch64`, the shared codegen hardcodes `crate::arch::aarch64::{abi, ops}`, and the
sole abstraction (`CodegenPlatform`) is OS-level (macOS vs Linux on AArch64), not
ISA-level. A future x86_64 backend is a much larger effort than the allocator (it
needs its own instruction emitter, encoder, and ABI). This plan does **not** build
that backend — but it must not produce an allocator that x86_64 would force a
rewrite of. So the allocator is split into two layers:

- **ISA-neutral allocator core** (in shared code): operates on virtual registers,
  basic blocks, live intervals, and an abstract instruction view exposing, per
  instruction, its operands' *roles* (def / use), each operand's *register-class
  constraint*, any *pinned* physical register, any *tied* (two-address /
  read-modify-write) operand, and the instruction's *clobber set*. Liveness,
  linear-scan, spill-slot assignment, and move/reload insertion live here and know
  nothing about specific register names. **The core queries the active ISA module
  (`crate::arch::<isa>`) for every register fact** — it hardcodes none.

- **Per-ISA register model, resident in `src/arch/<isa>/`.** Each ISA module owns
  and answers the register questions; the allocator asks. This is where the
  already-scattered AArch64 register facts get formalized:
  `abi::argument_register`, `abi::return_register`, `abi::temporary_register`,
  `abi::stack_pointer`, and `abi::is_callee_saved` (`src/arch/aarch64/abi.rs`) are
  the seed — today they encode the bump-allocator's fixed numbering; they become the
  data behind a single queried `RegisterModel` interface. `src/arch/mod.rs`
  currently exports only `aarch64`; a future `src/arch/x86_64/` is added as a
  sibling implementing the same interface, and the allocator is unchanged. The model
  answers:
  - **register classes and their physical banks / counts** — which registers exist,
    and which class each belongs to (Integer vs FP/SIMD);
  - **the FP/SIMD class and its sub-register aliasing** — that `d_n ⊂ v_n` on
    AArch64 (one file; §4.6), and the analogous XMM/YMM relationship on x86_64;
  - **caller-saved vs callee-saved partition, per class** — `x19–x28`/`d8–d15`
    callee-saved on AArch64; empty FP callee-saved on x86_64-SysV (§5 below);
  - **ABI-pinned registers** — argument, return, stack pointer, frame base, and the
    reserved scratch (`x16`/`x17` on AArch64);
  - **per-runtime-helper clobber sets** — e.g. `_mfb_arena_alloc`'s scratch list,
    and the inlined-`math::`-kernel clobber set `{v0–v7, v16–v31}` (§4.6);
  - **the flags resource** (NZCV / EFLAGS) as a non-allocatable pinned register;
  - **emitters for spill, reload, and register-to-register move** in each class
    (`str`/`ldr`/`fmov` on AArch64).

This split is chosen up front because x86_64 imposes constraints the *abstract
instruction view* must already be able to express — and which AArch64 also needs, so
they cost nothing extra now:

- **Tied / two-address operands** — x86 `addsd xmm0, xmm1` writes `xmm0` (dst == one
  src). The operand model must allow a def tied to a use.
- **Fixed-register instructions** — x86 `idiv` pins `rax`/`rdx`, variable shifts pin
  `cl`. Modeled as pinned-operand + clobber constraints (AArch64 uses the same
  mechanism for `x0`/`x1` at calls and `x16`/`x17` scratch).
- **Smaller integer file and a distinct FP/vector class** — 16 GPRs, XMM/YMM as the
  FP class. The class/bank table is data, so the same linear-scan handles both
  register-file sizes; only spill frequency differs.
- **Flags register** — x86 `EFLAGS` (and AArch64 NZCV) as a fixed single-register
  resource the allocator must not clobber across a flag-dependent branch; modeled as
  a pinned clobber, not a generally-allocatable register.
- **No callee-saved FP on x86_64-SysV.** AArch64 has `d8–d15` callee-saved (§4.6),
  which is *why* loop-carried `Float` values can stay resident across calls and
  inlined kernels there. The System V x86_64 ABI has **no** callee-saved XMM
  registers (Windows x64 preserves `xmm6–xmm15`). So the "callee-saved FP set" is
  target data: `d8–d15` on AArch64, **empty** on x86_64-SysV (loop-carried floats
  must spill across any call there), `xmm6–15` on Win64. The allocator core handles
  whatever the register description declares; this is the sharpest reason the
  callee-saved set must be per-target data, not baked in. The x86_64 transcendental
  kernels will likewise need a documented vector-scratch convention analogous to the
  AArch64 "avoid `v8–v15`" rule (§4.6), or be treated wholly as clobber regions.

What this plan delivers now: the AArch64 implementation behind that two-layer split.
What it explicitly leaves for the x86_64 effort: a second target register description
and the x86_64 instruction emitter/encoder. The acceptance bar for "target-parametric"
this plan can verify is structural — the core compiles and runs with the register
description passed in as an abstraction, not as hardcoded `x`/`d` names — since there
is no second ISA to differential-test against yet.

## Layout / ABI Impact

None to value representation or the call ABI. Internal change only: which physical
register (or spill slot) holds a value during a function body. Spec topics under
`src/spec/architecture/06_native.md` (and `13_native-ir.md`,
`mfb spec memory native-calling-convention`) that describe the bump-and-reset model
and the "break into LETs" limitation are rewritten to describe the allocator,
classes, callee-saved usage, and spilling (see Validation).

## Phases

1. **Stage A — virtual layer + `BumpAndReset` strategy** (§4.1, §4.2). Acceptance:
   byte-identical to pre-change (all goldens, including `.ncode`, unchanged), with
   `BumpAndReset` as the default `-regalloc` and the only strategy. Lowest risk;
   proves the vreg plumbing and the `AllocationStrategy`/`RegisterModel` interfaces.
2. **Stage B — liveness + integer linear-scan + spilling** (§4.2, §4.4). Acceptance:
   `.run`/`.ir`/`.ast` unchanged; `.ncode` regenerated; the previously-failing
   register-pressure program compiles; integer-heavy benchmarks improve.
3. **Stage C — FP register class** (§4.1 Fp, rewrite `emit_float_binary`).
   Acceptance: float benchmarks show `fadd d,d,d` with no per-statement spill /
   `fmov` round-trip; `.run` unchanged; float-loop medians drop toward `cc -O0`.
4. **Stage D — loop-carried residency + callee-saved FP** (§4.2 loops, §4.5).
   Acceptance: leibniz/nbody accumulators resident across iterations (disasm);
   medians improve further; callee-saved FP save/restore correct.

Each stage gates on the full (unfiltered) `scripts/test-accept.sh` plus a
byte-identical `.run` diff (output cannot change), and Stage C/D additionally on the
float benchmark medians and a disassembly check. During development, narrow the run
with name globs (`test-accept.sh … '<glob>' …`) for a fast inner loop, but the gate
is always the full suite — see the Stage D part 2 note for the recommended buckets.

## Validation Plan

- **Function tests:** existing suites stay green unchanged. Add a register-pressure
  valid test (a deep nested expression that previously needed manual LET splitting)
  proving it compiles and runs; this is the observable proof spilling works.
- **Runtime proof:** the `benchmark/` suite — capture float-leibniz, float-nbody,
  math-trig, math-explog medians per stage; the bar is Stage C/D bringing
  float-leibniz compute to a small multiple of `cc -O0`.
- **Output identity:** byte-identical `.run` goldens at every stage (assert with an
  explicit diff); `.ncode`/`.nobj`/`.hex` regenerated as expected instruction
  changes, each reviewed to be assignment-only (no semantic drift).
- **Clobber-safety proof:** a test that holds many values live across a
  `bl _mfb_arena_alloc` (the documented hazard) and verifies correct results at a
  size past the small-input threshold the guide warns about.
- **Doc sync (mandatory, each stage):** rewrite the value/register-model description
  in `src/spec/architecture/06_native.md` (and `13_native-ir.md` /
  `mfb spec memory native-calling-convention`) to the allocator + classes +
  callee-saved + spilling, and **delete the "break nested exprs into LETs" rule**
  from the spec and any compiler note in the stage that removes it. Document the
  per-ISA `RegisterModel` location (`src/arch/<isa>/`) and the pluggable
  `AllocationStrategy` in the native/extending topics, and document the
  `-regalloc <name>` build flag in the commands spec
  (`src/spec/architecture/01_commands.md`) when the flag lands.
- **Acceptance:** `scripts/test-accept.sh target/debug/mfb target/accept-actual`
  green after every stage. The unfiltered run is the gate; `test-accept.sh` also
  accepts trailing name globs (e.g. `'arithmetic-*' 'func_math_*'`) to run a single
  bucket for fast iteration, but a filtered pass never substitutes for the full run.

## Open Decisions

- **Allocator algorithm — SETTLED: pluggable strategy, linear-scan default.** The
  allocation method is a swappable `AllocationStrategy` module (§4.2) selected by
  `-regalloc <name>` (default `linear-scan`); graph-coloring or others can be added
  later as additional impls without touching liveness, the `RegisterModel`, or the
  rewrite pass. Linear-scan is the V1 implementation — simple, fits the linear
  instruction stream, fast compile times, and sufficient to keep loop-carried values
  resident.
- **Keep or retire the Phase 3a peephole — SETTLED: keep until Stage C/D.** The
  store-to-load forwarding pass stays in place until Stage C/D demonstrably removes
  the spills it forwards, then re-evaluate; it is cheap and still helps integer paths
  and any residual spill in the meantime.
- **Pre-colored vs. constrained virtuals for ABI/scratch registers — SETTLED:
  pre-colored.** ABI/scratch registers (`x0–x7`/`d0–d7`/`x16`/`x17`/`sp`/`x19` on
  AArch64, and their per-ISA equivalents from the `RegisterModel`) are represented as
  pre-colored vregs with move insertion at boundaries — the allocator never
  special-cases physical names. This also keeps the core ISA-neutral: the set of
  pre-colored registers comes from the `RegisterModel`, not from hardcoded names.
- **Where the register model lives — SETTLED:** in `src/arch/<isa>/` (alongside the
  existing `abi::*` register primitives it formalizes), queried by the shared
  allocator core via `crate::arch::<isa>`. AArch64 implements it now; x86_64 is a
  sibling module later. The remaining sub-decision is interface *shape* — recommend
  a `RegisterModel` trait the ISA module implements over a generic-over-ISA
  allocator (less type machinery); revisit only if x86_64 shows the interface is too
  narrow (e.g. needs richer operand-tie or sub-register modeling).

## Non-Goals

- FMA / hardware-instruction adoption (that is `plan-02`).
- `Fixed` lowering changes.
- Auto-vectorization or SIMD-widening of scalar code.
- Cross-function (interprocedural) allocation or changing the call ABI.
- **Building the x86_64 backend.** This plan makes the allocator *core*
  target-parametric (§5) and supplies the AArch64 register description; the x86_64
  instruction emitter/encoder and its register description are a separate effort. A
  broader ISA-abstraction of the *instruction emitters* (the 8 shared-codegen files
  that hardcode `arch::aarch64`) is likewise out of scope here — only the allocator
  is abstracted now.

## Implementation Progress

**Stage A — DONE (commit 72748022), full acceptance suite byte-identical.**
- `src/arch/aarch64/regmodel.rs`: the `RegisterModel` trait + `Aarch64RegisterModel`
  (allocatable/caller-saved banks per class, FP class with `d⊂v`, callee-saved
  partition incl. `d8–d15`, spill/reload/move emitters).
- `src/target/shared/code/regalloc/`: the ISA-neutral core — the `%vN` virtual
  register sentinel carried in instruction register fields, the rewrite pass, the
  `AllocationStrategy` trait, and the `BumpAndReset` reference strategy (unit-tested).
- `allocate_register` mints a virtual register while computing the bump physical
  eagerly (drives the byte-identical replay via `vreg_eager`, preserves callee-saved
  marking order). `run_register_allocation` colors the stream before the peephole
  pass + `finalize_frame` at all three `CodeBuilder` finalize sites.
- `-regalloc <name>` build flag (default `bump`), threaded through `BuildOptions`.
- Native-codegen / commands spec topics updated.
- One latent hazard fixed: `emit_callable_branch` distinguished a register-held
  callable from a function symbol with `location.starts_with('x')`; a `%vN` failed
  the test and leaked into a relocation. It now also accepts a virtual register.

**Stage B — DONE (commit bbeff85e). `linear-scan` is now the default.**
- `regalloc/analysis.rs`: ISA-neutral CFG + liveness. Physical-register liveness is
  a single-word (31-register) backward dataflow; virtual-register liveness is sparse
  interned-id dataflow producing per-vreg `[min,max]` intervals (textual spans are
  unsound — a temp held across a loop back-edge is live for the whole loop). Scales
  to the 488K-instruction / 41K-vreg generated regex function.
- `regalloc/linear_scan.rs`: interval linear-scan, binary-search physical
  interference, spill on pressure or when a value's interval crosses a call,
  reload/store insertion with a per-instruction free-scratch pick.
- Validated by a bump-vs-linear-scan runtime differential over the whole runnable
  corpus plus the full acceptance suite. Spilling proof:
  `tests/codegen-register-pressure-valid` (50 spill slots, correct result).
- The allocator **found and fixed a latent bump miscompile**: `FALSE XOR f(...)`
  clobbered the XOR left operand across the call; linear-scan spills it and computes
  the spec-correct result (XOR always evaluates both sides). The `logic-valid`
  golden was corrected; `-regalloc bump` still reproduces the legacy bug as the
  byte-identical oracle.

**Stage C — DONE (commit 4cdaa61d).** FP register class: `analysis`/`linear_scan`
parameterized by a `ClassModel`, run once per class (the `x` and `d` files never
interfere); FP classifier matches `d_n` and `v_n` (kernel `v5` marks `d5` busy,
§4.6). `allocate_fp_register` mints `%fN`; `str d`/`ldr d` spill ops
(reference-verified). `emit_float_binary` runs `+`/`-`/`*`/`/` on FP vregs and
records the result GPR as also resident in a `d`-register; `operand_as_double` +
`lower_arithmetic_binary` carry that residency across the operand slot round-trip
so chained float arithmetic stays in `d`-registers (residency recorded only under
linear-scan — the bump oracle reuses `d0`–`d7` per statement). Differential
535/535, suite green, ~20% fewer FP moves.

**Stage D part 1 — DONE (callee-saved FP).** FP pool extended to `d8`–`d15`;
`RegisterModel::callee_saved_survives_calls` (Fp true, Int false); linear-scan
keeps an FP value live across a call in a callee-saved `d8`–`d15` instead of
spilling (sound: helpers touch no FP, libc is PCS-compliant, kernels avoid
`v8`–`v15`); `finalize_frame` saves/restores callee-saved `d`-registers with
`str d`/`ldr d`. Verified: `(a+b)*call()` keeps `a+b` in `d8` across the call,
frame `calleeSaved` includes `d8`.

**Stage D part 2 — REMAINING (loop-carried local promotion).** The leibniz/nbody
wall-clock win needs a float accumulator *local* held in `d8`–`d15` across loop
iterations instead of round-tripping through its stack slot every iteration. This
is register-promotion of a loop-carried, non-escaping float local: pre-load it
into an FP vreg at loop entry, route the loop body's reads/writes through that
vreg, store back at loop exit. It is a separable optimization pass over the
slot-based local handling (`builder_control.rs` loops + local access + escape
analysis) — the allocator infrastructure (FP class + callee-saved `d8`–`d15`) is
now in place to color such a vreg. Not a correctness gap (no behavior change);
purely the performance optimization.

This pass is high silent-miscompile risk in loop bodies (conditional-assignment
merges, exit/escape store-back), so it must land verified. To keep the edit→test
cycle fast, iterate with **filtered** acceptance runs while developing — pass one
or more name globs to `test-accept.sh` to run only the buckets a promotion pass can
plausibly break (loop/codegen/arithmetic/collection), e.g.

```
scripts/test-accept.sh target/debug/mfb target/accept-actual \
  'codegen-*' 'arithmetic-*' 'collection-*' 'control-flow-*' 'func_math_*'
```

This drops the inner loop from ~30–40 min to seconds. **It is an iteration aid, not
the gate:** a silent miscompile can hide in a bucket the glob omitted, so the
completion gate below still requires one full **unfiltered** `test-accept.sh` run
(plus the runtime/disasm proofs) before this stage is done.

## Old notes (superseded)

**Stage C/D — remaining. Sharpened blocker for the next session.** The plan's
"Current State" undercounts the migration: the lowerings hardcode ~9000 physical
register names (`x9`/`x10`/… as within-statement scratch), not only the ~551
`allocate_register` sites. A *correct* linear-scan therefore cannot color a virtual
register without modelling the **liveness of those hardcoded physicals** (and the
helper clobber sets) — the conservative "forbid a physical that is *textually
touched* inside a vreg's range" shortcut is **unsound**, because a callee-saved
physical can be live *across* a vreg's range with no textual mention inside it
(`mov x20,…; <vreg range>; use x20`). So Stage B requires real CFG + backward-
dataflow liveness over both virtual and physical registers before any coloring.

Design notes for the Stage B allocator (analysis already done):
- *Effect model can be generic*, not per-op: a field named `dst` is a pure def
  (AArch64 is three-address; no tied operands); `src`/`lhs`/`rhs`/`minuend`/
  `base`/`register`/`addend` are uses; classify each operand value by name
  (`x*`/`%vN` → Int, `d*`/`v*` → Fp-ignore-in-B, `sp`/immediates/symbols → not a
  managed register). Only `BranchLink`/`BranchLinkRegister`/`Svc` clobber.
- *Soundness rule for calls*: treat any virtual register whose live range crosses
  a call as must-spill (no register is safe across an internal helper — e.g.
  `_mfb_arena_alloc` clobbers callee-saved `x20–x28`, see `.ai/compiler.md`).
  Loop-carried residency across calls (`d8–d15` survival) is the Stage D refinement.
- *Useful invariant*: `allocate_register` temporaries never live across a statement
  boundary (`reset_temporary_registers` forces cross-statement values to stack
  slots), so vreg live ranges are short and region-local.
- *Validation without mass golden churn*: keep `bump` the default (all
  `.ncode`/`.hex`/`.nobj` goldens unchanged), implement `LinearScan` behind
  `-regalloc linear-scan`, and **runtime-differential** it — build+run the whole
  test corpus with `-regalloc linear-scan` and diff stdout/exit against the `bump`
  build. Only after that differential is fully green should the default flip and
  the native-code goldens be regenerated (each reviewed assignment-only). Add the
  register-pressure regression test (a deep nested expression that overflows the
  temporary pool) once spilling lands — it is the observable proof.

## Summary

The engineering risk is a silent, layout-sensitive miscompile, so the plan is
staged to land a behavior-preserving virtual-register layer first (identity
coloring, byte-identical output), then liveness/linear-scan, then the FP class, then
loop residency — each gated on byte-identical `.run` goldens and the clobber-safety
proof the guide demands. Nothing changes program output, error codes, the owned/
un-linked kernels, the deterministic `Fixed` path, or any value's memory layout/ABI;
the allocator only stops the backend from spilling every value to the stack between
statements and round-tripping floats through integer registers. The interim Phase 3a
forwarding pass already banked ~17–21% on the spill-heavy float benchmarks; the FP
class and loop residency (Stages C–D) are where the remaining multiple closes.
