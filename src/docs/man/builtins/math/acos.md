# acos

Arc cosine in radians — the inverse of `math::cos` on its principal branch.

## Synopsis

```
math::acos(value AS Float) AS Float
math::acos(value AS Fixed) AS Fixed
math::acos(values AS List OF Float) AS List OF Float
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

`math::acos` returns the angle in radians whose cosine is `value`. The result is
the inverse of `math::cos` restricted to a single branch and always lies in
`[0, pi]`. `acos(1)` is zero, `acos(0)` is `pi/2`, and `acos(-1)` is `pi`.

The arc cosine is defined only on the closed interval `[-1, 1]`. A `value`
outside that domain fails, and the two overloads report it with **different**
error codes: the `Float` overload raises `ErrFloatDomain`, the `Fixed` overload
raises `ErrInvalidArgument`.
[[src/target/shared/code/builder_simd_float_math.rs:FloatKernel]]
[[src/target/shared/code/builder_fixed_math.rs:emit_fixed_asin]]

The `Float` overload uses the half-angle identity
`acos(x) = 2 * atan(sqrt((1 - x) / (1 + x)))` rather than `pi/2 - asin(x)`, which
avoids catastrophic cancellation as `x` approaches `+1` (where the true result
approaches zero); `1 +/- x` is exact for `|x| <= 1`, so the result stays within
1 ULP across the whole domain. The `Fixed` overload uses
`atan2(sqrt(1 - x^2), x)` in Q32.32.
[[src/target/shared/code/builder_fixed_math.rs:emit_fixed_asin]]

`math::acos` accepts `Float` or `Fixed` **only**. `Integer` is not accepted and
neither is `Money`: the transcendental functions do not take part in the `Money`
dimension, so `math::acos(1)` and `math::acos(1.00m)` are both compile-time errors.
Convert explicitly first. [[src/builtins/math.rs:one_float_or_fixed]]

The `Float` overload is computed by a hand-written **in-tree kernel**: MFBASIC
never links or calls a platform math library, so the answer does not vary with
the host `libm`. The scalar and array overloads share that one kernel, so
`math::acos(x)` and the corresponding element of the array form are bit-identical.
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
array overload for `math::acos` — a `Fixed` list is a compile-time error.
[[src/builtins/math.rs:resolve_call]]

## Overloads

**`math::acos(value AS Float) AS Float`**

Hand-written in-tree kernel using the half-angle identity, within 1 ULP of macOS `libm`. Fails with `ErrFloatDomain` when `|value| > 1`.

**`math::acos(value AS Fixed) AS Fixed`**

Deterministic Q32.32 `atan2(sqrt(1 - x^2), x)`, rounded to the nearest representable `Fixed`. Fails with `ErrInvalidArgument` when `|value| > 1`.

**`math::acos(values AS List OF Float) AS List OF Float`**

Applies `math::acos` to every element and returns a new `List OF Float` of the same
length. Each element equals the corresponding scalar result exactly.
A per-element failure is accumulated across the whole list and reported once
after every element has been processed, so the reported error does not depend on
which element triggered it; no list is returned in that case.
[[src/target/shared/code/builder_simd_float_math.rs:lower_simd_float_unary]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `Float` or `Fixed` | The cosine value whose angle is wanted. Must lie in the closed interval `[-1, 1]`. This parameter has no alternate name. [[src/builtins/math.rs:call_param_names]] |
| `values` | `List OF Float` | The array form: mapped element-wise. The empty list yields an empty list. [[src/builtins/math.rs:resolve_call]] |

## Return value

| Type | Description |
| --- | --- |
| Same as the argument | The arc cosine of `value` in radians, in the argument's own type, within `[0, pi]`. The array form returns a new `List OF Float` of the same length. [[src/builtins/math.rs:resolve_call]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050012` | `ErrFloatDomain` | The `Float` overload only: `|value| > 1`, outside the arc cosine's domain. [[src/target/shared/code/error_constants.rs:ERR_FLOAT_DOMAIN_CODE]] |
| `77050002` | `ErrInvalidArgument` | The `Fixed` overload only: `|value| > 1`. [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] |

## Type checking

`math::acos` takes exactly one argument. [[src/builtins/math.rs:arity]] It must be
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
  LET a AS Float = math::acos(0.5)
  LET b AS Fixed = math::acos(-1.0F)
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
  LET results AS List OF Float = math::acos(angles)
  io::print(toString(collections::get(results, 0)))
END SUB
```

## See also

- `mfb man math asin`
- `mfb man math atan`
- `mfb man math atan2`
- `mfb man math cos`
- `mfb man math`
