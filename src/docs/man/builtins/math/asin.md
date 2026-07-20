# asin

Arc sine in radians — the inverse of `math::sin` on its principal branch.

## Synopsis

```
math::asin(value AS Float) AS Float
math::asin(value AS Fixed) AS Fixed
math::asin(values AS List OF Float) AS List OF Float
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

`math::asin` returns the angle in radians whose sine is `value`. The result is
the inverse of `math::sin` restricted to a single branch and always lies in
`[-pi/2, pi/2]`. `asin(0)` is zero, `asin(1)` is `pi/2`, and `asin(-1)` is
`-pi/2`.

The arc sine is defined only on the closed interval `[-1, 1]`. A `value` outside
that domain fails, and the two overloads report it with **different** error
codes: the `Float` overload raises `ErrFloatDomain`, the `Fixed` overload raises
`ErrInvalidArgument`.
[[src/target/shared/code/builder_simd_float_math.rs:FloatKernel]]
[[src/target/shared/code/builder_fixed_math.rs:emit_fixed_asin]]

The `Float` overload evaluates `asin(x) = atan(x / sqrt(1 - x^2))`; the divide
yields an infinity exactly at `x = +/-1`, which `atan` maps back to `+/-pi/2`, so
the endpoints come out exact. The `Fixed` overload uses the equivalent
`atan2(x, sqrt(1 - x^2))` in Q32.32.
[[src/target/shared/code/builder_fixed_math.rs:emit_fixed_asin]]

`math::asin` accepts `Float` or `Fixed` **only**. `Integer` is not accepted and
neither is `Money`: the transcendental functions do not take part in the `Money`
dimension, so `math::asin(1)` and `math::asin(1.00m)` are both compile-time errors.
Convert explicitly first. [[src/builtins/math.rs:one_float_or_fixed]]

The `Float` overload is computed by a hand-written **in-tree kernel**: MFBASIC
never links or calls a platform math library, so the answer does not vary with
the host `libm`. The scalar and array overloads share that one kernel, so
`math::asin(x)` and the corresponding element of the array form are bit-identical.
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
array overload for `math::asin` — a `Fixed` list is a compile-time error.
[[src/builtins/math.rs:resolve_call]]

## Overloads

**`math::asin(value AS Float) AS Float`**

Hand-written in-tree kernel, within 1 ULP of macOS `libm`. Fails with `ErrFloatDomain` when `|value| > 1`.

**`math::asin(value AS Fixed) AS Fixed`**

Deterministic Q32.32 `atan2(x, sqrt(1 - x^2))`, rounded to the nearest representable `Fixed`. Fails with `ErrInvalidArgument` when `|value| > 1`.

**`math::asin(values AS List OF Float) AS List OF Float`**

Applies `math::asin` to every element and returns a new `List OF Float` of the same
length. Each element equals the corresponding scalar result exactly.
A per-element failure is accumulated across the whole list and reported once
after every element has been processed, so the reported error does not depend on
which element triggered it; no list is returned in that case.
[[src/target/shared/code/builder_simd_float_math.rs:lower_simd_float_unary]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `Float` or `Fixed` | The sine value whose angle is wanted. Must lie in the closed interval `[-1, 1]`. This parameter has no alternate name. [[src/builtins/math.rs:call_param_names]] |
| `values` | `List OF Float` | The array form: mapped element-wise. The empty list yields an empty list. [[src/builtins/math.rs:resolve_call]] |

## Return value

| Type | Description |
| --- | --- |
| Same as the argument | The arc sine of `value` in radians, in the argument's own type, within `[-pi/2, pi/2]`. The array form returns a new `List OF Float` of the same length. [[src/builtins/math.rs:resolve_call]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050012` | `ErrFloatDomain` | The `Float` overload only: `|value| > 1`, outside the arc sine's domain. [[src/target/shared/code/error_constants.rs:ERR_FLOAT_DOMAIN_CODE]] |
| `77050002` | `ErrInvalidArgument` | The `Fixed` overload only: `|value| > 1`. [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] |

## Type checking

`math::asin` takes exactly one argument. [[src/builtins/math.rs:arity]] It must be
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
  LET a AS Float = math::asin(0.5)
  LET b AS Fixed = math::asin(1.0F)
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
  LET results AS List OF Float = math::asin(angles)
  io::print(toString(collections::get(results, 0)))
END SUB
```

## See also

- `mfb man math acos`
- `mfb man math atan`
- `mfb man math atan2`
- `mfb man math sin`
- `mfb man math`
