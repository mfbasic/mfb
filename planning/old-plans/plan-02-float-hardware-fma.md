# MFBASIC Float Hardware-Instruction (FMA) Plan

Last updated: 2026-07-08

> **Status (2026-07-08). Still unstarted; both blocking preconditions have now
> landed, so all three phases are fully unblocked. The one substantive change since
> the plan was written is that the codebase moved to a 3-backend MIR architecture
> (aarch64 + x86_64 + riscv64), so Phases 2–3 must be re-anchored from the
> AArch64-only builder to the shared MIR layer (§0, §2.1, §5).**
>
> - **Phase 1 (direct-op mop-up) — NOT done.** Scalar `math::min`/`max(Float)` still
>   compare/branch — `float_subtract_d` + `float_compare_zero_d` + branch
>   (`builder_math.rs:709-716`), not `fminnm`/`fmaxnm`, and it still hardcodes
>   `d0/d1/d2` instead of using the FP residency that now exists. Scalar
>   `math::abs(Float)` still clears the sign bit with a GPR `and_registers`
>   (`builder_math.rs:511`), not `fabs`.
> - **Phase 2 (encoders + dormant recognizer) — partial and dead.** `fmadd_d` exists
>   as a **MIR mirror op** (`CodeOp::FMaddD`; `mir.rs:818,1441`) and is emitted by the
>   aarch64 and riscv64 backends — **but x86_64 has no scalar `fmadd_d` emitter** (it
>   only does vector `vfmadd231pd`). The AArch64 encoder wrapper
>   `abi::float_multiply_add_d` is still `#[allow(dead_code)]` with **zero callers**.
>   `fmsub`/`fnmadd`/`fnmsub` scalar forms are absent on every backend; there is no
>   recognizer. (`fabs_d` also exists, but plan-16 added it for the finiteness check,
>   not for `math::abs` — §4.)
> - **Phase 3 (enable fusion) — NOT done.** `emit_float_binary`
>   (`builder_numeric.rs:976`) still lowers one operator per call; no `a*b±c` ever
>   becomes `fmadd`.
> - **Precondition 1 satisfied — FP residency (§2.1).** plan-01 Phase 3 / FP residency
>   landed via plan-03 (FP register class) + plan-16 Piece B (`float_residents` + the
>   `fmov`-shuttle peephole, `f45dec39`) and plan-01-float-dnative (d-register-native
>   `Float` carrier). Operands in a chain already live in `d`-registers.
> - **Precondition 2 satisfied — finiteness at boundaries (§6.2).** plan-17
>   (float-finiteness-at-boundaries) has **landed**. Finiteness is now checked at
>   observation boundaries, not per op, so the "does the fused intermediate trap?"
>   question is already answered for *every* intermediate (fused or not). Phase 3 now
>   reduces to pure instruction selection with **no trap-semantics decision and no
>   §6.2 spec edit of its own** — plan-17 owns that wording. §6.2 below is retained
>   only as historical rationale.
> - **Reconciliation carried into the body:** §2 motivation is partly stale — plan-16
>   already removed the `fmov`-to-GPR round-trips this plan originally cited; fusion's
>   remaining win is *fewer instructions + single rounding*, **not** `fmov`
>   elimination.

## 0. Architecture note — this plan is now multi-backend (MIR)

When first written this plan targeted a single AArch64 backend and described the
encoders/recognizer in AArch64 builder terms. The codebase has since moved to a
shared **MIR** layer with three backends: **aarch64**, **x86_64**, and **riscv64**.
Native `Float` ops are neutral MIR mnemonics (`fadd_d`, `fmul_d`, `fmadd_d`, …) that
each backend lowers. This changes *where* the work lands, not the goal:

- **The recognizer belongs at the MIR / lowering seam, not in the AArch64 builder**,
  so fusion applies uniformly on all three targets (this is exactly the "decide
  fusion in target-neutral lowering" policy the Settled Decisions already require —
  it is now a structural fact, not an aspiration).
- **`fmadd_d` is already a shared MIR mirror op** emitted by aarch64 and riscv64.
  Phase 2's encoder work is therefore: (a) add a **scalar `fmadd_d` emitter to
  x86_64** (currently missing — it only has vector FMA3), and (b) add
  `fmsub_d`/`fnmadd_d`/`fnmsub_d` as **new MIR mirror ops** with an emitter on each of
  the three backends, byte-tested per backend.
- **`fminnm`/`fmaxnm`/`fabs` (Phase 1)** likewise become MIR ops (`fminnm_d`/
  `fmaxnm_d`/`fabs_d` — `fabs_d` already exists) lowered on all three backends,
  rather than an AArch64-only `abi::` call.
- Any future backend must supply these scalar FP forms or opt out of fusion; the
  x86 FMA3 baseline requirement in Settled Decisions now also literally applies to
  the existing x86_64 backend.

This plan finishes moving native `Float` code onto the hardware FP instructions the
CPU already provides — specifically **fused multiply-add** — in the one place it is
not yet used: user-level scalar `a*b±c` expressions (the owned transcendental
kernels and the directly-mappable ops are *already* on hardware). The single
behavioral outcome a correct implementation produces: **`Float` multiply-accumulate
chains in user code lower to `fmadd`/`fmsub`/`fnmadd`/`fnmsub` (a single fused,
single-rounded instruction), making `float-nbody` and the math benchmarks faster
and — because FMA rounds once instead of twice — at least as accurate, with every
result re-validated ≤1 ULP and the small set of affected goldens regenerated and
justified.**

It complements:

- `./mfb spec architecture math-kernels` (`src/spec/architecture/17_math-kernels.md`
  — the ≤1 ULP accuracy contract and the kernel design these changes must respect).
- `./mfb spec architecture native` and the AArch64 instruction-set topic
  (`src/spec/architecture/14_aarch64-instruction-set.md` — where the new
  `fmadd`/`fmsub`/`fnmadd`/`fnmsub` scalar encoders are documented).
- `./mfb spec language error-model` (`src/spec/language/08_error-model.md` — the
  `Float` finiteness contract; this plan clarifies that an *intermediate* product
  in a fused expression is not a named `Float` and so does not independently trap).
- `tools/math-kernels/runtime_ulp.py` (the ULP harness — the correctness gate for
  every accuracy-affecting change here).

### A note on "bit-identical" (two senses)

Two distinct properties travel under this name; this plan affects them differently:

- **Cross-version identity** — does a `Float` result's bits change *before vs.
  after* this work (i.e. do `.run` goldens churn)?
- **Cross-target identity** — do two targets (today both AArch64; a future x86_64)
  produce the *same* bits?

**This plan (plan-02) spends cross-version identity exactly once, and preserves
cross-target identity.** FMA rounds a fused `a*b±c` a single time instead of twice,
so some `Float` results shift by a last bit — *toward* correctly-rounded truth —
which is a one-time, justified golden regeneration (§6.3). It is not ongoing drift.
Cross-target identity is preserved because fusion is decided in target-neutral
lowering and IEEE-754 FMA is deterministic and correctly-rounded (Settled
Decisions), so every target fuses identically. Note the spec does **not** *guarantee*
`Float` cross-target identity — only `Fixed` is guaranteed; the `Float` freedom is
held in reserve for a future target lacking hardware FMA and is not exercised here.
(plan-01, by contrast, changes *neither* sense — it is pure codegen with no result
change.)

## 1. Goal

- Fuse `Float` multiply-then-add/subtract in **user-level scalar expressions** into
  the single AArch64 FMA instructions (`fmadd`, `fmsub`, `fnmadd`, `fnmsub`),
  removing the intermediate `fmov`/store and the intermediate inf/NaN check.
- Mop up the remaining non-FMA direct ops where a hardware instruction is a clean
  win: `math::min`/`max` → `fminnm`/`fmaxnm`, `math::abs` (Float) → `fabs`.
- Audit and close any straggler in the owned kernels still using discrete
  `fmul`+`fadd` instead of `fmla`/`fmls`.
- Prove, via `runtime_ulp.py`, that every changed operation stays ≤1 ULP and does
  not regress against the current implementation; regenerate only the goldens whose
  (now more accurate) values legitimately change, with each change justified.

### Non-goals (explicit constraints)

- **Do not link libm.** FMA, `fminnm`, `fabs` are CPU instructions, not libm. This
  plan adds no platform math import — it is fully consistent with the owned-kernel
  decision (`plan-01-libm-kernels`).
- **`Fixed` is untouched and stays bit-identical across targets.** FMA is a
  floating-point instruction; the deterministic Q32.32 `Fixed` path
  (`builder_fixed_math.rs`, `builder_simd_fixed_math.rs`, the `emit_fixed_*`
  routines) is not modified. `Fixed` remains the carrier of cross-target
  reproducibility.
- **`Float` is not required to be bit-identical across targets** — but this plan
  *chooses* to decide fusion in target-neutral lowering so that IEEE-754 FMA (a
  correctly-rounded, deterministic operation) produces identical `Float` results on
  every IEEE target that has FMA. That keeps the `.run` goldens portable. The
  "Float not bit-identical" freedom is the fallback for a future target lacking
  hardware FMA, not a license to diverge gratuitously (see Open Decisions).
- **No language-surface, layout, ABI, or value/copy/move/freeze change.** A `Float`
  is still an 8-byte f64; only the instruction selection inside a function body
  changes.
- **Accuracy must not regress.** A fused result must be ≤1 ULP and no worse than the
  current discrete result on the `runtime_ulp.py` sweep. If any input regresses,
  that fusion is not applied there.

## 2. Current State

Most `Float` operations are **already on hardware** — this plan must not redo them:

- **Scalar `sqrt`** → `float_sqrt_d` (FSQRT, correctly-rounded) with a `< 0` domain
  pre-check (`builder_math.rs:894`).
- **Scalar `floor`/`ceil`/`round`** → hardware float→signed-int conversion
  (`float_floor_to_signed_x` / `_ceil_` / `_round_`, i.e. the FCVT family with the
  matching rounding mode; `round` is ties-away, matching the man page) plus an
  Integer-range overflow check (`builder_math.rs:704`).
- **Unary negate** → `float_negate_d` (FNEG, `builder_numeric.rs:230`).
- **Array overloads** → NEON hardware throughout: `vector_fsqrt`
  (`builder_simd_math.rs:286`, `builder_simd_float_math.rs:369`), `frintm_v` /
  `frintp_v` / `frinta_v` for floor/ceil/round (`builder_simd_math.rs:66`).
- **Transcendental kernels already use FMA.** `builder_simd_float_math.rs` evaluates
  its polynomials and argument reductions with `vector_fmla`/`vector_fmls`
  (e.g. lines 368, 468–477, 499–525, 565, 600–602). So sin/cos/exp/log and the
  reductions are already fused.

What is **not** on hardware FMA:

- **User-level scalar `a*b±c`.** `emit_float_binary` (`builder_numeric.rs:976`)
  lowers exactly one operator per call: `a*b` emits `fmul d` then a finiteness check,
  and `± c` emits a separate `fadd`/`fsub d` then another check. (Post-plan-16 the
  operands and result stay in `d`-registers and the check is FP-domain `fabs`/`fcmp`;
  the `fmov`-to-GPR round-trips the original draft described are gone — see §2.1.) A
  multiply-accumulate chain — pervasive in `float-nbody`
  (`vx0 = vx0 - dx01 * m1 * mag01`, the `dt * v` integration, the energy sums) —
  never becomes a single `fmadd`. Each conceptual FMA is still two FP ops with two
  finiteness checks instead of one fused, single-rounded `fmadd` with one check.
- **`math::min`/`max` (scalar Float)** — comparison/branch based
  (`builder_math.rs:644-716`, `lower_math_min_max`): `float_subtract_d` +
  `float_compare_zero_d` + branch on hardcoded `d0/d1/d2`, not `fminnm`/`fmaxnm`.
- **`math::abs` (scalar Float)** — integer AND clearing the sign bit in a GPR
  (`builder_math.rs:511`, `lower_math_abs`), not `fabs`.

### 2.1 Relationship to plan-01

`plan-01-float-codegen` makes `Float` values FP-register-resident (its Phase 3) and
removes the per-op inf/NaN checks where provably finite. Scalar FMA fusion here is
only profitable **on top of** that residency: fusing `a*b+c` while operands live in
GPRs would still pay `fmov` round-trips and gain little. Therefore **Phase 3 of this
plan depends on plan-01 Phase 3.** Phases 1–2 here are independent of plan-01.

**Update (2026-07-08): that dependency is fully satisfied.** FP residency shipped via
plan-03 (the FP register class), plan-16 Piece B (the `float_residents` map plus the
post-allocation `fmov`-shuttle peephole, `f45dec39`), and plan-01-float-dnative (the
d-register-native `Float` carrier). So `Float` operands in a chain already live in
`d`-registers, and the per-op `fmov`-to-GPR/reload that §2 describes is **already
gone**. Two consequences: Phase 3 is unblocked, and its win is now purely *two ops →
one fused op* + single-rounding accuracy (the `fmov` savings the plan originally also
counted have already been banked by plan-16).

Note the recognizer that consumes this residency must live at the **MIR seam** so it
fuses uniformly across aarch64/x86_64/riscv64 (§0), not in the AArch64 builder as the
original §5 drafted.

## 3. Design Overview

Three phases, lowest-risk first:

1. **Direct-op mop-up + kernel FMA audit** (§4) — `min`/`max` → `fminnm`/`fmaxnm`,
   Float `abs` → `fabs`; and an audit confirming every kernel polynomial uses
   `fmla`/`fmls`, fixing any straggler. For finite operands `fminnm`/`fmaxnm`/`fabs`
   are bit-identical to today's branch/AND results, so this phase is expected to
   produce **zero golden churn** — lowest risk, separately valuable.
2. **Scalar FMA encoders + recognizer (no semantic change yet)** (§5) — add the
   `fmadd`/`fmsub`/`fnmadd`/`fnmsub` scalar instruction encoders and a
   multiply-accumulate *recognizer* over the lowered float-op stream, but keep it
   **disabled** behind the residency precondition. Lands the machinery and unit
   tests with no behavior change.
3. **Enable scalar fusion** (§6) — turn on fusion once plan-01 FP-residency is
   present. This is where results change (single vs double rounding) and where the
   intermediate-overflow-trap semantics are decided (§6.2). Highest risk; behind the
   ULP gate and explicit golden regeneration.

Correctness risk concentrates in Phase 3: it both changes numeric results and
changes when (whether) an intermediate overflow traps.

## 4. Detailed Design — Phase 1: direct-op mop-up + kernel audit

- **`min`/`max` (Float):** replace the compare/branch with `fminnm`/`fmaxnm`. Every
  live MFBASIC `Float` is finite, so the IEEE NaN-quieting distinction between
  `fmin` and `fminnm` is moot; `fminnm`/`fmaxnm` are chosen for their standard
  finite semantics. Result is bit-identical to the current ordered compare → no
  golden change expected (assert it).
- **`abs` (Float):** replace the GPR `and` (sign-bit clear) with `fabs` once the
  value is FP-resident; until plan-01 residency lands, leave the GPR `and` (it is
  already a single cheap instruction). Bit-identical.
- **Kernel FMA audit:** sweep `builder_simd_float_math.rs` and `builder_pow.rs` for
  any `fmul` immediately feeding an `fadd`/`fsub` on the same lane that was *not*
  written as `fmla`/`fmls`. Convert stragglers, each gated on `runtime_ulp.py`
  showing equal-or-better ULP. Expected to be a small or empty set (kernels are
  largely fused already).

Proof: full suite + acceptance green with no `.run` golden change; `runtime_ulp.py`
unchanged or improved for any kernel straggler converted.

## 5. Detailed Design — Phase 2: encoders + recognizer (dormant)

- **Encoders (now MIR ops across three backends — see §0).** `fmadd_d`
  (= `lhs*rhs + addend`, one round) **already exists as a shared MIR mirror op**
  (`CodeOp::FMaddD`; `mir.rs:818,1441`; field reqs in `code_impl.rs`; classified in
  `peephole.rs`) and is emitted by the **aarch64** (`encode/emitter.rs:emit_fmadd_d`)
  and **riscv64** (`encode/emitter.rs:293`) backends. Two encoder gaps remain:
  - **x86_64 has no scalar `fmadd_d`** (only vector `vfmadd231pd`). Add a scalar
    `vfmadd*sd` emitter, documented in
    `src/docs/spec/architecture/15_x86_64-instruction-set.md`.
  - **`fmsub`/`fnmadd`/`fnmsub` are absent on all three backends.** Add them as new
    MIR mirror ops (`fmsub_d`/`fnmadd_d`/`fnmsub_d`) with an emitter on each backend —
    AArch64 `fmsub`/`fnmadd`/`fnmsub` scalar forms, riscv64 `fmsub.d`/`fnmadd.d`/
    `fnmsub.d`, x86 `vfnmadd*sd`/`vfmsub*sd`/`vfnmsub*sd`. Document in
    `.../14_aarch64-instruction-set.md`, `.../15_x86_64-instruction-set.md`,
    `.../16_mir-instruction-set.md`, and the riscv64 topic. Unit-test each encoding
    against known-good bytes **per backend**; drop the `dead_code` allowance on the
    unused AArch64 `abi::float_multiply_add_d` wrapper (or delete it — the MIR op is
    the live path) once the recognizer emits.
- **Recognizer (at the MIR seam).** A peephole over the per-statement lowered
  MIR float-op stream that
  matches a `*` whose single-use result feeds a `+`/`-` (in either operand
  position), mapping the four sign combinations to the four FMA forms:
  - `a*b + c` → `fmadd(a,b,c)`
  - `a*b - c` → `fmsub` form (`a*b + (-c)` is *not* what fmsub computes; use the
    encoding whose rounding matches `round(a*b - c)` — `fnmsub`/`fmsub` selected by
    the exact AArch64 semantics, pinned by a byte-level unit test and a numeric
    test, not by guessing).
  - `c - a*b` → `fnmadd`/`fmsub` (the form computing `round(c - a*b)`).
  - `-(a*b)` followed by `+c` similarly.
  The recognizer requires the `*` result to be **single-use** (no other consumer
  observes the unrounded product) and both the `*` and the `±` to be ordinary
  checked `Float` ops (not already-fused, not a kernel boundary).
- **Dormant:** the plan-01 residency precondition is now always satisfied, so the
  "dormant" gate is purely a staging choice — land the encoders and the recognizer
  compiled and unit-tested but **not yet wired into emission**, keeping Phase 2 a
  no-op on generated code so the encoder/recognizer machinery can be reviewed before
  any result changes (Phase 3 flips the switch).

Proof: encoder unit tests (byte-exact) and recognizer unit tests (pattern → chosen
form) pass; no change to any generated binary or golden.

## 6. Detailed Design — Phase 3: enable scalar fusion

### 6.1 Emission

With plan-01 FP-residency, the recognizer's matched chains emit a single
`fmadd`/`fmsub`/`fnmadd`/`fnmsub` on resident `d`-registers, followed by **one**
inf/NaN result check on the fused output (replacing the two checks the unfused form
emitted). The intermediate product is never materialized to a GPR and never
separately checked.

### 6.2 Intermediate-overflow trap semantics (the decision)

FMA computes `a*b` at infinite precision and rounds once. Consequence: an expression
like `a*b + c` where `a*b` would overflow to ±inf in isolation but `a*b + c` is
finite **no longer raises `ErrFloatInf` on the intermediate** — it produces the
finite fused result. The current unfused code *does* trap on the intermediate `*`.

Decision (settled): treat the intermediate product as **not a named `Float`** —
the language names only the operands and the final value of the expression, so the
finiteness contract applies to those, not to a compiler-internal product. Fusion is
therefore conformant and, in this edge case, *more* correct (it returns the true
finite result instead of a spurious overflow). Clarify this in
`src/spec/language/08_error-model.md` and `…/architecture/17_math-kernels.md`, and
regenerate any golden that asserted the intermediate trap, justifying each as an
intended fused-semantics change. Fusion is still **not** applied when the `*`
result is multiply-used (then the product *is* a named, observable value).

**Update (2026-07-08): this section is now MOOT — plan-17 has landed.**
plan-17 (float-finiteness-at-boundaries) relaxed the whole finiteness rule to "no
*user-accessible* `Float` is non-finite," checked at observation boundaries rather
than per op — which is exactly this §6.2 decision, generalized to *every* intermediate
(not just fused products). Because plan-17 is in the tree, there is no per-op
intermediate check left to reason about, the "single-use vs multiply-used product"
distinction above is moot (a multiply-used product is just a value that crosses an
observation boundary and is checked there like any other), and **Phase 3 reduces to
pure instruction selection with no trap-semantics decision and no §6.2 spec edit of
its own** — plan-17 already owns that spec wording. This section is retained only as
historical rationale for why fusion is conformant; it prescribes no work.

### 6.3 ULP gate and golden regeneration

- Run `tools/math-kernels/runtime_ulp.py` across the affected ops and any kernel
  touched; require ≤1 ULP and no per-input regression vs. the pre-fusion build.
- Diff every `.run` golden. Expected changes: last-bit differences in
  `Float`-printing tests and the benchmark checksums (toward correctly-rounded
  truth) and removal of the rare intermediate-overflow trap (§6.2). Regenerate
  exactly those, listing each in the commit message with its before/after value and
  the ULP-truth it moved toward.

Proof: `runtime_ulp.py` non-regression table; regenerated goldens with per-file
justification; `float-nbody` and math benchmark medians improved; acceptance green.

## Layout / ABI Impact

None. Instruction selection only. A `Float` is still an 8-byte f64 in every stored
location; the call ABI and callee-saved sets are unchanged. No `mfb spec memory` /
`mfb spec package` topic changes. `Fixed` lowering and its bit-identical cross-target
guarantee are untouched.

## Phases

1. **Direct-op mop-up + kernel audit** (§4). Lowest risk; expected zero golden
   churn. Acceptance: suite + acceptance green, no `.run` diff; `runtime_ulp.py`
   equal-or-better for any kernel straggler converted.
2. **FMA encoders (MIR ops, all 3 backends) + dormant recognizer** (§0, §5).
   Acceptance: byte-exact encoder unit tests *per backend* (aarch64/x86_64/riscv64);
   recognizer pattern tests; zero generated-code change.
3. **Enable scalar fusion** (§6). Both preconditions (FP residency, plan-17) landed;
   sequenced after Phase 2. Highest risk, last.
   Acceptance: ULP non-regression table; justified golden regeneration;
   `float-nbody`/math benchmark speedup; acceptance green.

## Validation Plan

- **Function tests:** existing `tests/func_math_*_valid/**` and `_invalid/**` stay
  green (Phase 1–2 with no value change). Phase 3: add a valid test proving
  `a*b+c` fuses and stays ≤1 ULP, and an invalid/edge test pinning the §6.2
  intermediate-overflow decision (multiply-used product still traps; single-use
  fused product does not).
- **ULP proof:** `tools/math-kernels/runtime_ulp.py` is the gate for every
  accuracy-affecting change — ≤1 ULP and no regression vs. the prior build.
- **Runtime/speed proof:** `benchmark/float-nbody` (FMA-dense), `math-trig`,
  `math-explog` medians before/after Phase 3, recorded in the commit.
- **Output identity / churn:** Phases 1–2 assert byte-identical `.run` goldens.
  Phase 3 regenerates only the legitimately-changed goldens, each justified.
- **Doc sync (mandatory, each phase updates the spec as it lands):**
  - Phase 1: if `min`/`max`/`abs` lowering is described in
    `src/spec/architecture/06_native.md` or `…/17_math-kernels.md`, update it to
    name the hardware instructions (`fminnm`/`fmaxnm`/`fabs`).
  - Phase 2: document the new scalar FMA encoders (`fmadd`/`fmsub`/`fnmadd`/
    `fnmsub`) across all affected backends — the MIR mnemonics in
    `.../16_mir-instruction-set.md`, and the per-backend encodings in
    `.../14_aarch64-instruction-set.md`, `.../15_x86_64-instruction-set.md`, and the
    riscv64 instruction-set topic (note the real tree path is `src/docs/spec/...`).
  - Phase 3: (a) clarify in `src/spec/language/08_error-model.md` that a fused
    expression's intermediate product is not a named `Float` and does not
    independently trap (§6.2 Settled); (b) update `…/06_native.md` to describe
    scalar multiply-accumulate fusion as the lowering for `Float` `a*b±c`; (c)
    reconcile `…/17_math-kernels.md` — its determinism wording ("no last-ULP
    platform drift") must be made precise: `Fixed` is *guaranteed* bit-identical
    across targets; `Float` is bit-identical across IEEE targets *in practice*
    under the uniform-fusion policy but is *not contractually guaranteed* to be.
  - `.ai/specifications.md` requires the embedded spec stay current with every
    compiler change; treat the above as the concrete checklist, not optional.
- **Acceptance:** `scripts/test-accept.sh target/debug/mfb target/accept-actual`
  green after every phase.

## Settled Decisions

- **Cross-target fusion policy — SETTLED: uniform fusion.** Fusion is decided in
  target-neutral MIR lowering (§0), so it applies identically on all three current
  backends (aarch64/x86_64/riscv64); the x86_64 backend must meet the FMA3 baseline
  (x86-64-v3) for the scalar `vfmadd*sd` forms, matching its existing vector FMA3 use.
  Because IEEE-754 FMA is a correctly-rounded, deterministic operation, fusing the
  *same* expression on every target yields identical `Float` bits — so `.run`
  goldens stay portable and `Float` remains bit-identical across IEEE targets *in
  practice*. (The spec still does not *guarantee* `Float` cross-target identity —
  that headroom is held in reserve for a future target that lacks hardware FMA; we
  simply do not exercise it here.) (§1, §6.2)
- **Intermediate-product finiteness — SETTLED** (§6.2): a fused expression's
  internal product is not a named `Float` and does not independently trap.

## Open Decisions

- **`fmsub`/`fnmadd`/`fnmsub` form selection** — recommend pinning each sign pattern
  to its AArch64-defined form with a byte-exact encoder test *and* a numeric test,
  rather than reasoning about signs in prose. (§5)
- **Phase 3 sequencing vs plan-01 — RESOLVED.** plan-01 FP residency and plan-17
  finiteness-at-boundaries have both landed, so Phase 3 is unblocked with the full
  win (no fallback `fmov`-per-operand path needed). The only remaining sequencing
  constraint is internal: Phase 2 (encoders + dormant recognizer) before Phase 3
  (wiring). (§2.1)

## Non-Goals

- Kernel rewrites beyond FMA straggler fixes (degree reduction, table-based
  reduction, new reductions) — separate work.
- Any `Fixed` change.
- Linking libm; using x87 transcendentals on a future x86 backend (inaccurate and
  drags in 80-bit intermediates) — owned software kernels remain the x86
  transcendental path too.
- Auto-vectorizing scalar user loops or SIMD-widening scalar `Float` code.
- Reworking the inf/NaN check itself (that is plan-01 Phase 1); this plan only
  *reduces the count* of checks as a side effect of fusing two ops into one.

## Summary

The surprise this plan encodes: native `Float` is already almost entirely on
hardware instructions — scalar `sqrt`/`round`/`floor`/`ceil`/`neg`, all array
paths, and the transcendental kernels (already FMA-fused) — so the only real
remaining hardware-instruction win is **fusing user-level scalar `a*b±c` into
`fmadd` and friends**, plus trivial `min`/`max`/`abs` mop-up. The engineering risk
is concentrated entirely in Phase 3, which changes last-bit results (toward
correctly-rounded truth) and resolves whether an internal product traps on
overflow; it lands last, depends on plan-01's FP residency, and is gated on a ULP
non-regression proof and explicit, justified golden regeneration. Nothing here links
libm, touches the deterministic bit-identical `Fixed` path, or changes any value's
memory layout or ABI.
