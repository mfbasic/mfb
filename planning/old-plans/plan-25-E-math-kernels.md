# plan-25-E: Transcendental kernel throughput (Goal 2)

Last updated: 2026-07-14
Effort: large (3h–1d) — but see status: most of it is already done or obsolete.

> **CLOSED (2026-07-14).** Superseded by `planning/plan-39-benchmark-perf.md`
> sub-plan **B** (transcendental + float kernels), which covers the whole math band
> — including `pow` — and explicitly folds in this plan's lone remaining lever:
> "Overlaps `planning/plan-25-E-math-kernels.md` (pow unroll) — fold that in"
> (`plan-39-benchmark-perf.md:221-222`). Of the four original levers, E1 was
> superseded (scalar bit0 branch), E3 is obsolete (4-segment fdlibm atan), E4 is
> DONE, and E2's home-reuse half landed via `PowHomes`. The only survivor — the E2
> pow-array-loop unroll — is subsumed by plan-39-B's stronger B1 fix (emit each
> scalar transcendental as one shared out-of-line leaf, attacking the per-call
> setup re-emitted 2M× rather than just unrolling the array loop). No live work
> remains here; track it under plan-39-B. Kept for historical context.

> **Status (2026-07-12 re-review).** The kernels have moved a lot since this plan
> was written; three of the four levers are overtaken by events. Summary:
>
> | Lever | Plan intent | Reality now |
> |-------|-------------|-------------|
> | **E1** tan single-lane divide | replace the dd quotient with `fdiv`+FMA | **superseded** — the scalar tan path already halves the work a *different, accuracy-preserving* way: `emit_tan_body_scalar` branches on bit0 and drops the branchless quadrant selection, but **keeps** the dd `emit_tan_divide` (plan-03 Phase 3, 75d0f58f). The `fdiv`+FMA swap was never applied and would trade away ULP margin the branch already recovered. |
> | **E2** pow home reuse / unroll | reuse FP homes + unroll the array loop | **half done** — `PowHomes` (builder_pow.rs) already keeps the fdlibm body's values in registers, so the "resetting ~19 FP homes per element" premise is gone. Only the *loop unroll* (2–4×) remains. |
> | **E3** atan degree-12 | regenerate a shorter `ATAN_COEFFS` Horner | **obsolete** — atan was rewritten as a 4-segment fdlibm atan (plan-01-simd Phase 5, bc15c2f5) with segmented range reduction + the `ATAN_AT` split polynomial. The `ATAN_COEFFS[19]` array this lever targets is now **dead code** (declared, never referenced). |
> | **E4** sin/cos scalar single-poly | one poly chain per quadrant in the scalar path | **DONE** — `emit_sin_cos_body_scalar` (plan-03 Phase 3, 75d0f58f) branches on bit0 and runs only the selected polynomial. |
>
> Net remaining work: **only the E2 pow-loop unroll** is a live, unstarted lever.
> Everything else has either landed or been replaced by a superior approach under
> plan-03 / plan-01-simd. See `planning/old-plans/plan-03-transcendental-kernels.md`
> (memory: "ALL PHASES RESOLVED; scalar quadrant branch runs ONE dd-Horner for
> sin/cos/tan"). Before doing anything here, re-measure the benchmark — the median
> table below predates the scalar-path landing and is stale.

Goal 2 of the benchmark — "math within ~2 ms of C `-O0`" — is met only by `sqrt`
today. The software transcendental kernels run 2–8× C‑O0. This sub-plan lowers
their per-call instruction count without breaking the ≤1 ULP accuracy contract.
`tan` (72 ms) and `pow` (94 ms) are the outliers and get dedicated attention.

It complements:

- `./mfb spec package math` (the accuracy contract these kernels satisfy).
- `planning/old-plans/plan-03-transcendental-kernels.md`,
  `plan-01-libm-kernels.md` (the existing minimax/double-double kernels — E tunes
  them, it does not rewrite the math).
- `tools/math-kernels/` (coefficient generation + `runtime_ulp.py` accuracy gate).

## 1. Goal

- Reduce per-call cost of `tan`, `pow`, `atan`/`asin`/`acos`/`atan2`, `sin`/`cos`,
  `log`/`log10`, `exp` while keeping every kernel within its current ULP budget
  (validated by `tools/math-kernels/runtime_ulp.py` and the array-vs-scalar
  bit-identity harness).
- Move `tan` and `pow` from ~8–9× C‑O0 toward ~3–4×; move the others toward
  ~2–3×.

### Non-goals (explicit constraints)

- **No accuracy regression.** Every kernel must still pass `runtime_ulp.py` at its
  current tolerance and stay bit-identical between the scalar and array code paths
  (the standing contract from plan-03). A degree reduction that busts ULP is
  reverted.
- No change to `Float` carrier representation, the d-native calling convention, or
  IEEE result semantics (finiteness observation from plan-17 stays).

## 2. Current State

All scalar transcendentals are emitted **inline** at each call site via the SIMD
float kernels (`src/target/shared/code/builder_simd_float_math.rs`,
`builder_math.rs`, `builder_pow.rs`). The array overloads run a 2-lane `.2d` body;
scalar calls go through `lower_simd_float_scalar` (`:452`), which now dispatches
sin/cos/tan to dedicated single-lane bodies. Key facts (line refs current as of
2026-07-12):

- **tan** — array body `emit_tan_body` (`:826-869`) computes sin and cos each as
  full **double-doubles**, applies branchless quadrant selection to both halves,
  then a one-step double-double-accurate divide (`emit_tan_divide` `:874-884`).
  The **scalar** body `emit_tan_body_scalar` (`:893-927`) already exploits tan's
  π-period: it branches on bit0, picks the (num,den) dd pair directly, and drops
  the whole branchless quadrant-selection block — *the E1-class win, already
  landed* (plan-03 Phase 3). It still calls the dd `emit_tan_divide` for accuracy.
- **pow** — scalar `emit_pow_scalar` (`builder_pow.rs:154`); array `lower_pow_array`
  (`:373`). Per-element fdlibm log2 dd decomposition (`emit_pow_log2` `:548`) +
  exp2 scaling (`emit_pow_exp2` `:704`) + integer-exponent sign rule. The fdlibm
  body's values now live in a `PowHomes` register-home struct (`:68`) — the
  per-op load/store the plan flagged is **gone**. But `lower_pow_array` still
  loops **one element at a time** (loop `:446-488`); it is **not unrolled**.
- **atan** — `emit_atan_core` (`:607`) is now a **4-segment fdlibm atan**
  (`ATAN_SEG_THRESH` range reduction into segments 0–3, then the `ATAN_AT` split
  odd/even polynomial `:670-693`), strict ≤1 ULP (plan-01-simd Phase 5). The old
  single degree-18 `ATAN_COEFFS[19]` is **dead code** (`simd_kernel_coeffs.rs:81`,
  declared but unreferenced). `asin`/`acos`/`atan2` all route through this core.
- **sin/cos** — array `emit_sin_cos_body` (`:720`) computes both compensated
  Horner chains (its two lanes may disagree). The **scalar** `emit_sin_cos_body_scalar`
  (`:758`) branches on bit0 and runs only the selected polynomial — *the E4 win,
  already landed* (plan-03 Phase 3).
- **log/log10** — `emit_log_body` (`:978`): double-double compensated Horner.
- **exp** — `emit_exp_body` (`:930`): single-precision Horner.
- **sqrt**: hardware `fsqrt` — already at the bar (8.6 vs 7.3).

## 3. Design Overview

> Levers as originally scoped, below. See the status table at the top: only E2's
> loop unroll is still live — E4 shipped, E1 was superseded, E3 is obsolete.

Four independent tuning levers, each behind the ULP gate:

- **E1 — tan single-lane divide.** Replace the double-double quotient with a
  scalar `fdiv` + one FMA correction (`q=sh/ch; r=sh−q*ch; tan=q+r/ch`). The
  3-part Cody-Waite reduction already keeps arguments away from poles, so ~0.5 ULP
  of extra error is affordable. Removes one division + several FMAs.
- **E2 — pow loop unroll / home reuse.** Unroll the array loop 2–4× and reuse the
  FP register homes across iterations to hide latency and stop resetting the
  register file per element. The fdlibm body itself stays scalar (data-dependent
  bit ops don't vectorize cleanly).
- **E3 — atan degree reduction.** Regenerate `ATAN_COEFFS` as a degree-12 minimax
  (`tools/math-kernels/gen_coeffs.py`), shortening the Horner chain from 19 to 13
  terms. Benefits atan, asin, acos, atan2 together.
- **E4 — sin/cos scalar single-poly.** In the scalar (1-lane) path, compute only
  the polynomial actually selected by the quadrant rather than both sin and cos
  chains.

Correctness risk is concentrated in E1 and E3 (accuracy). Both are gated by
`runtime_ulp.py`; if a change busts the budget it is reverted and the coefficient
degree bumped back.

## Phases

### Phase 1 — E3: atan degree-12 — **OBSOLETE, do not do**

Superseded by the 4-segment fdlibm atan (plan-01-simd Phase 5, bc15c2f5). The
target `ATAN_COEFFS[19]` is now dead code; the live kernel uses segmented range
reduction + the `ATAN_AT` split polynomial and is already ≤1 ULP with a much
shorter effective chain. There is no degree-18 Horner left to shorten.

- Optional cleanup (not part of this plan's goal): delete the unreferenced
  `ATAN_COEFFS` const in `simd_kernel_coeffs.rs:81` (a `#[allow(dead_code)]` or a
  compiler warning may already flag it).

Status: OBSOLETE (goal met by a different kernel).

### Phase 2 — E1: tan single-lane divide — **SUPERSEDED, do not do**

The intended win (halve tan's scalar cost) already landed a better way:
`emit_tan_body_scalar` (`builder_simd_float_math.rs:893`) branches on bit0 and
drops the branchless quadrant-selection block, keeping the dd `emit_tan_divide`
so ≤1 ULP is preserved (plan-03 Phase 3, 75d0f58f). Do **not** replace
`emit_tan_divide` with `fdiv`+FMA — it would spend the ULP margin the scalar
branch was designed to protect, for a marginal instruction saving.

Status: SUPERSEDED (scalar bit0 branch shipped instead; accuracy-preserving).

### Phase 3 — E4: sin/cos scalar single-poly — **DONE**

Landed as `emit_sin_cos_body_scalar` (`builder_simd_float_math.rs:758`), reached
via `lower_simd_float_scalar` (`:452`): the scalar path branches on bit0 and runs
only the selected polynomial; the 2-lane array `emit_sin_cos_body` (`:720`) is
unchanged. ULP + scalar-vs-array bit identity hold.

Status: DONE (plan-03 Phase 3, 75d0f58f).

### Phase 4 — E2: pow array unroll — **ONLY REMAINING LIVE LEVER**

Home reuse already landed: `PowHomes` (`builder_pow.rs:68`) keeps the fdlibm
body's values in registers, so the per-op load/store is gone. What remains:

- [ ] Unroll the `lower_pow_array` element loop (`builder_pow.rs:446-488`) 2–4×
      to overlap the log2→exp2 dependency chains across iterations; measure FP
      register pressure (the body already homes ~19 values, so unrolling shares
      one `PowHomes` set and may spill — validate before committing).
- [ ] Re-measure the pow benchmark median (the ~94 ms / ~55 ms figures predate
      the `PowHomes` landing and are stale).
- [ ] Validate pow ULP; scalar-vs-array bit identity.

Acceptance: pow median drops measurably vs the current (post-`PowHomes`) baseline;
ULP gate green. Weigh the gain against register-pressure risk before landing.
Commit: —

## Layout / ABI Impact

None. Kernels are inline codegen; no layout, ABI, or `Float` representation change.
`mfb spec package math` accuracy statements stay true (ULP budget unchanged).

## Validation Plan

- Accuracy gate: `tools/math-kernels/runtime_ulp.py` for every touched function at
  its current tolerance; scalar-vs-array bit-identity harness.
- Function tests: `tests/func_math_*` for each op, valid/invalid.
- Whole-benchmark: re-run the math group vs C‑O0.
- Acceptance: `scripts/test-accept.sh` (math goldens re-blessed only where the
  kernel bytes change; ULP dumps prove accuracy held).

## Theorized gains (median) — **STALE, pre-dates the scalar-path landing**

> These numbers were measured before plan-03 Phase 3 (scalar sin/cos/tan) and the
> `PowHomes` home-reuse landed, so the "now (ms)" column overstates current cost
> for tan/sin/cos and the E1/E3/E4 drivers no longer describe the shipped code.
> Kept only for historical context — re-measure before acting.

| op    | now (ms) | c‑O0 | driver             | after (ms) | Δ    |
|-------|---------:|-----:|--------------------|-----------:|-----:|
| tan   |   71.6   | 9.1  | E1 single divide   |  ~40       | −44% |
| pow   |   94.3   | 17.7 | E2 unroll/home reuse|  ~55       | −42% |
| atan  |   23.6   | 7.7  | E3 degree-12       |  ~14       | −41% |
| asin  |   28.2   | 9.8  | E3                 |  ~17       | −40% |
| acos  |   28.7   | 8.5  | E3                 |  ~17       | −41% |
| atan2 |   30.0   | 13.6 | E3                 |  ~19       | −37% |
| sin   |   32.3   | 7.8  | E4 single poly     |  ~20       | −38% |
| cos   |   32.2   | 7.6  | E4                 |  ~20       | −38% |
| log   |   31.9   | 7.5  | (Horner trim, opt) |  ~24       | −25% |
| log10 |   33.4   | 7.6  | (as log)           |  ~25       | −25% |
| exp   |   18.8   | 7.7  | (Horner trim, opt) |  ~14       | −26% |
| sqrt  |    8.6   | 7.3  | already at bar     |   8.6      |   0% |

These narrow but do **not** fully close Goal 2 to ±2 ms for the hard ops (tan,
pow, atan2 remain > 2 ms over C‑O0). Reaching the full bar would need either
vectorized batch evaluation of the benchmark's array inputs or a further kernel
redesign — recorded as an open decision below.

## Open Decisions

- **Full Goal-2 closure vs. accuracy** — accept the narrowed gap (recommended: the
  ≤1 ULP contract is worth more than the last few ms), or pursue a lower-accuracy
  "fast math" kernel variant behind a flag (rejected: adds a semantic surface).
- **pow vectorization** — unroll-only (recommended, low risk) vs. a true 2-lane
  fdlibm (large, data-dependent-branch heavy).

## Summary

E was a tuning pass on already-correct kernels, gated at every step by the ULP
harness. As of the 2026-07-12 re-review it is **largely overtaken by events**:
E4 (sin/cos scalar single-poly) and the E1-class tan win both shipped under
plan-03 Phase 3 via dedicated scalar bodies; E3 (atan degree reduction) is
obsolete because atan was rewritten as a 4-segment fdlibm kernel (plan-01-simd
Phase 5) and its target coeff array is now dead code; and E2's home-reuse half
landed via `PowHomes`. The **only live, unstarted lever is the E2 pow-loop
unroll** — and even that should be re-justified against a fresh benchmark and the
register-pressure risk before committing. The plan does not reach the ±2 ms bar
for the hardest ops (tan, pow, atan2); that residual remains called out rather
than hidden.
