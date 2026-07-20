# sqrt

Non-negative square root of a `Float` or `Fixed`.

## Synopsis

```
math::sqrt(value AS Float) AS Float
math::sqrt(value AS Fixed) AS Fixed
math::sqrt(values AS List OF Float) AS List OF Float
math::sqrt(values AS List OF Fixed) AS List OF Fixed
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

`math::sqrt` returns the non-negative square root of `value`. `sqrt(0)` is
zero and `sqrt(1)` is one.

Only a non-negative argument has a real square root, so a negative `value` fails
rather than producing a NaN. Note that the two overloads report that failure with
**different** error codes: the `Float` overload raises `ErrFloatDomain`, the
`Fixed` overload raises `ErrInvalidArgument`.
[[src/target/shared/code/builder_math.rs:lower_math_sqrt]]

The `Float` overload is the hardware `fsqrt` instruction, which is IEEE-exact —
correctly rounded, 0 ULP — and is guarded by a comparison against zero that also
catches a NaN argument, so `math::sqrt` of a NaN raises `ErrFloatDomain` too.
[[src/target/shared/code/builder_math.rs:lower_math_sqrt]] The `Fixed` overload
is a digit-by-digit restoring integer square root over the raw Q32.32 radicand.
[[src/target/shared/code/builder_fixed_math.rs:emit_fixed_sqrt]]

`math::sqrt` accepts `Float` or `Fixed` **only**. `Integer` is not accepted and
neither is `Money`: the transcendental functions do not take part in the `Money`
dimension, so `math::sqrt(2)` and `math::sqrt(2.00m)` are both compile-time errors.
Convert explicitly first. [[src/builtins/math.rs:one_float_or_fixed]]

The `Float` overload is computed by a hand-written **in-tree kernel**: MFBASIC
never links or calls a platform math library, so the answer does not vary with
the host `libm`. Accuracy is within 1 ULP of macOS `libm`. The scalar and array
overloads share that one kernel, so `math::sqrt(x)` and the corresponding element
of `math::sqrt` over a one-element list are bit-identical. Across macOS,
Linux-glibc, and Linux-musl, and across the aarch64, x86-64, and riscv64
backends, a `Float` result is bit-identical in practice — though, unlike `Fixed`,
that is a uniform-lowering policy rather than a contractual guarantee.
[[src/docs/spec/architecture/18_math-kernels.md]]

The `Fixed` overload runs a deterministic raw Q32.32 routine with no
floating-point step anywhere, so its result is bit-identical on every target
**by construction** — for `Fixed` that cross-target identity is a contractual
guarantee. [[src/docs/spec/architecture/18_math-kernels.md]]

The array overloads map `math::sqrt` over a list and return a new list of the same
element type and length; the input list is not mutated. Both a `List OF Float` and a `List OF Fixed` array overload exist.
[[src/builtins/math.rs:resolve_call]]

## Overloads

**`math::sqrt(value AS Float) AS Float`**

Hardware `fsqrt`, correctly rounded. Fails with `ErrFloatDomain` when `value` is negative or a NaN.

**`math::sqrt(value AS Fixed) AS Fixed`**

Deterministic Q32.32 restoring square root, rounded to the nearest representable `Fixed`. Fails with `ErrInvalidArgument` when `value` is negative.

**`math::sqrt(values AS List OF Float) AS List OF Float`**
**`math::sqrt(values AS List OF Fixed) AS List OF Fixed`**

Applies `math::sqrt` to every element and returns a new list of the same element
type and length. Each element equals the corresponding scalar result exactly.
A per-element failure is accumulated across the whole list and reported once
after every element has been processed, so the reported error does not depend on
which element triggered it; no list is returned in that case.
[[src/target/shared/code/builder_simd_float_math.rs:lower_simd_float_unary]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `Float` or `Fixed` | The number whose square root is taken. Must be zero or positive. This parameter has no alternate name. [[src/builtins/math.rs:call_param_names]] |
| `values` | `List OF Float` or `List OF Fixed` | The array form: a homogeneous list, mapped element-wise. The empty list yields an empty list. [[src/builtins/math.rs:resolve_call]] |

## Return value

| Type | Description |
| --- | --- |
| Same as the argument | The non-negative square root of `value`, in the argument's own type. Zero returns zero. The array forms return a new list of the same element type and length. [[src/builtins/math.rs:resolve_call]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050012` | `ErrFloatDomain` | The `Float` overload only: `value` is negative or a NaN, so it has no real square root. [[src/target/shared/code/error_constants.rs:ERR_FLOAT_DOMAIN_CODE]] |
| `77050002` | `ErrInvalidArgument` | The `Fixed` overload only: `value` is negative. [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] |

## Type checking

`math::sqrt` takes exactly one argument. [[src/builtins/math.rs:arity]] It must be
a single `Float` or `Fixed`, or `List OF Float` or `List OF Fixed`. An `Integer`, a `Money`, or any
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
  LET a AS Float = math::sqrt(9.0)
  LET b AS Fixed = math::sqrt(4.0F)
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
  LET samples AS List OF Float = [9.0, 9.0]
  LET results AS List OF Float = math::sqrt(samples)
  io::print(toString(collections::get(results, 0)))
END SUB
```

## See also

- `mfb man math pow`
- `mfb man math exp`
- `mfb man math log`
- `mfb man math abs`
- `mfb man math`
