# bug-130 — NEON `exp` kernel mis-handles both result-range boundaries (spurious ErrFloatInf; flush-to-zero of representable subnormals)

**Status:** OPEN. Filed 2026-07-11 (goal-02 review, G9).
**Severity:** MED — wrong result / spurious trap near the exp range boundaries;
scalar and array Float `exp` share the kernel.
**Class:** correctness.

## Finding

`src/target/shared/code/builder_simd_float_math.rs:930-965` (`emit_exp_body_lo`:
`n > 1023` inf mask at :957, `n < -1022` flush at :959-963).

(a) `n = floor(x/ln2 + 0.5) > 1023` raises ErrFloatInf, but for x ∈ [709.44,
709.782] n = 1024 while the true result (P(r)·2^1024 with P<1) is a finite
normal double — the man page says ErrFloatInf fires "when the result overflows
to infinity".
(b) `n < -1022` selects 0: every subnormal result (x ∈ (−745.14, −708.39])
is returned as +0.0 rather than its representable subnormal value, contradicting
the "within 1 ULP of macOS libm" claim.

## Trigger

- `math::exp(709.5)` → traps ErrFloatInf (libm: 1.3549e308).
- `math::exp(-709.0)` → 0.0 (libm: 1.216e-308).

## Fix sketch

Push the overflow decision to the actual result (check after the 2^n scale, or
raise the n-threshold to 1025 and let the finiteness check catch true
overflow), and implement the subnormal tail (two-step scale) instead of
flush-to-zero.

## Prior art

bug-68 covered tail sign-zero/NaN-reduce, not range boundaries.
