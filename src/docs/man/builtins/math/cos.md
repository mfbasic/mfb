# cos

Cosine of an angle given in radians.

## Synopsis

```
math::cos(value AS Float) AS Float
math::cos(value AS Fixed) AS Fixed
math::cos(values AS List OF Float) AS List OF Float
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

`math::cos` returns the trigonometric cosine of `value`, where `value` is a plane
angle **in radians**. For an argument in the kernel's accurate range the result
lies in `[-1, 1]`, and `cos(0)` is one.

Cosine is defined for every real argument, so there is no restricted domain and
no finite input is rejected outright.

The `Float` kernel reduces the angle with a Cody-Waite scheme that is accurate
only for `|value| < 2^20 * pi/2` (about `1.6e6`); it does not implement a
Payne-Hanek reduction for huge arguments. Beyond that range the result
progressively loses meaning — for a magnitude around `1e18` it can even fall
outside `[-1, 1]` — and for a large enough magnitude, as well as for an infinity
or a NaN argument, the reduction produces a NaN, which is reported as
`ErrFloatNaN` rather than returned. Keep the angle in a sensible range if the
answer must be meaningful.
[[src/target/shared/code/builder_simd_float_math.rs:FloatKernel]]

The `Fixed` overload reduces the angle to `[-pi/4, pi/4]`, tracks the quadrant,
and runs unrolled CORDIC rotation. It has no failure path at all.
[[src/target/shared/code/builder_fixed_math.rs:emit_fixed_sin_cos]]

`math::cos` accepts `Float` or `Fixed` **only**. `Integer` is not accepted and
neither is `Money`: the transcendental functions do not take part in the `Money`
dimension, so `math::cos(1)` and `math::cos(1.00m)` are both compile-time errors.
Convert explicitly first. [[src/builtins/math.rs:one_float_or_fixed]]

The `Float` overload is computed by a hand-written **in-tree kernel**: MFBASIC
never links or calls a platform math library, so the answer does not vary with
the host `libm`. The scalar and array overloads share that one kernel, so
`math::cos(x)` and the corresponding element of the array form are bit-identical.
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
array overload for `math::cos` — a `Fixed` list is a compile-time error.
[[src/builtins/math.rs:resolve_call]]

## Overloads

**`math::cos(value AS Float) AS Float`**

Hand-written in-tree kernel, within 1 ULP of macOS `libm`. Fails with `ErrFloatNaN` when the computed result is a NaN — an infinity or NaN argument, or a magnitude past the reduction range.

**`math::cos(value AS Fixed) AS Fixed`**

Deterministic Q32.32 CORDIC, rounded to the nearest representable `Fixed`. Never fails.

**`math::cos(values AS List OF Float) AS List OF Float`**

Applies `math::cos` to every element and returns a new `List OF Float` of the same
length. Each element equals the corresponding scalar result exactly.
A per-element failure is accumulated across the whole list and reported once
after every element has been processed, so the reported error does not depend on
which element triggered it; no list is returned in that case.
[[src/target/shared/code/builder_simd_float_math.rs:lower_simd_float_unary]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `Float` or `Fixed` | The angle whose cosine is taken, in radians. Every value is accepted, but see the note on large magnitudes. This parameter has no alternate name. [[src/builtins/math.rs:call_param_names]] |
| `values` | `List OF Float` | The array form: mapped element-wise. The empty list yields an empty list. [[src/builtins/math.rs:resolve_call]] |

## Return value

| Type | Description |
| --- | --- |
| Same as the argument | The cosine of `value`, in the argument's own type; within `[-1, 1]` for an argument in the accurate range. `cos(0)` is one. The array form returns a new `List OF Float` of the same length. [[src/builtins/math.rs:resolve_call]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050013` | `ErrFloatNaN` | The `Float` overload only: the computed result is a NaN, which happens for an infinity or NaN argument and for a sufficiently large magnitude. The `Fixed` overload never fails. [[src/target/shared/code/error_constants.rs:ERR_FLOAT_NAN_CODE]] |

## Type checking

`math::cos` takes exactly one argument. [[src/builtins/math.rs:arity]] It must be
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
  LET a AS Float = math::cos(math::pi)
  LET b AS Fixed = math::cos(math::piFixed)
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
  LET results AS List OF Float = math::cos(angles)
  io::print(toString(collections::get(results, 0)))
END SUB
```

## See also

- `mfb man math sin`
- `mfb man math tan`
- `mfb man math acos`
- `mfb man math atan2`
- `mfb man math`
