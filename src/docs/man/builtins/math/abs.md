# abs

Absolute value of a numeric value, preserving its type.

## Synopsis

```
math::abs(value AS Integer) AS Integer
math::abs(value AS Float) AS Float
math::abs(value AS Fixed) AS Fixed
math::abs(value AS Money) AS Money
math::abs(values AS List OF Integer) AS List OF Integer
math::abs(values AS List OF Float) AS List OF Float
math::abs(values AS List OF Fixed) AS List OF Fixed
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

`math::abs` returns the magnitude of `value` — the value with any negative sign
removed. A non-negative argument is returned unchanged; a negative argument is
returned negated.

The function is selected by the exact type of its argument, and the return type
is always that same type: an `Integer` yields an `Integer`, a `Float` a `Float`,
a `Fixed` a `Fixed`, and a `Money` a `Money`. There is no mixed-type or
promoting overload. [[src/builtins/math.rs:resolve_call]]

`Money` stays in its dimension here: the magnitude of an amount is still an
amount, so `math::abs` is one of the four `math::` functions (`abs`, `min`,
`max`, `clamp`) that accept and return `Money`.
[[src/builtins/math.rs:is_numeric]]

`Integer`, `Fixed`, and `Money` are all stored as a signed 64-bit value, whose
negative range extends one step further than its positive range. Negating the
minimum representable value therefore has no in-range counterpart, and the call
fails rather than wrapping. The `Float` overload instead clears the sign bit with
hardware `fabs`, which is exact for every input and cannot overflow.
[[src/target/shared/code/builder_math.rs:lower_math_abs]]

The array overloads map `abs` over a numeric `List` and return a new `List` of
the same element type and length; the input list is not mutated.
[[src/target/shared/code/builder_math.rs:lower_math_abs_array]] There is **no**
`List OF Money` overload — the array forms cover `Integer`, `Float`, and `Fixed`
only. [[src/builtins/math.rs:is_numeric_list]]

## Overloads

**`math::abs(value AS Integer) AS Integer`**

Magnitude of an `Integer`. Fails with `ErrOverflow` when `value` is the minimum
`Integer`, whose magnitude is not representable.

**`math::abs(value AS Float) AS Float`**

Magnitude of a `Float`, computed by clearing the sign bit. Never fails; `abs` of
`-0.0` is `0.0`.

**`math::abs(value AS Fixed) AS Fixed`**

Magnitude of a `Fixed`. Fails with `ErrOverflow` at the minimum `Fixed`.

**`math::abs(value AS Money) AS Money`**

Magnitude of a `Money` amount, still a `Money`. Fails with `ErrOverflow` at the
minimum representable amount. [[src/target/shared/code/builder_math.rs:lower_math_abs]]

**`math::abs(values AS List OF Integer) AS List OF Integer`**
**`math::abs(values AS List OF Float) AS List OF Float`**
**`math::abs(values AS List OF Fixed) AS List OF Fixed`**

Applies `abs` to every element and returns a new `List` of the same element type
and length. Each element equals the corresponding scalar result exactly. A
per-element failure is accumulated across the whole list and reported once after
every element has been processed, so the reported error does not depend on which
element triggered it; no list is returned in that case.
[[src/target/shared/code/builder_math.rs:lower_math_abs_array]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `Integer`, `Float`, `Fixed`, or `Money` | The number whose magnitude is taken. Every value is accepted except the minimum `Integer`/`Fixed`/`Money`. This parameter has no alternate name. [[src/builtins/math.rs:call_param_names]] |
| `values` | `List OF Integer`, `List OF Float`, or `List OF Fixed` | The array form: a homogeneous numeric list, mapped element-wise. The empty list is accepted and yields an empty list. [[src/builtins/math.rs:any_numeric_list]] |

## Return value

| Type | Description |
| --- | --- |
| Same as the argument | The non-negative magnitude of `value`, in the argument's own type. Zero (including `-0.0` for `Float`) returns zero. The array forms return a new list of the same element type and length. [[src/builtins/math.rs:resolve_call]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050010` | `ErrOverflow` | `value` is the minimum representable `Integer`, `Fixed`, or `Money` (`-9223372036854775808` as a raw 64-bit value), whose magnitude has no in-range counterpart. The `Float` overload never raises this. [[src/target/shared/code/error_constants.rs:ERR_OVERFLOW_CODE]] [[src/target/shared/code/builder_math.rs:lower_math_abs]] |

## Type checking

`math::abs` takes exactly one argument. [[src/builtins/math.rs:arity]] It must be
a single `Integer`, `Float`, `Fixed`, or `Money`, or a `List OF Integer`,
`List OF Float`, or `List OF Fixed`. Any other type — a `String`, `Boolean`,
`Byte`, `Scalar`, record, union, resource, thread, or function value, or a
`List OF Money` — is a compile-time type error.
[[src/builtins/math.rs:expected_arguments]]

## Examples

Magnitude of a negative `Integer` and a negative `Float`:

```
IMPORT math
IMPORT io

SUB main()
  LET count AS Integer = math::abs(-4)
  LET distance AS Float = math::abs(-3.5)
  io::print(toString(count))
  io::print(toString(distance))
END SUB
```

A `Money` magnitude stays `Money`, and a list is mapped element-wise:

```
IMPORT math
IMPORT io
IMPORT collections

SUB main()
  LET owed AS Money = math::abs(-4.50m)
  io::print(toString(owed))

  LET deltas AS List OF Integer = [-1, 2, -3]
  LET sizes AS List OF Integer = math::abs(deltas)
  io::print(toString(collections::get(sizes, 0)))
END SUB
```

## See also

- `mfb man math min`
- `mfb man math max`
- `mfb man math clamp`
- `mfb man math sqrt`
- `mfb man math`
