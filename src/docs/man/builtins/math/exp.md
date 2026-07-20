# exp

`e` raised to a `Float` or `Fixed` power.

## Synopsis

```
math::exp(value AS Float) AS Float
math::exp(value AS Fixed) AS Fixed
math::exp(values AS List OF Float) AS List OF Float
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

`math::exp` returns `e` (Euler's number, approximately `2.718281828459045`)
raised to the power `value` — the inverse of `math::log`. `exp(0)` is one,
`exp(1)` is `e`, and a negative `value` yields a positive fraction below one.

`exp` grows without bound, so a sufficiently large positive `value` drives the
result past the representable range and the call fails rather than saturating.
The two overloads report that differently: the `Float` overload saturates the
out-of-range lane to an infinity and raises `ErrFloatInf`, while the `Fixed`
overload raises `ErrOverflow` when recombining the exponent leaves `Fixed`
range. A sufficiently negative `Fixed` argument underflows quietly to zero.
[[src/target/shared/code/builder_fixed_math.rs:emit_fixed_exp]]

The `Fixed` overload range-reduces `value` by the nearest integer multiple of
`ln 2`, evaluates a Taylor series on the remainder, and recombines by scaling by
a power of two. [[src/target/shared/code/builder_fixed_math.rs:emit_fixed_exp]]

`math::exp` accepts `Float` or `Fixed` **only**. `Integer` is not accepted and
neither is `Money`: the transcendental functions do not take part in the `Money`
dimension, so `math::exp(2)` and `math::exp(2.00m)` are both compile-time errors.
Convert explicitly first. [[src/builtins/math.rs:one_float_or_fixed]]

The `Float` overload is computed by a hand-written **in-tree kernel**: MFBASIC
never links or calls a platform math library, so the answer does not vary with
the host `libm`. Accuracy is within 1 ULP of macOS `libm`. The scalar and array
overloads share that one kernel, so `math::exp(x)` and the corresponding element
of `math::exp` over a one-element list are bit-identical. Across macOS,
Linux-glibc, and Linux-musl, and across the aarch64, x86-64, and riscv64
backends, a `Float` result is bit-identical in practice — though, unlike `Fixed`,
that is a uniform-lowering policy rather than a contractual guarantee.
[[src/docs/spec/architecture/18_math-kernels.md]]

The `Fixed` overload runs a deterministic raw Q32.32 routine with no
floating-point step anywhere, so its result is bit-identical on every target
**by construction** — for `Fixed` that cross-target identity is a contractual
guarantee. [[src/docs/spec/architecture/18_math-kernels.md]]

The array overloads map `math::exp` over a list and return a new list of the same
element type and length; the input list is not mutated. There is a `List OF Float` array overload but **no** `List OF Fixed` one; `math::exp` over a `Fixed` list is a compile-time error.
[[src/builtins/math.rs:resolve_call]]

## Overloads

**`math::exp(value AS Float) AS Float`**

Hand-written in-tree kernel. Fails with `ErrFloatInf` when the result overflows to infinity and with `ErrFloatNaN` when `value` is a NaN.

**`math::exp(value AS Fixed) AS Fixed`**

Deterministic Q32.32 range reduction plus Taylor series, rounded to the nearest representable `Fixed`. Fails with `ErrOverflow` when the result leaves `Fixed` range.

**`math::exp(values AS List OF Float) AS List OF Float`**

Applies `math::exp` to every element and returns a new list of the same element
type and length. Each element equals the corresponding scalar result exactly.
A per-element failure is accumulated across the whole list and reported once
after every element has been processed, so the reported error does not depend on
which element triggered it; no list is returned in that case.
[[src/target/shared/code/builder_simd_float_math.rs:lower_simd_float_unary]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `Float` or `Fixed` | The exponent to raise `e` to. Any finite value is accepted; a large positive value whose result exceeds the type's range fails. This parameter has no alternate name. [[src/builtins/math.rs:call_param_names]] |
| `values` | `List OF Float` | The array form: a homogeneous list, mapped element-wise. The empty list yields an empty list. [[src/builtins/math.rs:resolve_call]] |

## Return value

| Type | Description |
| --- | --- |
| Same as the argument | `e` raised to `value`, in the argument's own type. Zero returns one; a negative argument returns a positive result below one. The array forms return a new list of the same element type and length. [[src/builtins/math.rs:resolve_call]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050014` | `ErrFloatInf` | The `Float` overload only: the result overflowed to an infinity, which happens for a sufficiently large positive `value`. [[src/target/shared/code/error_constants.rs:ERR_FLOAT_INF_CODE]] |
| `77050013` | `ErrFloatNaN` | The `Float` overload only: `value` is a NaN, so the result is a NaN. [[src/target/shared/code/error_constants.rs:ERR_FLOAT_NAN_CODE]] |
| `77050010` | `ErrOverflow` | The `Fixed` overload only: the result lies outside `Fixed` range. [[src/target/shared/code/error_constants.rs:ERR_OVERFLOW_CODE]] |

## Type checking

`math::exp` takes exactly one argument. [[src/builtins/math.rs:arity]] It must be
a single `Float` or `Fixed`, or `List OF Float`. An `Integer`, a `Money`, or any
non-numeric value such as a `String`, `Boolean`, `Byte`, `Scalar`, record,
union, resource, thread, or function value is a compile-time type error.
[[src/builtins/math.rs:expected_arguments]]

## Examples

The `Float` and the deterministic `Fixed` overload (note the `F` suffix, which
makes the literal a `Fixed` rather than a `Float`):

```
IMPORT math
IMPORT io

SUB main()
  LET a AS Float = math::exp(1.0)
  LET b AS Fixed = math::exp(2.0F)
  io::print(toString(a))
  io::print(toString(b))
END SUB
```

Mapped element-wise over a list:

```
IMPORT math
IMPORT io
IMPORT collections

SUB main()
  LET samples AS List OF Float = [1.0, 1.0]
  LET results AS List OF Float = math::exp(samples)
  io::print(toString(collections::get(results, 0)))
END SUB
```

## See also

- `mfb man math log`
- `mfb man math log10`
- `mfb man math pow`
- `mfb man math e`
- `mfb man math`
