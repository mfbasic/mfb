# bug-128 — Fixed `atan2` silently wrong for large-magnitude args; negation of raw i64::MIN is a no-op

**Status:** OPEN. Filed 2026-07-11 (goal-02 review, G9).
**Severity:** MED — silent wrong result (no trap) for valid Fixed inputs.
**Class:** correctness.

## Finding

`src/target/shared/code/builder_fixed_math.rs:252-326` (`emit_fixed_atan2`),
:306-307 (reflection), :220-249 (`emit_cordic_vectoring`).

(a) CORDIC vectoring magnitude grows to ≈1.6468·hypot(x,y); any input with raw
|value| ≳ 2^63/1.6468 (real value ≳ ~1.3e9) overflows the signed i64 working
registers mid-iteration, flipping `vy`'s sign tests and corrupting the
accumulated angle — silent wrong result, no trap.
(b) For `x == -2147483648.0F` (raw i64::MIN) the reflection `vx = 0 - vx` is a
no-op (two's-complement identity), so CORDIC runs with vx < 0, violating its
`vx > 0` precondition → wrong quadrant/garbage.

atan2's domain is all Fixed values, so these are valid inputs.

## Trigger

- `math::atan2(1.0F, -2147483648.0F)` → garbage (expected ≈π).
- `math::atan2(2000000000.0F, 2000000000.0F)` → corrupted by overflow (expected
  π/4 ≈ 0.7854).

## Fix sketch

Pre-scale (down-shift) large-magnitude inputs before the CORDIC iteration (the
angle is scale-invariant), and handle the i64::MIN input specially rather than
negating it.
