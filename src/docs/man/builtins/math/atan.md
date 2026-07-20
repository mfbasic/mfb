# atan

Arc tangent in radians — the inverse of `math::tan` on its principal branch.

## Synopsis

```
math::atan(value AS Float) AS Float
math::atan(value AS Fixed) AS Fixed
math::atan(values AS List OF Float) AS List OF Float
```

## Package

math

## Imports

```
IMPORT math
```

`math` is a built-in package, so no manifest dependency is required.
[[src/builtins/math.rs:is_math_call]]

## Description

`math::atan` returns the angle in radians whose tangent is `value`. The result is
the inverse of `math::tan` restricted to a single branch and lies in the open
interval `(-pi/2, pi/2)`. `atan(0)` is zero.

The arc tangent is defined for every real number, so there is no restricted
domain and no finite input is rejected. As `|value|` grows the result approaches
`+/-pi/2` without reaching it; an infinite `Float` argument is accepted and
returns exactly `+/-pi/2`. Only a NaN argument drives the result to a NaN, which
the `Float` overload reports as `ErrFloatNaN`. Unlike `sin`/`cos`/`tan`, `atan`
needs no angle reduction, so a huge argument is handled correctly rather than
losing meaning. [[src/target/shared/code/builder_simd_float_math.rs:FloatKernel]]

The `Float` overload uses the fdlibm four-segment reduction. The `Fixed` overload
computes `atan(value) = atan2(value, 1)` with CORDIC vectoring in Q32.32 and has
no failure path at all.
[[src/target/shared/code/builder_fixed_math.rs:emit_fixed_atan2]]

`math::atan` accepts `Float` or `Fixed` **only**. `Integer` is not accepted and
neither is `Money`: the transcendental functions do not take part in the `Money`
dimension, so `math::atan(1)` and `math::atan(1.00m)` are both compile-time errors.
Convert explicitly first. [[src/builtins/math.rs:one_float_or_fixed]]

The `Float` overload is computed by a hand-written **in-tree kernel**: MFBASIC
never links or calls a platform math library, so the answer does not vary with
the host `libm`. The scalar and array overloads share that one kernel, so
`math::atan(x)` and the corresponding element of the array form are bit-identical.
Across macOS, Linux-glibc, and Linux-musl, and across the aarch64, x86-64, and
riscv64 backends, a `Float` result is bit-identical in practice — though, unlike
`Fixed`, that is a uniform-lowering policy rather than a contractual guarantee.
[[src/docs/spec/architecture/18_math-kernels.md]]

The `Fixed` overload runs a deterministic raw Q32.32 routine with no
floating-point step anywhere, so its result is bit-identical on every target
**by construction** — for `Fixed` that cross-target identity is a contractual
guarantee. [[src/docs/spec/architecture/18_math-kernels.md]]

The array overload takes a `List OF Float` and returns a new `List OF Float` of
the same length; the input list is not mutated. There is **no** `List OF Fixed`
array overload for `math::atan` — a `Fixed` list is a compile-time error.
[[src/builtins/math.rs:resolve_call]]

## Overloads

**`math::atan(value AS Float) AS Float`**

Hand-written in-tree kernel (fdlibm four-segment reduction), within 1 ULP of macOS `libm`. Fails with `ErrFloatNaN` only when `value` is itself a NaN; every other `Float`, including an infinity, succeeds.

**`math::atan(value AS Fixed) AS Fixed`**

Deterministic Q32.32 `atan2(value, 1)` by CORDIC, rounded to the nearest representable `Fixed`. Never fails.

**`math::atan(values AS List OF Float) AS List OF Float`**

Applies `math::atan` to every element and returns a new `List OF Float` of the same
length. Each element equals the corresponding scalar result exactly.
A per-element failure is accumulated across the whole list and reported once
after every element has been processed, so the reported error does not depend on
which element triggered it; no list is returned in that case.
[[src/target/shared/code/builder_simd_float_math.rs:lower_simd_float_unary]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `Float` or `Fixed` | The tangent value whose angle is wanted. Any value is accepted; there is no restricted domain. This parameter has no alternate name. [[src/builtins/math.rs:call_param_names]] |
| `values` | `List OF Float` | The array form: mapped element-wise. The empty list yields an empty list. [[src/builtins/math.rs:resolve_call]] |

## Return value

| Type | Description |
| --- | --- |
| Same as the argument | The arc tangent of `value` in radians, in the argument's own type, within `(-pi/2, pi/2)`; exactly `+/-pi/2` for an infinite `Float`. The array form returns a new `List OF Float` of the same length. [[src/builtins/math.rs:resolve_call]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050013` | `ErrFloatNaN` | The `Float` overload only: `value` is a NaN. The `Fixed` overload never fails. [[src/target/shared/code/error_constants.rs:ERR_FLOAT_NAN_CODE]] |

## Type checking

`math::atan` takes exactly one argument. [[src/builtins/math.rs:arity]] It must be
a single `Float` or `Fixed`, or a `List OF Float`. An `Integer`, a `Money`, a
`List OF Fixed`, or any non-numeric value such as a `String`, `Boolean`, `Byte`,
`Scalar`, record, union, resource, thread, or function value is a compile-time
type error. [[src/builtins/math.rs:expected_arguments]]

## Examples

The `Float` and the deterministic `Fixed` overload (the `F` suffix makes a
literal a `Fixed` rather than a `Float`):

```
IMPORT math
IMPORT io

SUB main()
  LET a AS Float = math::atan(1.0)
  LET b AS Fixed = math::atan(1.0F)
  io::print(toString(a))
  io::print(toString(b))
END SUB
```

Mapped element-wise over a list of `Float`:

```
IMPORT math
IMPORT io
IMPORT collections

SUB main()
  LET angles AS List OF Float = [0.0, 0.5]
  LET results AS List OF Float = math::atan(angles)
  io::print(toString(collections::get(results, 0)))
END SUB
```

## See also

- `mfb man math atan2`
- `mfb man math asin`
- `mfb man math acos`
- `mfb man math tan`
- `mfb man math`
