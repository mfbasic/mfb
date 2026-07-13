# bug-164 — SIMD/scalar `math::exp` kernel has no large-argument guard; Cody-Waite reduction breaks down for large finite inputs

Last updated: 2026-07-12
Severity: MEDIUM — wrong finite result (or spurious ErrFloatInf) for large-magnitude `math::exp` arguments.
Class: Correctness.
Status: Open

## Finding

`src/target/shared/code/builder_simd_float_math.rs:930-970` (`emit_exp_body`,
the shared kernel used by both scalar and array `math::exp`). It computes
`n = frintm(x/ln2 + 0.5)` then `r = x - n*ln2_hi - n*ln2_lo` with no early-out.
For `|x| ≳ 2^52·ln2 ≈ 3e15`, `n*ln2` carries rounding error ≫ 0.5, so the
reduced `r` is garbage instead of lying in `[-ln2/2, ln2/2]` and the Horner
`P(r)` is meaningless. Separately, the `2^n` scaling `(n_i + 1023) << 52`
(:957-960) builds a nonsense biased-exponent field once `n1+1023` goes far
negative. Result: a wrong finite value returned with no error, or a spurious
`ErrFloatInf` from the `result*0` finiteness check (:965-969). Unlike the sin/cos
kernels (which document the accepted `|x| < 2^20·pi/2` medium-range limit), exp
has no acknowledgment or guard — no `709/745`-style clamp anywhere in the exp
path.

## Trigger

`math::exp(-1.0e16)` (finite, passes the Float finiteness boundary): correct
result is `0.0` with no error; observed is a wrong finite value or a spurious
`ErrFloatInf`. Likewise very large positive `x` should overflow to `+Inf`.

## Fix

Before reduction, mask lanes with `|x|` above the overflow/underflow thresholds
(~709.78 / ~-745.13) and force those results to `+Inf` (with the finite-check
policy) / `+0.0` respectively, bypassing the reduction (as glibc does). Add
runtime tests for `exp(710)`, `exp(-745)`, `exp(-1e16)`, `exp(1e16)`.
