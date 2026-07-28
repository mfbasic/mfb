# MFBASIC Transcendental Kernel Performance Plan

Last updated: 2026-06-29

> **Status (2026-06-29). ALL phases resolved.** Phases 1–9 + 12 landed; Phases
> 10–11 (`log`/`log10`) investigated and have no safe ≤1 ULP-preserving win.
> - **Phase 1 `pow` — ✅ DONE** (`ab99430b`): register-resident working set;
>   math-explog 13.4×→4.3× c-O2; 100% ≤1 ULP. The dominant win.
> - **Phase 2 SIMD kernel constant pool — ✅ DONE** (`353bd502`): shared
>   infrastructure across all SIMD kernels; math-invtrig 4.3×→2.7×, math-trig
>   6.9×→6.0×; accuracy-neutral (outputs byte-identical). *(This replaced the
>   original plan's "Phase 2 = tan"; the per-kernel work shifted to Phases 3+.)*
> - **Phases 3–5 `tan`/`cos`/`sin` — ✅ DONE** (`caa8c12`, `75d0f58`): a single
>   scalar lane branches on the reduced quadrant and evaluates only the one
>   double-double Horner the result selects (the 2-lane array body must compute
>   both because its lanes can disagree). `tan` branches on bit0 (period π; bit1
>   cancels in the ratio, bit-identically). **Bit-identical** to the array path
>   (proven by 10,001-point scalar-vs-array sweeps, 0 mismatches at 80 decimals),
>   so still strict ≤1 ULP. **math-trig 189.9→137.7 ms (5.84×→4.33× c-O2, −27%).**
> - **Phases 6–9 `acos`/`asin`/`atan`/`atan2` — ✅ DONE** (`caa8c12`): the shared
>   atan core's 12 per-call segment selects collapsed from an `orr;bsl;orr` triple
>   to a single `BIT acc,val,mask`. Bit-identical; −24 instructions/atan call.
>   **math-invtrig 70.4→64.5 ms (2.84×→2.63× c-O2, −8%).**
> - **Phase 12 `exp` — ✅ DONE** (`81c2578`): `n = floor(x/ln2 + 0.5)`'s `fdiv`
>   became a reciprocal multiply `x*(1/ln2)` (the Cody-Waite `n*ln2_hi/lo`
>   subtraction keeps the reduction exact). 100% ≤1 ULP vs truth on 738 vectors;
>   acceptance goldens byte-identical. ~5% faster on an isolated exp micro-bench
>   (lost in pow-dominated math-explog). Also extended `runtime_ulp.py` to gate
>   exp/log/log10.
> - **Phases 10–11 `log`/`log10` — ☑ NO SAFE WIN (investigated, closed).** Both
>   are single-path minimax kernels (no "compute-both-select-one" structure to
>   skip) and their only `fdiv` is the genuine ratio `s=(m-1)/(m+1)` — not a
>   constant reciprocal, so not replaceable by a multiply the way `exp`'s was.
>   The polynomials are minimax-optimal at ≤1 ULP (cannot drop a term). The
>   remaining gap in math-explog is overwhelmingly `pow` (Phase 1, done); the
>   log/log10 slice is small and has no accuracy-preserving lever left.
> - **Constraint discovered in Phases 1–2 (held throughout):** the kernel
>   **polynomials are already minimax-optimal at the ≤1 ULP boundary**
>   (`runtime_ulp.py` maxULP=1). So Phases 3–12 **could not reduce polynomial
>   degree**. The wins came from the other two levers the plan named: a dedicated
>   **scalar path** (the trig branch trick, Phases 3–5) and cheaper **argument
>   reduction** (exp's reciprocal multiply, Phase 12), plus a one-instruction
>   select for the atan core (Phases 6–9).

MFBASIC computes every transcendental (`exp`/`log`/`sin`/`cos`/`pow`/…) from its own
software kernels — no libm (`plan-01-libm-kernels`). After plan-17 removed the per-op
finiteness checks, the math benchmarks are dominated entirely by these kernels, and
they are well behind libm: math-explog 13.4×, math-trig 6.9×, math-invtrig 4.3× vs
`c -O2`. The single behavioral outcome a correct implementation produces: **each
transcendental kernel runs materially faster while staying ≤1 ULP** (the accuracy
contract is non-negotiable), proven per kernel by `tools/math-kernels/runtime_ulp.py`,
with each accuracy-affecting change re-validated and any legitimately-changed golden
regenerated and justified.

It complements:

- `./mfb spec architecture math-kernels` (`src/docs/spec/architecture/17_math-kernels.md`
  — the ≤1 ULP contract every phase must hold; canonical specs under `src/docs/spec/**`)
- `tools/math-kernels/runtime_ulp.py` (the per-kernel ULP gate)

## 1. Goal

- Speed up the owned transcendental kernels, **one kernel per phase**, ordered by
  measured impact, holding ≤1 ULP throughout.
- No new platform math import — the kernels stay owned software (consistent with
  `plan-01-libm-kernels` and plan-02's non-goal).

### Non-goals (explicit constraints)

- **Accuracy must not regress.** Every changed kernel stays ≤1 ULP and no worse than
  today on the `runtime_ulp.py` sweep; if any input regresses, that change is not
  applied. This gates every phase.
- **No libm, no x87.** Owned kernels remain the transcendental path on every target.
- **No change to the finiteness rule (plan-17), the float value model (plan-01-dnative),
  or FMA fusion (plan-02).** Those are separate; the kernels already use FMA.
- **No change to error semantics** — a kernel's domain/overflow errors keep their
  current codes and trigger points (`ErrFloatDomain` for `log`/`asin`/`acos` domain,
  `ErrFloatOverflow`/`Nan` at the boundary).

## 2. Current State — measured per-kernel cost

Micro-benchmark, 2,000,000 scalar calls each, loop baseline 11.0 ms subtracted
(`tools`/ad-hoc; reproduce before each phase). The ranking is the phase order:

| # | kernel | 2M ms | −baseline | where it dominates |
|---|--------|-------|-----------|--------------------|
| 1 | **pow**  | 367.7 | **~357** | math-explog (≈83% of it); ~40× libm — the outlier |
| 2 | tan    |  77.0 | ~66 | math-trig |
| 3 | cos    |  69.1 | ~58 | math-trig |
| 4 | sin    |  61.4 | ~50 | math-trig |
| 5 | acos   |  41.8 | ~31 | math-invtrig |
| 6 | asin   |  41.0 | ~30 | math-invtrig |
| 7 | atan   |  36.8 | ~26 | math-invtrig (base for atan2) |
| 8 | atan2  |  40.4 | ~29 | math-trig (reuses atan) |
| 9 | log10  |  37.6 | ~27 | math-explog |
| 10| log    |  35.5 | ~25 | math-explog |
| 11| exp    |  29.9 | ~19 | math-explog |

Kernel locations: `FloatKernel` (`Exp`/`Log`/`Log10`/`Sin`/`Cos`/`Tan`/`Atan`/`Asin`/
`Acos`) in `src/target/shared/code/builder_simd_float_math.rs` (SIMD bodies, run
single-lane for scalar `math::`); **`pow` is separate and scalar** in
`src/target/shared/code/builder_pow.rs` (fdlibm log2-space, 713 lines).

**Shared infrastructure** (optimize once in the first phase of a group; siblings
inherit): `sin`/`cos`/`tan` share the π/2 argument reduction; `asin`/`acos` share
their core; `log`/`log10` share the mantissa/exponent split (`log10 = log·log10(e)`);
`atan2` is built on `atan`.

### 2.1 The `pow` lead (Phase 1)

`pow` is ~40× libm and ~10× the next kernel. Its `builder_pow.rs` helpers `pld`/`pst`
(`pow-load`/`pow-store`) move **every f64 intermediate to/from a stack slot** —
the whole fdlibm algorithm runs through memory, not registers. The first lever is
almost certainly *register-residency of the working set* (the fdlibm pow working set
is bounded and should fit in `d`-registers), not an algorithm change — a contained,
accuracy-neutral rewrite. Profile to confirm before touching the polynomials.

## 3. Design Overview

Each phase is one kernel: **(a) profile** the current body (instruction histogram of
its hot path; is the cost memory traffic like pow, polynomial degree, the argument
reduction, or single-lane SIMD-setup overhead for the scalar path?), **(b) optimize**
the identified bottleneck (residency, lower-degree-but-still-≤1-ULP polynomial,
cheaper/shared reduction, a dedicated scalar path if the 1-lane SIMD setup dominates),
**(c) prove** ≤1 ULP and no per-input regression on `runtime_ulp.py`, **(d) measure**
the 2M-call micro-benchmark and the owning benchmark. Lowest-risk, highest-impact
first; accuracy is the gate on every phase.

## Layout / ABI Impact

None. Kernels are internal routines; no value layout, ABI, or error-code change. Math
benchmark `.run` goldens change only where a (now-equal-or-better, ≤1 ULP) last bit
legitimately shifts — regenerated per phase with the ULP justification; most kernels
should be byte-stable.

## Phases

Each phase: profile → optimize → `runtime_ulp.py` ≤1 ULP → measure.

1. **`pow` — ✅ DONE** (`ab99430b`). Register-resident the `pld`/`pst` stack-slot
   working set (§2.1) — accuracy-neutral, no polynomial change. math-explog
   13.4×→4.3× c-O2 (496→162 ms); `runtime_ulp.py pow` 100% ≤1 ULP. The biggest
   single win in the suite.
2. **SIMD math-kernel constant pool — ✅ DONE** (`353bd502`). Shared infrastructure,
   not a single kernel: the per-call constant materialization in the SIMD kernel
   bodies became a shared constant pool, helping every `Sin`/`Cos`/`Tan`/`Exp`/`Log`/
   `Atan`/`Asin`/`Acos` body at once (math-invtrig 4.3×→2.7×, math-trig 6.9×→6.0×).
   Accuracy-neutral — benchmark outputs byte-identical.
3. **`tan` — ✅ DONE** (`75d0f58`). Scalar branch on bit0 (period π; bit1 cancels
   in the sin/cos ratio bit-identically) drops the branchless sin_full/cos_full
   quadrant-selection block; both Horners still run. math-trig 138.9→132.9 ms.
4. **`cos` — ✅ DONE** (`caa8c12`). Scalar quadrant branch: evaluate only the one
   double-double Horner the result selects (array body still computes both).
5. **`sin` — ✅ DONE** (`caa8c12`). Same scalar quadrant branch. sin+cos together:
   math-trig 189.9→138.9 ms; both **bit-identical** to the array path.
6. **`acos` — ✅ DONE** (`caa8c12`). atan core selects with one `BIT` instruction.
7. **`asin` — ✅ DONE** (`caa8c12`). Reuses the atan core.
8. **`atan` — ✅ DONE** (`caa8c12`). The `BIT` select; −24 instructions/call.
9. **`atan2` — ✅ DONE** (`caa8c12`). Reuses the atan core. math-invtrig 70.4→64.5 ms.
10. **`log10` — ☑ NO SAFE WIN.** Single-path minimax; only `fdiv` is the genuine
    ratio `(m-1)/(m+1)` (not a constant reciprocal). No degree headroom. Closed.
11. **`log` — ☑ NO SAFE WIN.** Same as Phase 10. Closed.
12. **`exp` — ✅ DONE** (`81c2578`). `x/ln2` divide → `x*(1/ln2)` reciprocal
    multiply (Cody-Waite keeps the reduction exact). 100% ≤1 ULP; ~5% faster on
    an isolated exp micro-bench (lost in pow-dominated math-explog).

(Phases 10–12 are the math-explog *remainder* after `pow`; small individually, but
together they are the rest of that benchmark's gap. Re-measure the §2 table before
each phase — earlier shared-infrastructure work shifts the siblings' numbers. The
biggest remaining lever is the trig group, Phases 3–5: math-trig is still 6.0× c-O2.)

## Validation Plan

- **ULP gate (every phase):** `tools/math-kernels/runtime_ulp.py` for the changed
  kernel — ≤1 ULP and no per-input regression vs. the prior build. Non-negotiable.
- **Function tests:** existing `tests/func_math_<fn>_valid/_invalid` stay green;
  add a `_valid` case if a new edge (e.g. a reduced-range boundary) is introduced.
- **Speed proof:** the 2M-call micro-benchmark for the kernel + the owning benchmark
  (`math-explog`/`math-trig`/`math-invtrig`) median, before/after, in the commit.
- **Doc sync:** if a kernel's algorithm/accuracy notes in
  `src/docs/spec/architecture/17_math-kernels.md` change, update them.
- **Acceptance:** `scripts/test-accept.sh` green after each phase; regenerate only
  the math `.run` goldens whose value legitimately moved (≤1 ULP), justified.

## Open Decisions

- **Scalar path vs. single-lane SIMD.** The scalar `math::` calls run the SIMD kernel
  bodies one lane wide; if profiling shows the vector setup/teardown dominates a
  scalar call, a dedicated scalar kernel may beat the 1-lane SIMD path. Decide per
  kernel in its profile step (§3). (Recommend: measure first; only fork the path if
  the setup is a real fraction.)
- **`pow` algorithm vs. residency.** Recommend residency-only first (accuracy-neutral,
  likely most of the win); revisit the polynomial degree only if still short of libm
  and ≤1 ULP headroom exists. (§2.1)

## Non-Goals

- FMA fusion of user `a*b±c` (plan-02) and the float value model (plan-01-dnative) —
  separate; the kernels already use FMA internally.
- Any accuracy relaxation below the ≤1 ULP contract to buy speed.
- Linking libm or using x87 transcendentals.

## Summary

The work is mechanically uniform — profile, optimize the one real bottleneck, prove
≤1 ULP, measure — repeated per kernel in measured-impact order. The order is set by
data, not guesswork: `pow` is a true outlier (~40× libm, ~83% of the worst benchmark)
and its lead is concrete (a stack-slot working set that should be register-resident),
so it lands first and alone could roughly halve math-explog; the trig and inverse-trig
families follow, each phase reusing the shared reduction the first sibling reworks;
the cheap `log`/`log10`/`exp` remainder is last. Accuracy (≤1 ULP) gates every phase.
