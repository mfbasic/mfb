# bug-131 — Float `atan2(0.0, 0.0)` traps ErrFloatNan; man page promises it succeeds

**Status:** OPEN. Filed 2026-07-11 (goal-02 review, G9).
**Severity:** MED — documented input traps; diverges from spec and the Fixed
sibling.
**Class:** correctness (docs-vs-behavior).

## Finding

`src/target/shared/code/builder_simd_float_math.rs:1348-1357`
(`emit_float_binary_body` Atan2: `q = y/x` then `emit_result_nan_into_mask`),
reached from builder_math.rs:254-265 and the array driver. The kernel computes
`atan(y/x)` + quadrant offset; at the origin `0/0 = NaN`, the result-NaN mask
fires, and the scalar/array overloads raise ErrFloatNan.

`src/docs/man/builtins/math/atan2.txt:39-51` states: "every other pair of Float
arguments, including the origin (0, 0), succeeds" (and atan2(0, x>0)=0). The
Fixed overload handles (0,0)→0 explicitly
(builder_fixed_math.rs:272-277) — the Float overload diverges from both the spec
and its Fixed sibling.

## Trigger

`math::atan2(0.0, 0.0)` → ErrFloatNan; expected 0.0.

## Fix

Add the IEEE atan2 special-case prologue (origin and axis cases) to the Float
kernel, matching the Fixed overload and the man page.
