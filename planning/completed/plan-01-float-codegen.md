# MFBASIC Float Codegen Performance Plan

Last updated: 2026-06-28

> **Status (2026-06-28).**
> - **Phase 1 (inf/NaN check) — DONE, committed.** The finiteness check is now a
>   single shift+compare (one scratch register). Verified behavior-preserving; ~0
>   measured speedup (the finite hot path was already cheap), but a real code-size /
>   register-pressure win. The provably-finite ops already skipped the check, so no
>   elision surface remained.
> - **Phase 2 (LICM) — DEFERRED into `plan-03` Stage D.** Empirically the only
>   per-iteration cost it removes (e.g. the `-1.0` rematerialization in
>   `float-leibniz`) is ~2 instructions/iteration, and hoisting is only worthwhile
>   when the hoisted value can stay *resident* in a register across the loop — which
>   requires the register allocator. Folded into plan-03's loop-carried-residency
>   stage rather than shipped as a standalone marginal-win pass.
> - **Phase 3 — 3a DONE, structural allocator → `plan-03`.** Phase 3a (block-local
>   store-to-load forwarding) is committed (~17–21% on `float-leibniz`/`float-nbody`).
>   The full virtual-register allocator (the rest of Phase 3) is a backend-wide
>   architectural change and is scoped in **`plan-03-register-allocator.md`**.

This plan makes native-compiled `Float` (and scalar numeric) code dramatically
faster without changing any observable result, by attacking the three structural
costs the `benchmark/` suite exposed: a per-operation inf/NaN check emitted in the
integer domain, the absence of a register allocator (every value spills to a stack
slot between statements and every `Float` round-trips through general-purpose
registers), and the rematerialization of loop-invariant constants. The single
behavioral outcome a correct implementation produces: **identical program output
and identical error behavior to today, with the hot loops of `float-leibniz`,
`float-nbody`, `math-trig`, `math-invtrig`, and `math-explog` running materially
closer to `cc -O0` and within a small multiple of `cc -O2`.**

It complements:

- `./mfb spec language types` and `./mfb spec language error-model` (the `Float`
  finiteness contract — every live `Float` is finite; non-finite results raise
  `ErrFloatInf` / `ErrFloatNan` / `ErrFloatDomain`; the canonical specs live under
  `src/spec/language/**`).
- `./mfb spec diagnostics error-codes` (`ErrFloatInf` 77050014, `ErrFloatNan`
  77050013, `ErrFloatDomain` 77050012, `ErrOverflow` 77050010 — the build input for
  `errorCode::`; under `src/spec/diagnostics/02_error-codes.md`).
- `./mfb man math` (the `Float` overloads "return only finite values"; `Fixed`
  uses deterministic Q32.32 arithmetic).

### A note on "bit-identical" (two senses)

Two distinct properties travel under this name; the two float plans affect them
differently, and conflating them causes confusion:

- **Cross-version identity** — does a `Float` result's bits change *before vs.
  after* this work (i.e. do `.run` goldens churn)?
- **Cross-target identity** — do two targets (today both AArch64; a future x86_64)
  produce the *same* bits?

**This plan (plan-01) changes neither.** It alters only instruction selection and
where a value lives during a function body — never the arithmetic operations or
their rounding. Every `Float` result is bit-for-bit unchanged in both senses, which
is exactly why this plan is specified as *no golden churn*. (plan-02, by contrast,
spends cross-version identity *once* — FMA rounds a fused product a single time —
while keeping cross-target identity via uniform fusion. `Fixed` is the type
*guaranteed* bit-identical across targets; `Float` carries only the ≤1 ULP accuracy
contract.)

## 1. Goal

- Eliminate the redundant integer-domain inf/NaN check after `Float` arithmetic
  whose result is provably finite, and make the remaining checks cheaper.
- Introduce a register allocator with a floating-point register class so `Float`
  values live in `d`-registers across an expression and loop-carried locals stay
  in (callee-saved) registers across loop iterations instead of spilling to the
  stack on every statement.
- Stop rematerializing loop-invariant constants and subexpressions (the `-1.0`
  rebuilt with `mov`/`movk`/`fmov`/`fneg` every iteration) — hoist them.
- All three are pure codegen improvements: byte-for-byte identical program output,
  identical error codes at identical points, and `scripts/test-accept.sh` green.

### Non-goals (explicit constraints)

These are the guardrails; a change that violates one is wrong even if it is faster:

- **Do not link libm.** The transcendental kernels stay owned/in-tree (the
  `plan-01-libm-kernels` decision). This plan does not add any platform math
  import. Hardware FP *instructions* the CPU already provides (`fsqrt`, `frintn`,
  `fmadd`) are not libm and are not excluded — but adopting them in the kernels is
  out of scope here (see Non-Goals).
- **`Float` is not required to be bit-identical across targets; `Fixed` is.**
  This plan may freely use per-target hardware FP for `Float` because `Float`
  carries only the ≤1 ULP accuracy contract, not cross-ISA reproducibility. The
  deterministic Q32.32 `Fixed` path (`builder_fixed_math.rs`,
  `builder_simd_fixed_math.rs`) must remain bit-for-bit unchanged and is the
  carrier of cross-target reproducibility. No phase here touches Fixed lowering.
- **No change to observable error behavior.** `ErrFloatInf` / `ErrFloatNan` /
  `ErrFloatDomain` / `ErrOverflow` must still be raised, with the same code, for
  exactly the same inputs and at the same program point (TRAP routing,
  scope-drop ordering). A check may only be elided when the result is *provably*
  finite; it may never be elided on a guess.
- **No change to language surface, value/copy/move/freeze semantics, layout/ABI,
  or thread-transfer rules.** A `Float` is still an 8-byte value; its in-memory
  representation in records, collections, and thread-transfer buffers is
  unchanged. Register residency is a function-internal concern only — every value
  that escapes to memory (store to a local that is read across a call, record
  field, collection element, transfer buffer) is materialized to its existing
  8-byte little-endian f64 bit pattern exactly as today.
- **No golden churn.** `.run` goldens are unchanged; `.ir`/`.ast` goldens are
  unchanged (this is a code-emission change below the IR, not an IR change).

## 2. Current State

### 2.1 Value / register model

`CodeBuilder` lowers each function with a **bump-and-reset register allocator**,
not a real one:

- `allocate_register` (`builder_codegen_primitives.rs:4`) hands out
  `abi::temporary_register(self.next_register)` and increments `next_register`. It
  walks a fixed sequence (`x8, x9, …`) and **fails** when a single expression
  exceeds the temporary budget — hence the standing rule "break nested exprs into
  LETs; the allocator has no spilling."
- `reset_temporary_registers` (`builder_codegen_primitives.rs:24`) simply sets
  `next_register = 8`. It is called at the top of essentially every statement
  (`builder_numeric.rs:112,273,306,350,401,462`), so **no temporary survives a
  statement boundary**. Anything needed later is a local, and locals live in stack
  slots (`allocate_stack_object`, `builder_codegen_primitives.rs:50`) — read back
  with a `ldr` on every use and written with a `str` on every assignment.
- A `ValueResult`'s `location` is a **general-purpose register name** (a `String`
  like `"x9"`). There is no FP register class: a `Float` holds its f64 *bit
  pattern* in a GPR / stack slot, never in a `d`-register across operations.

### 2.2 Float arithmetic lowering

`emit_float_binary` (`builder_numeric.rs:847`) is the core of the tax. For `a + b`:

1. `load_numeric_as_double("d0", left)` / `("d1", right)` — `fmov` GPR→FP for each
   operand (`builder_numeric.rs:854`).
2. the one real instruction (`fadd`/`fmul`/`fdiv`/…).
3. `float_move_x_from_d(dst, "d0")` — `fmov` FP→GPR (`builder_numeric.rs:893`).
4. `emit_float_result_check(dst, …)` (`builder_numeric.rs:894`).

So every float op is bracketed by GPR↔FP `fmov` crossings *and* the result is
moved back to a GPR purely so the next stage can re-load it.

### 2.3 The inf/NaN check

`emit_float_result_check` (`builder_math.rs:981`) emits, **in the integer domain**,
after every arithmetic op: build the exponent mask `0x7ff0…` into `x17`,
`and`+`cmp`+`b.ne`; on the all-ones path build the mantissa mask, `and`+`cmp`+`b.eq`
to split inf from NaN, then a cold trap-return path. That is ~8–10 instructions and
a branch per float op, on top of the `fmov` round-trip. The recent commit
`5f7c5cc4` ("drop dead inf/NaN check on Float unary negation") and the comment at
`builder_numeric.rs:223` ("negation just flips the sign bit, so a finite operand
stays finite") establish the precedent that these checks are elidable when the
result is provably finite.

### 2.4 Constants

Float literals are materialized inline at each use (`mov`/`movk`/`fmov`, plus
`fneg` for `-1.0`) rather than loaded from a constant pool or hoisted; in a loop
they are rebuilt every iteration. There is a `local.constant` field
(`builder_codegen_primitives.rs:28`) carrying per-local constant propagation, but
no loop-invariant code motion.

### 2.5 Measured gaps (this machine, startup ~6 ms subtracted)

| benchmark | mfb compute | vs `cc -O0` | vs `cc -O2` |
|---|---|---|---|
| float-leibniz (pure arithmetic) | ~12 ms | 3.2× | 15× |
| math-trig (sin/cos/tan/atan2) | ~211 ms | 7× | 8× |
| math-explog (exp/log/log10/pow) | ~599 ms | 16× | 19× |

The pure-arithmetic row isolates §2.1–2.4 (this plan). The kernel rows add the
owned-kernel cost on top (out of scope — Non-Goals).

## 3. Design Overview

Three independently-landable pieces, ordered lowest-risk first (per
`.ai/planning.md`, highest-risk codegen last). Note this **reorders** the items as
listed in the request to land the safe, isolated win first and the structural
allocator last, behind the other two:

1. **Inf/NaN check elision + cheapening** (Phase 1) — a local change to
   `emit_float_binary` / `emit_float_result_check`. Track a `finite: bool` on
   float results and skip the check when provably finite; emit the unavoidable
   checks in the FP domain (`fabs` + `fcmp` against `+inf`) instead of the GPR mask.
   No new infrastructure; fully covered by existing goldens.
2. **Loop-invariant constant/expression hoisting** (Phase 2) — a pre-pass over
   loop bodies that lifts invariant float-literal materialization and invariant
   pure subexpressions to the loop preheader. Independently valuable even while
   values still spill (it removes recompute), and amplified by Phase 3.
3. **Register allocator with an FP register class** (Phase 3) — the structural
   fix. Replace bump-and-reset with a real allocator: an integer class and a
   floating-point class (`d8–d15` callee-saved for loop-carried values, `d0–d7`
   for temporaries), liveness so loop-carried locals stay resident across
   iterations, and spilling so the "break into LETs" limitation disappears. This
   is where correctness risk concentrates and it lands last, behind Phases 1–2.

The phases compose: Phase 1 removes per-op checks, Phase 2 removes per-iteration
recompute, Phase 3 removes per-use memory traffic and the GPR↔FP crossings. Phase 3
makes a `Float` `ValueResult.location` able to name a `d`-register, which is what
finally lets `fadd d_acc, d_acc, d_t` happen with no surrounding `fmov`/`ldr`/`str`.

## 4. Detailed Design — Phase 1: inf/NaN elision and cheapening

### 4.1 Provably-finite propagation

Add a `finite` predicate to the `Float` lowering result (either a field on
`ValueResult` or a side table keyed by the produced register). Rules:

- Literals, and the result of a check that already ran, are finite.
- `-x`, `abs(x)`, `floor/ceil/round(x)`, `min/max(a,b)`, `clamp(...)` of finite
  inputs are finite (sign/magnitude-only or order-only operations).
- `a + b`, `a - b`, `a * b` of finite inputs are **not** provably finite (can
  overflow to ±inf) → keep the check.
- `a / b`, `MOD`: divisor already domain-checked non-zero, but the quotient can
  still overflow → keep the result check; the *divide-by-zero pre-check* stays.
- Kernel outputs keep their existing kernel-specific error mapping unchanged.

Only elide when the rule proves finiteness. When in doubt, keep the check — this is
the guardrail from §1.

### 4.2 Cheaper residual check

Where a check is still required, replace the GPR exponent/mantissa mask in
`emit_float_result_check` (`builder_math.rs:981`) with an FP-domain test on the
value still live in `d0`: `fabs d2, d0`; compare against a `+inf` constant; branch
to the existing trap labels, distinguishing inf from NaN with an FP unordered
compare (`fcmp`'s NaN flag) rather than a second GPR mask. This removes the
`fmov` FP→GPR done *solely* to feed the integer mask, and shrinks the check from
~8–10 instructions to ~3–4. The trap-return bodies (`emit_float_nan_return`,
`emit_float_inf_return`, `emit_float_overflow_return`) are reused verbatim, so
error codes and TRAP routing are byte-identical.

### 4.3 Risk and proof

Lowest risk: the trap paths and error codes are untouched; only the *guard*
changes. Proven by the existing `tests/func_math_*` valid/invalid suite and the
`*_rt` float-domain/overflow runtime tests, plus a new pair that forces an
overflow through `+`/`*` (result check must still fire) and one that exercises an
elided op (`-x`, `abs`) for a value that would be inf if mis-elided.

## 5. Detailed Design — Phase 2: loop-invariant hoisting

A pre-pass over each loop body (the `FOR`/`WHILE` lowering in
`builder_control.rs`) identifies float-literal materializations and pure
subexpressions whose inputs are all loop-invariant (not assigned in the body, not
derived from the induction variable) and emits them once in the loop preheader,
referencing the hoisted result inside the body.

- V1 scope: hoist (a) float/integer literal materialization (the `-1.0` rebuild),
  and (b) invariant pure arithmetic subexpressions with no side effects and no
  trap potential, **or** whose trap would be hoistable without changing observed
  behavior (an invariant `a/b` that would trap must trap on iteration 0 anyway, so
  hoisting its trap to the preheader is observably identical only when the loop
  body is entered at least once — guard by hoisting trap-capable invariants only
  when the loop is provably non-empty, else leave them in-body). Conservative
  default: hoist only non-trapping invariants in V1.
- Until Phase 3 lands, a hoisted value lives in a stack slot (loaded once, reloaded
  per iteration) — already a win over rematerialization. After Phase 3 it stays in
  a register across the loop.

Proof: goldens unchanged; a runtime test with an invariant subexpression inside a
loop whose body also traps must still trap at the same iteration.

## 6. Detailed Design — Phase 3: register allocator with FP class

### 6.1 Model change

Replace the bump-and-reset counter (`next_register` /
`reset_temporary_registers`) with a per-function allocator over two register
classes:

- **Integer class** — the current GPR temporaries plus callee-saved `x19–x28`.
- **Floating-point class** — `d0–d7` (caller-saved temporaries) and `d8–d15`
  (callee-saved, for loop-carried `Float` locals). `d8–d15`'s lower 64 bits are
  callee-saved by the AArch64 PCS; spill/restore them in the prologue/epilogue via
  the existing `used_callee_saved` machinery (`mark_register_used`,
  `builder_codegen_primitives.rs:16`), extended to the FP class.

A `ValueResult.location` gains the ability to name a `d`-register. `Float`
arithmetic then becomes `fadd d_dst, d_a, d_b` with operands already resident — no
`load_numeric_as_double`, no `float_move_x_from_d`, no per-statement spill. A value
is materialized to/from its 8-byte memory form only at the genuine boundaries
(store to an escaping local, record field, collection element, transfer buffer,
call argument/return per the existing ABI) — preserving layout/ABI exactly (§1).

### 6.2 Liveness and loop-carried residency

Compute liveness over the existing lowered statement stream so that:

- loop-carried locals (`sum`, `sign`, induction-derived accumulators) are assigned
  callee-saved registers and **stay resident across iterations** instead of
  `ldr`/`str` per use;
- spilling exists (pressure beyond the file spills to stack slots — the same
  `allocate_stack_object` mechanism), **removing** the "break nested exprs into
  LETs" limitation rather than preserving it.

### 6.3 Staging within Phase 3 (de-risking)

3a. **Intra-statement FP residency (peephole bridge).** Before the full allocator,
    keep a `Float` in its `d`-register between consecutive ops within one
    expression and delete dead `fmov xN,dM`/`fmov dM,xN` pairs. Captures a large
    slice of the win with minimal structural change; independently landable.
3b. **FP register class + callee-saved FP spill/restore** in the allocator.
3c. **Liveness-driven loop-carried residency + general spilling.**

Each sub-phase is separately testable and reverts cleanly.

### 6.4 Risk

Highest risk in the plan: a mis-assigned or mis-spilled register is a silent
miscompile. Mitigation: land behind Phases 1–2; gate each sub-phase on the full
`scripts/test-accept.sh` plus the function-test suite; add a differential check
that the suite's `.run` goldens are byte-identical before/after each sub-phase
(they must be — output is unchanged by definition). The arena/register-clobber
gotchas already recorded (copy_record aliasing, arena_alloc clobbering
x9/x10/x14/x15/x20–x28) become *allocator constraints*: the allocator must treat
the registers clobbered across `bl` to runtime helpers as caller-saved and not
hold live values across those calls.

## Layout / ABI Impact

None. This plan changes only *in-register residency during a function body*. The
in-memory representation of every type is unchanged: a `Float` is still an 8-byte
little-endian f64 everywhere it is stored (locals that escape, record fields,
collection elements, thread-transfer buffers), and the call ABI (argument/return
registers, callee-saved set, stack alignment) is unchanged. No `mfb spec memory` /
`mfb spec package` topic changes. Cross-target reproducibility for `Fixed` is
untouched (no Fixed lowering is modified).

## Phases

1. **Inf/NaN elision + FP-domain cheapening** (§4). Lowest risk, isolated, no new
   infra. Acceptance: full suite green; new valid/invalid tests for an elided op
   and a still-checked overflow; disassembly of `float-leibniz` shows the
   per-op GPR mask gone.
2. **Loop-invariant hoisting** (§5). Acceptance: suite green; `-1.0` no longer
   rebuilt inside the leibniz loop (disasm); invariant-trap runtime test traps at
   the same point.
3. **Register allocator with FP class** (§6), staged 3a→3b→3c. Highest risk, last.
   Acceptance per sub-phase: full suite + acceptance green; `.run` goldens
   byte-identical; leibniz hot loop shows `fadd`/`fmul` on resident `d`-registers
   with no per-iteration `ldr`/`str` of `sum`/`sign`.

## Validation Plan

- **Function tests:** the existing `tests/func_math_*_valid/**` and `_invalid/**`
  (full overload coverage) must stay green unchanged. Add: `func_math` (or a
  numeric-codegen) valid/invalid pair proving (a) an elided-check op stays correct
  and (b) `+`/`*` overflow still raises `ErrFloatInf`/`ErrOverflow` at the same
  point; an invariant-in-loop runtime test for Phase 2.
- **Runtime proof (not just goldens):** the `benchmark/` suite is the execution
  proof. Capture `BENCH_RUNS=N` medians for `float-leibniz`, `float-nbody`,
  `math-trig`, `math-invtrig`, `math-explog` before each phase and after, and
  record the ratio improvement vs `cc -O0`/`-O2` in the phase's commit message.
  The bar: Phase 1+2 visibly narrow the pure-arithmetic gap; Phase 3 brings
  `float-leibniz` compute to a small multiple of `cc -O0`.
- **Output identity:** every benchmark's `mfb` checksum is unchanged before/after
  every phase (output is discarded by the runner, so assert it explicitly with a
  one-off diff during development).
- **Doc sync (mandatory, each phase updates the spec as it lands):**
  - Phase 1: if `src/spec/architecture/17_math-kernels.md` or
    `…/06_native.md` describes the per-op finiteness check, update it to state the
    elision rule (provably-finite results skip the check) and the FP-domain test;
    confirm `./mfb spec language error-model` still matches the (unchanged) trigger
    points and fix it if it ever over-/under-specified them.
  - Phase 3: update `src/spec/architecture/06_native.md` (and
    `…/13_native-ir.md` / `mfb spec memory native-calling-convention` /
    `…/runtime-helper-abi` as touched) to replace the bump-and-reset / "no spilling,
    break nested exprs into LETs" value-model description with the register
    allocator, the integer + floating-point register classes, callee-saved `d8–d15`
    usage, and spilling. The standing rule "break nested exprs into LETs" must be
    removed from the spec/compiler notes in the same phase that removes the
    limitation.
  - No error-code, language-surface, or layout topic changes (none occur).
  - `.ai/specifications.md` requires the embedded spec stay current with every
    compiler change; treat the above as the concrete checklist, not optional.
- **Acceptance:** `scripts/test-accept.sh target/debug/mfb target/accept-actual`
  green after every phase.

## Open Decisions

- **FP-domain inf/NaN test shape** (§4.2) — recommend `fabs`+`fcmp` against a
  `+inf` constant reused from a per-function pool, vs. reading FPSR exception
  flags after the op. Recommend the compare form: no dependence on FPSR being
  clean across the kernel calls, and trivially correct. (§4.2)
- **Phase 2 trap-capable invariants** — recommend the conservative V1 (hoist only
  non-trapping invariants; leave invariant `a/b`/`MOD` in-body) and revisit once
  Phase 3's liveness makes non-emptiness analysis cheap. (§5)
- **Allocator algorithm** (§6) — recommend linear-scan over the existing linear
  statement stream (simple, fast, fits the current single-pass shape) vs. a graph
  colorer (more optimal, more machinery). Linear-scan first; it is enough to keep
  loop-carried values resident. (§6.2)

## Non-Goals

- Adopting hardware `fsqrt`/`frintn`/FMA *inside the owned kernels*, or any kernel
  rewrite (degree reduction, table reduction). The `Float`-not-bit-identical
  decision *enables* that later; it is a separate plan. This plan does not touch
  `builder_math.rs`/`builder_simd_float_math.rs`/`builder_pow.rs` kernel bodies
  except `emit_float_result_check` (§4.2).
- Any `Fixed` lowering change (deterministic Q32.32 stays bit-identical).
- Linking libm (still owned, never linked).
- Auto-vectorizing scalar loops, or SIMD widening of scalar `Float` code.
- IR-level optimization; this is all below the NIR, so `.ir`/`.ast` goldens do not
  move.

## Summary

The real engineering risk is concentrated in Phase 3's register allocator — a
miscompile there is silent — which is why it lands last, staged, behind the two
cheaper wins and gated on byte-identical `.run` goldens. Phases 1 and 2 are
low-risk, separately valuable, and need no new infrastructure. Nothing in this plan
changes a single byte of program output, any error code or its trigger point, the
owned (un-linked) transcendental kernels, the deterministic bit-identical `Fixed`
path, or the memory layout / ABI of any value — it only stops the backend from
storing floats in integer registers, recomputing constants, re-checking provably
finite results, and reloading every value from the stack on every use.
