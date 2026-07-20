# pow

Raise a `Float` or `Fixed` base to a power of the same type.

## Synopsis

```
math::pow(base AS Float, exponent AS Float) AS Float
math::pow(base AS Fixed, exponent AS Fixed) AS Fixed
math::pow(base AS List OF Float, exponent AS List OF Float) AS List OF Float
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

`math::pow` returns `base` raised to `exponent`. Any base raised to a zero
exponent returns one.

Both arguments must already be the same numeric type, and the return type is
that type: two `Float`s yield a `Float` and two `Fixed`s a `Fixed`. `math::pow`
accepts `Float` or `Fixed` **only** — `Integer` and `Money` are compile-time
errors, and there is no mixed-type overload, so `math::pow(2.0, 3)` does not
compile. [[src/builtins/math.rs:two_same_float_or_fixed]]

Either argument may be given by name as well as positionally: the first accepts
`base` or `value`, the second `exponent` or `power`.
[[src/builtins/math.rs:call_param_names]]

The `Float` overload is a hand-written **in-tree** fdlibm `__ieee754_pow` kernel
evaluated in log2 space; MFBASIC never links or calls a platform math library.
It accepts any base and exponent, including fractional and negative exponents,
and it follows `libm` for a negative base with an integer exponent — `pow(-2.0,
3.0)` is `-8.0`. Accuracy is within 1 ULP of macOS `libm`.
[[src/target/shared/code/builder_pow.rs:emit_pow_scalar]] A combination with no
real result, such as a negative base raised to a fractional exponent, produces a
NaN and raises `ErrFloatNaN`; a magnitude too large to represent — including
`pow(0.0, -1.0)` — produces an infinity and raises `ErrFloatInf`. Neither is
returned to the program.
[[src/target/shared/code/builder_math.rs:lower_math_scalar_binary]]

The `Fixed` overload computes a deterministic raw Q32.32 result with no
floating-point step anywhere, so it is bit-identical on every target **by
construction**. A whole-number exponent uses exact repeated multiplication and
works for any base sign, taking the reciprocal when the exponent is negative; a
fractional exponent is evaluated as `exp(exponent * ln(base))` and therefore
requires `base > 0`, failing with `ErrInvalidArgument` otherwise. Any result
that leaves `Fixed` range fails with `ErrOverflow` — and note that this includes
a zero base with a negative exponent, whose reciprocal step overflows, so
`math::pow(0.0F, -1.0F)` raises `ErrOverflow`, not `ErrInvalidArgument`.
[[src/target/shared/code/builder_fixed_math.rs:emit_fixed_pow_general]]

The array overload pairs two `List OF Float` element-wise and returns a new
`List OF Float` of the same length; neither input is mutated. The two lists must
have the same length. It runs the *same* per-element scalar kernel, so
`math::pow(x, y)` and the corresponding element of the array form are
bit-identical. There is **no** `List OF Fixed` array overload.
[[src/target/shared/code/builder_pow.rs:lower_pow_array]]

## Overloads

**`math::pow(base AS Float, exponent AS Float) AS Float`**

In-tree fdlibm `__ieee754_pow`, within 1 ULP of macOS `libm`. Fails with
`ErrFloatNaN` when the result has no real value and with `ErrFloatInf` when the
result overflows to an infinity.

**`math::pow(base AS Fixed, exponent AS Fixed) AS Fixed`**

Deterministic Q32.32: exact repeated multiplication for a whole exponent,
`exp(exponent * ln(base))` for a fractional one. Fails with `ErrInvalidArgument`
when a fractional exponent is applied to a non-positive base, and with
`ErrOverflow` when the result leaves `Fixed` range — including a zero base with a
negative exponent. [[src/target/shared/code/builder_fixed_math.rs:emit_fixed_pow_general]]

**`math::pow(base AS List OF Float, exponent AS List OF Float) AS List OF Float`**

Applies `pow` across the two lists element-wise and returns a new
`List OF Float` of the same length. Each element equals the corresponding scalar
result exactly. Lists of differing length fail with `ErrInvalidArgument` before
any element is computed. [[src/target/shared/code/builder_pow.rs:lower_pow_array]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `base` | `Float`, `Fixed`, or `List OF Float` | The number being raised to a power. Also accepted under the name `value`. Subject to the `Fixed` domain restrictions described above. [[src/builtins/math.rs:call_param_names]] |
| `exponent` | Same type as `base` | The power to raise `base` to. Also accepted under the name `power`. May be whole or fractional, positive, zero, or negative. [[src/builtins/math.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| Same as the arguments | `base` raised to `exponent`. A zero exponent returns one. The `Fixed` overload rounds to the nearest representable `Fixed`; the array form returns a new `List OF Float` of the same length. [[src/builtins/math.rs:resolve_call]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050013` | `ErrFloatNaN` | The `Float` overload only: the result has no real value, such as a negative base raised to a fractional exponent. [[src/target/shared/code/error_constants.rs:ERR_FLOAT_NAN_CODE]] [[src/target/shared/code/builder_math.rs:lower_math_scalar_binary]] |
| `77050014` | `ErrFloatInf` | The `Float` overload only: the result overflowed to an infinity, including `math::pow(0.0, -1.0)`. [[src/target/shared/code/error_constants.rs:ERR_FLOAT_INF_CODE]] [[src/target/shared/code/builder_math.rs:lower_math_scalar_binary]] |
| `77050002` | `ErrInvalidArgument` | The `Fixed` overload: a fractional exponent applied to a non-positive base, which has no real result. Also raised by the array overload when the two lists have different lengths. [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] [[src/target/shared/code/builder_fixed_math.rs:emit_fixed_pow_general]] |
| `77050010` | `ErrOverflow` | The `Fixed` overload only: the result lies outside `Fixed` range, including a zero base with a negative exponent. [[src/target/shared/code/error_constants.rs:ERR_OVERFLOW_CODE]] [[src/target/shared/code/builder_fixed_math.rs:emit_fixed_pow_general]] |

## Type checking

`math::pow` takes exactly two arguments. [[src/builtins/math.rs:arity]] They must
share one type: two `Float`s, two `Fixed`s, or two `List OF Float`. Mixing
`Float` with `Fixed`, passing an `Integer`, a `Money`, a `List OF Fixed`, or any
non-numeric value such as a `String`, `Boolean`, `Byte`, `Scalar`, record,
union, resource, thread, or function value is a compile-time type error.
[[src/builtins/math.rs:expected_arguments]]

## Examples

A `Float` power and a deterministic `Fixed` power (the `F` suffix makes a
literal a `Fixed`), including a negative base with an odd integer exponent:

```
IMPORT math
IMPORT io

SUB main()
  LET scaled AS Float = math::pow(2.0, 8.0)
  LET signed AS Float = math::pow(-2.0, 3.0)
  LET area AS Fixed = math::pow(3.0F, 2.0F)
  io::print(toString(scaled))
  io::print(toString(signed))
  io::print(toString(area))
END SUB
```

Named arguments, and the element-wise array form:

```
IMPORT math
IMPORT io
IMPORT collections

SUB main()
  LET cube AS Float = math::pow(base := 2.0, exponent := 3.0)
  io::print(toString(cube))

  LET bases AS List OF Float = [2.0, 3.0]
  LET powers AS List OF Float = [8.0, 2.0]
  LET results AS List OF Float = math::pow(bases, powers)
  io::print(toString(collections::get(results, 0)))
END SUB
```

## See also

- `mfb man math sqrt`
- `mfb man math exp`
- `mfb man math log`
- `mfb man math abs`
- `mfb man math`
