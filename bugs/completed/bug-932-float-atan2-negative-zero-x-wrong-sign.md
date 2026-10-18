# bug-932: `math::atan2(y, -0.0)` returns the wrong sign: `atan2(1.0, -0.0)` is -pi/2, `atan2(-1.0, -0.0)` is +pi/2

Last updated: 2026-10-05
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness

Status: Fixed
Regression Test: `tests/runtime/rt_math_atan2_negative_zero_x.rs`

## Cause (coordinator, bug-staging-24)

- Rule: `atan2(y, ±0.0)` is `+pi/2` for `y > 0` and `-pi/2` for `y < 0` (IEEE 754 / C Annex F and `mfb man math atan2`: "with x zero the result is pi/2 or -pi/2 by the sign of y"); the origin `atan2(±0, ±0) = 0` stays (bug-131).
- Where: `src/codegen/builtins/vector/builder_simd_float_math.rs:emit_float_binary_body`, `Atan2` arm: the quadrant correction keys on `vector_fcmlt_zero(x)`, an ordered compare for which `-0.0 < 0` is false.
- Fix: the correction keys on the SIGN BIT of `x`, one integer test of bit 63; scalar and list overloads share the kernel. (Never `cargo clippy --fix` this file: it degrades its hi/lo constants.)

The `Float` `atan2` kernel (scalar and `List OF Float` overloads, every target) gives the **negated**
angle when `x` is negative zero and `y` is non-zero: `atan2(1.0, -0.0)` = `-1.570796` and
`atan2(-1.0, -0.0)` = `+1.570796`. With `x = +0.0` the same calls are right (`atan2(1.0, 0.0)` =
`+1.570796`). Negative zero is an ordinary value in a program — it is what `-1.0 * 0.0`, `-z` for
`z = 0.0`, or a coordinate difference like `0.0 - 0.0` after a negation produces — so the direction
of a vertical vector can flip silently, with no error.

**Correct behavior** (macOS libm, IEEE 754 / C Annex F, and the man page's own rule "with x zero the
result is pi/2 or -pi/2 by the sign of y"): `atan2(y, ±0.0)` for `y > 0` is `+pi/2`, for `y < 0` is
`-pi/2`. The origin cases stay as documented by bug-131 (`atan2(±0.0, ±0.0)` = 0).

References:

- `src/docs/spec/architecture/18_math-kernels.md` "Accuracy": `atan2` "≤1 ULP of macOS libm" (libm
  `atan2(1.0, -0.0)` = `1.5707963267948966`; MFBASIC is off by pi).
- `mfb man math atan2`: "with x zero the result is pi/2 or -pi/2 by the sign of y".
- bugs/completed/bug-131 (origin defined as 0; fixture `tests/rt-behavior/math/bug131_float_atan2_origin`
  covers only `+0.0` operands).
- Found in hardening batch B12 (architecture/18_math-kernels), signed-zero sweep.

## Failing Reproduction

```
IMPORT io
IMPORT math
FUNC main() AS Nothing
  LET z AS Float = 0.0
  LET nz AS Float = -1.0 * z
  io::print(toString(math::atan2(1.0, nz), toByte(6)))
  io::print(toString(math::atan2(-1.0, nz), toByte(6)))
  io::print(toString(math::atan2(1.0, z), toByte(6)))
  FOR EACH v IN math::atan2([1.0, -1.0, 2.0], [nz, nz, nz])
    io::print("list " & toString(v, toByte(6)))
  NEXT
END FUNC
```

- Observed (macOS aarch64 native, and identical bytes on linux-riscv64 under qemu):
  ```
  -1.570796
  1.570796
  1.570796
  list -1.570796
  list 1.570796
  list -1.570796
  ```
- Expected:
  ```
  1.570796
  -1.570796
  1.570796
  list 1.570796
  list -1.570796
  list 1.570796
  ```

Signed-zero matrix (`toString(…, toByte(17))`; libm = Python `math.atan2` on the same host):

| call | MFBASIC | libm |
| --- | --- | --- |
| `atan2(1.0, -0.0)` | -1.5707963267948966 ✗ | 1.5707963267948966 |
| `atan2(-1.0, -0.0)` | 1.5707963267948966 ✗ | -1.5707963267948966 |
| `atan2(0.0, -1.0)` | 3.14159265358979312 ✓ | 3.141592653589793 |
| `atan2(-0.0, -1.0)` | -3.14159265358979312 ✓ | -3.141592653589793 |
| `atan2(0.0, 0.0)` / `(-0.0, -0.0)` / `(0.0, -0.0)` | 0 (documented origin rule) | 0 / -pi / pi |

## Root Cause

`src/codegen/builtins/vector/builder_simd_float_math.rs:emit_float_binary_body` (`Atan2` arm) computes
`atan(y/x) + (x<0 ? copysign(pi, y) : 0)` with the quadrant mask from `vector_fcmlt_zero(x)` — an
IEEE *compare*, for which `-0.0 < 0` is false. For `x = -0.0`, `y/x` is `-inf` (y > 0), `atan(-inf)` is
`-pi/2`, and the `+pi` correction that would give `+pi/2` is skipped. The correction must key on the
**sign bit** of `x` (e.g. `x < 0 || x is -0.0`, or an integer test of bit 63), not the ordered compare;
the origin lanes are already forced to `+0.0` separately and are unaffected.

## Goal

- `atan2(±1.0, -0.0)` returns `±pi/2` (bit-identical to libm) on the scalar and list overloads.
- The bug-131 origin cases keep returning 0; all other quadrants unchanged.

### Non-goals (must NOT change)

- The origin rule `atan2(±0.0, ±0.0) = 0` (man page + bug-131), unless the owner chooses to adopt IEEE.
- `Fixed` atan2 (no negative zero; measured within 1 unit over 12 axis/extreme cases in this batch).

## Blast Radius

- `emit_float_binary_body` Atan2 — the only Float binary SIMD kernel; scalar and array share it, so one
  fix covers both and every backend (measured identical wrong output on aarch64 and riscv64).
- Other kernels that branch on `x < 0` via an ordered compare with a signed-zero-sensitive result —
  not surveyed (e.g. `acos` half-angle uses `1±x`, insensitive).

## Fix

`emit_float_binary_body` (`Atan2` arm) builds the quadrant mask with `vector_sshr(x, 63)` (the
sign bit of `x` smeared across the lane) instead of the ordered `vector_fcmlt_zero(x)`; scalar
and list overloads share the kernel. Spec `architecture/18_math-kernels.md` states the rule.

RED (main's compiler, `MFB_TEST_EXE=/tmp/main-dbg-target/release/mfb cargo test --release --test
rt_math_atan2_negative_zero_x`): `atan2(1.0, -0.0)` printed `-1.570796` (scalar and list). GREEN
with this branch; the origin, `(+-0, -1)` and the other quadrants asserted unchanged.

Goldens expected to move: `byte-identity/math/math_codegen_cover_rt` (five `.ncode` shas;
measured by `artifact-gate math`, 5 diffs): the kernel emits one different vector op. Any other
fixture whose native code contains the Float atan2 kernel moves the same way (ncode kind only).
