# atan2

Two-argument arc tangent in radians, resolving the full circle from the signs of both components.

## Synopsis

```
math::atan2(y AS Float, x AS Float) AS Float
math::atan2(y AS Fixed, x AS Fixed) AS Fixed
math::atan2(y AS List OF Float, x AS List OF Float) AS List OF Float
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

`math::atan2` returns the angle in radians of the vector whose horizontal
component is `x` and whose vertical component is `y`, following the standard
`atan2(y, x)` argument order — the vertical component comes **first**.

Unlike a single-argument `math::atan(y / x)`, `atan2` inspects the signs of both
arguments to place the angle in the correct quadrant, so the result spans the
whole circle and lies in `(-pi, pi]`. `atan2(0, x)` is zero for positive `x` and
`pi` for negative `x`; a positive `y` gives a positive angle and a negative `y` a
negative one.

The origin is defined, not an error: `atan2(0, 0)` is zero in both overloads.
The `Float` kernel captures the origin explicitly and forces those lanes to
`+0.0` before its NaN check, so the origin does not trip `ErrFloatNaN`.
[[src/target/shared/code/builder_simd_float_math.rs:FloatBinaryKernel]]

Both arguments must already be the same numeric type, and the return type is
that type: two `Float`s yield a `Float` and two `Fixed`s a `Fixed`. `math::atan2`
accepts `Float` or `Fixed` **only** — `Integer` and `Money` are compile-time
errors, and there is no mixed-type overload.
[[src/builtins/math.rs:two_same_float_or_fixed]]

The `Float` overload is computed by a hand-written **in-tree kernel** within
1 ULP of macOS `libm`: MFBASIC never links or calls a platform math library, so
the answer does not vary with the host. Its only failure is a NaN result, which
happens when `y` or `x` is itself a NaN — `atan2` is bounded by `pi`, so it has
no infinity failure path.
[[src/target/shared/code/builder_simd_float_math.rs:FloatBinaryKernel]] Across
macOS, Linux-glibc, and Linux-musl, and across the aarch64, x86-64, and riscv64
backends, a `Float` result is bit-identical in practice — though, unlike `Fixed`,
that is a uniform-lowering policy rather than a contractual guarantee.
[[src/docs/spec/architecture/18_math-kernels.md]]

The `Fixed` overload runs deterministic CORDIC circular vectoring in raw Q32.32
with no floating-point step anywhere, so its result is bit-identical on every
target **by construction**. It has no failure path at all.
[[src/target/shared/code/builder_fixed_math.rs:emit_fixed_atan2]]

The array overload pairs two `List OF Float` element-wise and returns a new
`List OF Float` of the same length; neither input is mutated. The two lists must
have the same length. There is **no** `List OF Fixed` array overload.
[[src/target/shared/code/builder_math.rs:lower_math_atan2_pow_array]]

## Overloads

**`math::atan2(y AS Float, x AS Float) AS Float`**

Hand-written in-tree kernel, within 1 ULP of macOS `libm`. Fails with
`ErrFloatNaN` when the computed result is a NaN, which happens only when `y` or
`x` is a NaN; every other pair, including the origin `(0, 0)`, succeeds.

**`math::atan2(y AS Fixed, x AS Fixed) AS Fixed`**

Deterministic Q32.32 CORDIC vectoring, rounded to the nearest representable
`Fixed`. Never fails.
[[src/target/shared/code/builder_fixed_math.rs:emit_fixed_atan2]]

**`math::atan2(y AS List OF Float, x AS List OF Float) AS List OF Float`**

Applies `atan2` across the two lists element-wise and returns a new
`List OF Float` of the same length. Each element equals the corresponding scalar
result exactly. Lists of differing length fail with `ErrInvalidArgument` before
any element is computed. A per-element NaN is accumulated across the whole list
and reported once after every element has been processed, so the reported error
does not depend on which element triggered it.
[[src/target/shared/code/builder_math.rs:lower_math_atan2_pow_array]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `y` | `Float`, `Fixed`, or `List OF Float` | The **vertical** component, and the first positional argument. Any value is accepted; there is no restricted domain. This parameter has no alternate name. [[src/builtins/math.rs:call_param_names]] |
| `x` | Same type as `y` | The **horizontal** component. Its sign selects the half-plane in which the angle is measured. Must be exactly the same type as `y`. This parameter has no alternate name. [[src/builtins/math.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| Same as the arguments | The angle in radians of the vector `(x, y)`, in `(-pi, pi]`. `atan2(0, 0)` is zero. The array form returns a new `List OF Float` of the same length. [[src/builtins/math.rs:resolve_call]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050013` | `ErrFloatNaN` | The `Float` overload only: the computed result is a NaN, which happens only when `y` or `x` is itself a NaN. The `Fixed` overload never fails. [[src/target/shared/code/error_constants.rs:ERR_FLOAT_NAN_CODE]] [[src/target/shared/code/builder_simd_float_math.rs:FloatBinaryKernel]] |
| `77050002` | `ErrInvalidArgument` | The array overload only: the two lists have different lengths. [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] [[src/target/shared/code/builder_math.rs:lower_math_atan2_pow_array]] |

## Type checking

`math::atan2` takes exactly two arguments. [[src/builtins/math.rs:arity]] They
must share one type: two `Float`s, two `Fixed`s, or two `List OF Float`. Mixing
`Float` with `Fixed`, passing an `Integer`, a `Money`, a `List OF Fixed`, or any
non-numeric value such as a `String`, `Boolean`, `Byte`, `Scalar`, record,
union, resource, thread, or function value is a compile-time type error.
[[src/builtins/math.rs:expected_arguments]]

## Examples

Angle of a `Float` vector, and a `Fixed` vector on the negative horizontal axis
(the `F` suffix makes a literal a `Fixed`):

```
IMPORT math
IMPORT io

SUB main()
  LET quarter AS Float = math::atan2(1.0, 1.0)
  LET half AS Fixed = math::atan2(0.0F, -1.0F)
  io::print(toString(quarter))
  io::print(toString(half))
END SUB
```

Element-wise over two same-length lists:

```
IMPORT math
IMPORT io
IMPORT collections

SUB main()
  LET ys AS List OF Float = [1.0, 0.0]
  LET xs AS List OF Float = [1.0, -1.0]
  LET angles AS List OF Float = math::atan2(ys, xs)
  io::print(toString(collections::get(angles, 1)))
END SUB
```

## See also

- `mfb man math atan`
- `mfb man math asin`
- `mfb man math acos`
- `mfb man math tan`
- `mfb man math`
