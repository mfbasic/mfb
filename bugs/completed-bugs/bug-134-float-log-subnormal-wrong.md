# bug-134 — Float `log`/`log10` of subnormal inputs silently wrong (frexp ignores denormal exponent)

**Status:** OPEN. Filed 2026-07-11 (goal-02 review, G9).
**Severity:** MED — silent wrong value (no trap) for subnormal Float inputs.
**Class:** correctness.

## Finding

`src/target/shared/code/builder_simd_float_math.rs:975-997` (`emit_log_body`
k/m extraction :980-991). `k = ((bits>>52)&0x7ff) − 1022` and `m = mantissa |
(1022<<52)` assume a normal input. A subnormal (exp field 0, no implicit bit)
gets k = −1022 and a fake m ∈ [0.5, 0.5+2^-12), ignoring the leading zeros of the
true mantissa. fdlibm pre-scales by 2^54; this kernel does not.

## Trigger

`math::log(5.0e-324)` → ≈ −709.09 instead of −744.44; any `log`/`log10` (scalar
or array) of a subnormal (reachable from arithmetic underflow — subnormals are
legal finite Floats). Silent wrong value, no trap.

## Fix

Add the fdlibm subnormal prologue: if the input is subnormal, multiply by 2^54,
compute log, and subtract 54·ln2 (log) / 54·log10(2) (log10) from the result.
