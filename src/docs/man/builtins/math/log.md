# log

Natural logarithm (base `e`) of a `Float` or `Fixed`.

## Synopsis

```
math::log(value AS Float) AS Float
math::log(value AS Fixed) AS Fixed
math::log(values AS List OF Float) AS List OF Float
math::log(values AS List OF Fixed) AS List OF Fixed
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

`math::log` returns the natural logarithm of `value` — the power to which `e`
must be raised to obtain `value`. `log(1)` is zero and `log(e)` is one.

The logarithm is defined only for a strictly positive argument, so a zero or
negative `value` fails rather than returning a negative infinity or a NaN. The
two overloads report that failure with **different** error codes: the `Float`
overload raises `ErrFloatDomain`, the `Fixed` overload raises
`ErrInvalidArgument`.
[[src/target/shared/code/builder_math.rs:lower_math_scalar_transcendental]]
[[src/target/shared/code/builder_fixed_math.rs:emit_fixed_log]]

A positive argument always succeeds. `math::log` has no overflow or
infinity failure path — the logarithm of the largest representable `Float` is
only about `709`. [[src/target/shared/code/builder_simd_float_math.rs:FloatKernel]]

The `Fixed` overload normalises `value` as `m * 2^e` with `m` in `[1, 2)`,
evaluates `ln(m)` by series expansion, and recombines as
`ln(value) = e * ln 2 + ln(m)`.
[[src/target/shared/code/builder_fixed_math.rs:emit_fixed_log]]

`math::log` accepts `Float` or `Fixed` **only**. `Integer` is not accepted and
neither is `Money`: the transcendental functions do not take part in the `Money`
dimension, so `math::log(2)` and `math::log(2.00m)` are both compile-time errors.
Convert explicitly first. [[src/builtins/math.rs:one_float_or_fixed]]

The `Float` overload is computed by a hand-written **in-tree kernel**: MFBASIC
never links or calls a platform math library, so the answer does not vary with
the host `libm`. Accuracy is within 1 ULP of macOS `libm`. The scalar and array
overloads share that one kernel, so `math::log(x)` and the corresponding element
of `math::log` over a one-element list are bit-identical. Across macOS,
Linux-glibc, and Linux-musl, and across the aarch64, x86-64, and riscv64
backends, a `Float` result is bit-identical in practice — though, unlike `Fixed`,
that is a uniform-lowering policy rather than a contractual guarantee.
[[src/docs/spec/architecture/18_math-kernels.md]]

The `Fixed` overload runs a deterministic raw Q32.32 routine with no
floating-point step anywhere, so its result is bit-identical on every target
**by construction** — for `Fixed` that cross-target identity is a contractual
guarantee. [[src/docs/spec/architecture/18_math-kernels.md]]

The array overloads map `math::log` over a list and return a new list of the same
element type and length; the input list is not mutated. Both a `List OF Float` and a `List OF Fixed` array overload exist.
[[src/builtins/math.rs:resolve_call]]

## Overloads

**`math::log(value AS Float) AS Float`**

Hand-written in-tree kernel. Fails with `ErrFloatDomain` when `value` is zero or negative. It raises no other error.

**`math::log(value AS Fixed) AS Fixed`**

Deterministic Q32.32 normalise-and-series, rounded to the nearest representable `Fixed`. Fails with `ErrInvalidArgument` when `value` is zero or negative.

**`math::log(values AS List OF Float) AS List OF Float`**
**`math::log(values AS List OF Fixed) AS List OF Fixed`**

Applies `math::log` to every element and returns a new list of the same element
type and length. Each element equals the corresponding scalar result exactly.
A per-element failure is accumulated across the whole list and reported once
after every element has been processed, so the reported error does not depend on
which element triggered it; no list is returned in that case.
[[src/target/shared/code/builder_simd_float_math.rs:lower_simd_float_unary]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `Float` or `Fixed` | The number whose natural logarithm is taken. Must be strictly positive. This parameter has no alternate name. [[src/builtins/math.rs:call_param_names]] |
| `values` | `List OF Float` or `List OF Fixed` | The array form: a homogeneous list, mapped element-wise. The empty list yields an empty list. [[src/builtins/math.rs:resolve_call]] |

## Return value

| Type | Description |
| --- | --- |
| Same as the argument | The natural logarithm of `value`, in the argument's own type. `1` returns zero. The array forms return a new list of the same element type and length. [[src/builtins/math.rs:resolve_call]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050012` | `ErrFloatDomain` | The `Float` overload only: `value` is zero or negative, so it has no real logarithm. [[src/target/shared/code/error_constants.rs:ERR_FLOAT_DOMAIN_CODE]] |
| `77050002` | `ErrInvalidArgument` | The `Fixed` overload only: `value` is zero or negative. [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] |

## Type checking

`math::log` takes exactly one argument. [[src/builtins/math.rs:arity]] It must be
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
  LET a AS Float = math::log(2.718281828459045)
  LET b AS Fixed = math::log(10.0F)
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
  LET samples AS List OF Float = [2.718281828459045, 2.718281828459045]
  LET results AS List OF Float = math::log(samples)
  io::print(toString(collections::get(results, 0)))
END SUB
```

## See also

- `mfb man math log10`
- `mfb man math exp`
- `mfb man math pow`
- `mfb man math ln2`
- `mfb man math`
