# bug-68: SIMD float array min/max/clamp odd-tail lane diverges in sign-of-zero from the vector body, and the array binary-float driver hardcodes a NaN-only error reduce (latent Inf drop) plus stale commented code

Last updated: 2026-07-09
Effort: small (<1h)

A cluster of LOW-severity defects in the SIMD/vector math codegen, batched (adjacent files,
same subsystem).

**(1) Float array min/max/clamp odd-tail sign-of-zero divergence (correctness).** The
2-lane vector body uses `fmin`/`fmax` (IEEE, orders `-0 < +0`), but the odd-length scalar
tail computes `d2 = d0 - d1` then `fcmp d2, #0` + `branch_le` — which cannot distinguish
`+0.0` from `-0.0` on a tie. So `math::min` of a final `(+0.0, -0.0)` pair yields `+0.0` in
the tail lane where every body lane (and the scalar `math::min(Float, Float)` overload,
which uses `fminnm`) yields `-0.0`. Observable if the result feeds `1.0/x` (`-inf` vs
`+inf`). Contradicts the plan-01-simd invariant that the odd tail lane is bit-identical to a
body lane.

**(2) Array binary-float driver hardcodes a NaN-only error reduce (latent dead-code).**
`lower_simd_float_binary` reduces only `FloatError::Nan`, while the scalar sibling
`lower_simd_float_binary_scalar` correctly loops `kernel.errors()`. `Pow::errors()` declares
`[Nan, Inf]`; if `Pow` were ever routed through the array driver, overflow would never be
reduced and `math::pow` array overflow would silently return a list containing `inf` instead
of raising `ErrFloatInf`. Latent today because array `pow` is diverted to `lower_pow_array`
and never reaches this driver; the per-iteration `v24` re-zeroing compounds the unsafety for
any future Inf-raising kernel wired here.

**(3) Stale commented-out code + docstrings (doc).** `builder_simd_float_math.rs:1327-1330`
is a commented-out duplicate of the four lines above it; docstrings at `:1157`/`:1258` claim
the array driver handles "atan2/pow" though pow never reaches it.

The single correct behavior a fix produces: (1) the float tail uses `fminnm`/`fmaxnm` so it
is bit-identical to the body (sign-of-zero included); (2) the array driver iterates
`kernel.errors()` like the scalar path (or asserts it is atan2-only); (3) the dead comments
are removed.

References (all under `src/target/shared/code/`):

- `builder_simd_math.rs:emit_simd_binary_scalar` (MinFloat/MaxFloat, ~`:605-616`) and
  `emit_simd_clamp_scalar` (Float, ~`:793-812`): the subtract-then-compare tail. Reached via
  `math::min`/`max`/`clamp` array overloads (`builder_math.rs:572-576`, `:624-625`).
  `fminnm`/`fmaxnm` now available per plan-02.
- `builder_simd_float_math.rs:lower_simd_float_binary` reduce (`:1258-1260`) vs
  `lower_simd_float_binary_scalar` (`:1288`, correct loop over `kernel.errors()`);
  `FloatBinaryKernel::Pow::errors()` (`:1367-1371`); array pow diversion
  (`builder_math.rs:325-326`). Dead comments `:1327-1330`; stale docstrings `:1157`, `:1258`.
- Invariant: plan-01-simd Open-Decision-#6 (odd tail lane bit-identical to a body lane);
  plan-02 float hardware `fminnm`/`fmaxnm`.
- Found during the goal-01 compiler source review of `src/target/shared/code/`.

## Failing Reproduction

(1)
```
IMPORT io
IMPORT math
FUNC main AS Integer
  LET a AS List OF Float = [1.0, 0.0]       ' odd tail after the first 2-lane body
  LET b AS List OF Float = [2.0, -0.0]
  LET m AS List OF Float = math::min(a, b)
  ' inspect the last element's sign: 1.0/m[1] is -inf if -0.0, +inf if +0.0
  io::print(toString(1.0 / m[1]))
  RETURN 0
END FUNC
```
(Use a genuinely odd-length list so the final pair falls in the scalar tail.)

- Observed: `+inf` (tail kept `+0.0`), diverging from a body lane / the scalar overload.
- Expected: `-inf` (`-0.0`, matching `fmin`/`fminnm`).

(2)/(3) latent / cosmetic — no runtime trigger today (array pow is diverted).

Contrast: non-zero finite pairs order correctly in the tail (the subtraction's sign, even
when it overflows to `±inf`, preserves ordering); Integer/Fixed min/max tails use integer
compares and match their bodies exactly; the scalar `math::min(Float, Float)` overload uses
`fminnm` and is correct.

## Root Cause

(1) The float tail emits `fsub` + `fcmp #0` + conditional branch, which treats `+0.0` and
`-0.0` as equal on a tie, whereas the body's `fmin`/`fmax` are sign-of-zero aware. (2) The
array driver bakes in "the only array binary caller is atan2, which fails only with Nan" and
hardcodes the reduce accordingly. (3) Leftover from the pow-array reroute.

## Goal

- (1) The float array min/max/clamp odd tail is bit-identical to a body lane, including
  signed zeros.
- (2) The array binary driver reduces every error its kernel declares (or asserts atan2-only).
- (3) No dead commented code / stale docstrings in the driver.

### Non-goals (must NOT change)

- Integer/Fixed min/max/clamp tails (already correct).
- The array `pow` diversion to `lower_pow_array`.
- Non-zero finite min/max results.

## Blast Radius

- `emit_simd_binary_scalar` (MinFloat/MaxFloat) + `emit_simd_clamp_scalar` (Float) — item (1).
- `lower_simd_float_binary` reduce — item (2).
- The commented block + docstrings — item (3).

## Fix Design

(1) Emit `fminnm`/`fmaxnm` in the float tail (plan-02 makes them available), matching both
the vector body and the scalar overload. (2) Replace the hardcoded
`emit_float_error_reduce(Nan)` with `for err in kernel.errors() { … }` (mirroring the scalar
path); if Pow is ever routed here, hoist the `v24` zero out of the per-iteration body — until
then, at minimum assert the driver is atan2-only. (3) Delete `:1327-1330` and correct the
docstrings.

## Phases

### Phase 1 — failing test

- [x] Add the odd-length signed-zero min/max/clamp test asserting the tail matches the body
      (fails today). Add a debug assertion (or test) that the array binary driver is
      atan2-only.

### Phase 2 — the fixes

- [x] `fminnm`/`fmaxnm` tail; iterate `kernel.errors()` (or assert atan2-only); delete the
      dead comments / fix docstrings.

### Phase 3 — validation

- [x] Regenerate SIMD goldens (delta = the float tail encoders); ULP/no-regression checks.
      `scripts/artifact-gate.sh`, `scripts/test-accept.sh`. (Runtime proof done here; the
      full artifact-gate / test-accept golden regeneration is run by the orchestrator.)

## Validation Plan

- Regression test(s): the signed-zero tail test; the atan2-only assertion.
- Runtime proof: `1.0 / math::min([...,+0.0],[...,-0.0])[last]` is `-inf`.
- Doc sync: correct the array-driver docstrings.
- Full suite: `scripts/artifact-gate.sh`, `scripts/test-accept.sh`.

## Summary

The float SIMD min/max/clamp odd tail uses subtract-compare, which loses sign-of-zero versus
the `fmin`/`fmax` body — a real (LOW) divergence reachable via odd-length Float arrays. The
array binary driver's NaN-only reduce and stale comments are latent/cosmetic. Fixes: use
`fminnm`/`fmaxnm` in the tail, iterate the kernel's declared errors, and delete the dead code.

## Resolution

Fixed 2026-07-10. IEEE-754 semantics chosen: **sign-of-zero-aware minimum/maximum** (ARM
`fminnm`/`fmaxnm`, i.e. the IEEE-754-2019 `minimumNumber`/`maximumNumber` flavor:
`min(+0,-0) = -0`, `max(-0,+0) = +0`). This is exactly what the 2-lane vector body
(`vector_fmin`/`vector_fmax`) and the scalar `math::min/max(Float,Float)` overload
(`float_min_d`/`float_max_d`) already use. `fmin`/`fmax` and `fminnm`/`fmaxnm` differ only on
NaN, and a `List OF Float` cannot hold a NaN/Inf (rejected at the float finiteness boundary),
so `fminnm`/`fmaxnm` in the tail is bit-identical to the `fmin`/`fmax` body for every
reachable input — satisfying the spec invariant "the array result matches the scalar result
element-wise" (`src/docs/spec/language/18_builtin-functions.md`), which the tail previously
violated on a `±0.0` tie.

Items:

1. **Sign-of-zero tail (correctness).** `emit_simd_binary_scalar` (MinFloat/MaxFloat) and
   `emit_simd_clamp_scalar` (Float) in `builder_simd_math.rs` now emit `float_min_d`/
   `float_max_d` (`fminnm`/`fmaxnm`) instead of `fsub`+`fcmp #0`+conditional branch. The old
   subtract-compare treated `+0.0`/`-0.0` as equal on a tie and kept the left/wrong-signed
   zero.
2. **Array binary NaN-only reduce (latent).** `lower_simd_float_binary` in
   `builder_simd_float_math.rs` now `debug_assert!`s the kernel is atan2-only and loops
   `for err in kernel.errors()` (mirroring `lower_simd_float_binary_scalar`) instead of the
   hardcoded `emit_float_error_reduce(FloatError::Nan)`. For the only reachable kernel
   (`Atan2`, `errors() == [Nan]`) this is byte-identical; a future Inf-raising kernel wired
   here trips the assert until its `v24` mask is hoisted out of the per-iteration body.
3. **Dead code / docstrings.** Removed the 4 commented-out duplicate lines in the atan2 body
   and corrected the `lower_simd_float_binary` docstring (was "atan2/pow"; pow never reaches
   this driver — it is diverted to `lower_pow_array`).

Docs: added a signed-zero paragraph to the `math::min`/`max`/`clamp` man pages
(`src/docs/man/builtins/math/{min,max,clamp}.txt`) — the spec was silent on the `±0.0` tie.

Regression test: `tests/rt-behavior/math/math_simd_signzero_tail_valid` (odd-length length-3
Float lists placing the signed-zero pair at a SIMD body lane index 0 and the scalar tail
index 2; the sign is observed through `atan2(z,-1)` = `±pi` because `toString` collapses
`±0.0`). Fails before / passes after.

Runtime proof (fixed binary): body lane and tail lane now agree —
`min: -pi/-pi  max: +pi/+pi  clamp: +pi/+pi` (before: tails were `+pi/-pi/-pi`).

Out-of-scope defect found while reproducing (NOT fixed): `math::clamp` on a `List OF Float`
with a **non-literal** (d-register-carried) scalar `low`/`high` bound miscompiles — the bound
is spilled with `store_u64` without being materialized to a GPR first, so the broadcast reads
garbage (e.g. `clamp(xs, 0.0 - 1.0, 3.0)` clamps against `0.0` instead of `-1.0`; a variable
bound raises a spurious `ErrInvalidArgument`). Lives in `lower_math_clamp_array`
(`builder_math.rs`), which — unlike `lower_math_min_max` — omits the `materialize_float` call.
Literal bounds work. Worth its own bug.

Note: the acceptance/artifact goldens for tests exercising the Float array min/max/clamp
**tail** (the `.ncode`/`.mir`/`.nir` encoders) shift; the atan2 array path is byte-identical
(the `errors()` loop reduces the same single `Nan`). No runtime output of any existing test
changes (no existing test hit a `±0.0` tail tie). The unit-test target could not be built
locally — a concurrent agent's in-flight edit to `src/target/shared/code/validation.rs` fails
to compile — but the `mfb` binary builds clean and the runtime behavior is proven above.
