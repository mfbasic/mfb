# tan

Tangent of an angle given in radians.

## Synopsis

```
math::tan(value AS Float) AS Float
math::tan(value AS Fixed) AS Fixed
math::tan(values AS List OF Float) AS List OF Float
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

`math::tan` returns the trigonometric tangent of `value`, where `value` is a
plane angle **in radians** — equivalently `sin(value) / cos(value)`. Unlike sine
and cosine the tangent is unbounded: as the angle approaches an odd multiple of
`pi/2` the magnitude of the result grows without limit. `tan(0)` is zero.

The `Float` overload does **not** raise an error at an odd multiple of `pi/2`. No
`Float` is exactly `pi/2`, so the tangent there is merely a very large finite
number (about `1.6e16` at the nearest `Float` to `pi/2`) and is returned as such.
`math::tan` has no infinity failure path; its only failure is a NaN result.
[[src/target/shared/code/builder_simd_float_math.rs:FloatKernel]]

The `Fixed` overload *does* have an undefined-point failure: it computes
`sin / cos` in Q32.32, and a computed cosine of exactly zero fails with
`ErrInvalidArgument` through the fixed-point divide.
[[src/target/shared/code/builder_fixed_math.rs:emit_fixed_tan]]

The `Float` kernel reduces the angle with a Cody-Waite scheme that is accurate
only for `|value| < 2^20 * pi/2` (about `1.6e6`); it does not implement a
Payne-Hanek reduction for huge arguments. Beyond that range the result
progressively loses meaning — for a magnitude around `1e18` it can even fall
outside `[-1, 1]` — and for a large enough magnitude, as well as for an infinity
or a NaN argument, the reduction produces a NaN, which is reported as
`ErrFloatNaN` rather than returned. Keep the angle in a sensible range if the
answer must be meaningful.
[[src/target/shared/code/builder_simd_float_math.rs:FloatKernel]]

`math::tan` accepts `Float` or `Fixed` **only**. `Integer` is not accepted and
neither is `Money`: the transcendental functions do not take part in the `Money`
dimension, so `math::tan(1)` and `math::tan(1.00m)` are both compile-time errors.
Convert explicitly first. [[src/builtins/math.rs:one_float_or_fixed]]

The `Float` overload is computed by a hand-written **in-tree kernel**: MFBASIC
never links or calls a platform math library, so the answer does not vary with
the host `libm`. The scalar and array overloads share that one kernel, so
`math::tan(x)` and the corresponding element of the array form are bit-identical.
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
array overload for `math::tan` — a `Fixed` list is a compile-time error.
[[src/builtins/math.rs:resolve_call]]

## Overloads

**`math::tan(value AS Float) AS Float`**

Hand-written in-tree kernel, **faithfully rounded** — within 1 ULP of the true value, which is more accurate than macOS `libm` — via a double-double sine/cosine and a compensated divide. Fails with `ErrFloatNaN` when the computed result is a NaN. It never raises `ErrFloatInf`.

**`math::tan(value AS Fixed) AS Fixed`**

Deterministic Q32.32 `sin / cos`, rounded to the nearest representable `Fixed`. Fails with `ErrInvalidArgument` when the computed cosine is exactly zero.

**`math::tan(values AS List OF Float) AS List OF Float`**

Applies `math::tan` to every element and returns a new `List OF Float` of the same
length. Each element equals the corresponding scalar result exactly.
A per-element failure is accumulated across the whole list and reported once
after every element has been processed, so the reported error does not depend on
which element triggered it; no list is returned in that case.
[[src/target/shared/code/builder_simd_float_math.rs:lower_simd_float_unary]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `Float` or `Fixed` | The angle whose tangent is taken, in radians. Every value is accepted, but see the notes on undefined points and large magnitudes. This parameter has no alternate name. [[src/builtins/math.rs:call_param_names]] |
| `values` | `List OF Float` | The array form: mapped element-wise. The empty list yields an empty list. [[src/builtins/math.rs:resolve_call]] |

## Return value

| Type | Description |
| --- | --- |
| Same as the argument | The tangent of `value`, in the argument's own type. The result is unbounded and may have either sign. `tan(0)` is zero. The array form returns a new `List OF Float` of the same length. [[src/builtins/math.rs:resolve_call]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050013` | `ErrFloatNaN` | The `Float` overload only: the computed result is a NaN, which happens for an infinity or NaN argument and for a sufficiently large magnitude. [[src/target/shared/code/error_constants.rs:ERR_FLOAT_NAN_CODE]] |
| `77050002` | `ErrInvalidArgument` | The `Fixed` overload only: the computed cosine of `value` is exactly zero, so the tangent is undefined at that point. [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] |

## Type checking

`math::tan` takes exactly one argument. [[src/builtins/math.rs:arity]] It must be
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
  LET a AS Float = math::tan(math::pi4)
  LET b AS Fixed = math::tan(math::pi4Fixed)
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
  LET results AS List OF Float = math::tan(angles)
  io::print(toString(collections::get(results, 0)))
END SUB
```

## See also

- `mfb man math sin`
- `mfb man math cos`
- `mfb man math atan`
- `mfb man math atan2`
- `mfb man math`
