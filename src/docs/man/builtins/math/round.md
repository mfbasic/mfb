# round

Nearest integer, with exact halves rounded away from zero.

## Synopsis

```
math::round(value AS Float) AS Integer
math::round(value AS Fixed) AS Integer
math::round(value AS Money) AS Integer
math::round(values AS List OF Float) AS List OF Integer
math::round(values AS List OF Fixed) AS List OF Integer
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

`math::round` returns the integer nearest to `value`, with an exact half resolved away from zero. It rounds to the nearest integer, breaking exact halves away from zero.

The result is **always** an `Integer`, whatever the argument type — this
function converts out of the fractional type rather than rounding within it. A
value that is already a whole number is returned unchanged.
[[src/builtins/math.rs:call_return_type_name]]

`math::round(3.5)` is `4` and `math::round(-2.5)` is `-3`.

`math::round` accepts `Float`, `Fixed`, and `Money` — but **not** `Integer`,
which is already whole; `math::round(3)` is a compile-time error.
[[src/builtins/math.rs:expected_arguments]]

Applying `math::round` to a `Money` yields the nearest whole-unit count, with an exact half resolved away from zero: an `Integer`, not a
`Money`. This is a deliberate exit from the `Money` dimension — the amount-ness
is dropped, which is why the result is a plain count of whole currency units.
`round` on a `Money` always breaks an exact half away from zero — it is a presentation-like operation and deliberately ignores the program-wide rounding mode. [[src/target/shared/code/builder_math.rs:emit_money_rounding_to_integer]]

The three overloads differ in how the conversion can fail. `Fixed` is Q32.32, so
its integer part always fits in `Integer` range and the conversion is exact and
platform-independent by construction; likewise every `Money` raw divides into an
in-range whole-unit count. A `Float`, by contrast, can hold a magnitude far
outside `Integer` range, or be a NaN or an infinity, so the `Float` overload
performs an explicit range check first and fails with `ErrOverflow` when the
result would not be representable.
[[src/target/shared/code/builder_math.rs:emit_float_rounding_integer_range_check]]

The array overloads map `math::round` over a `List OF Float` or `List OF Fixed`
and return a new `List OF Integer` of the same length; the input list is not
mutated. There is no `List OF Money` array form.
[[src/builtins/math.rs:one_floatish_list]]

## Overloads

**`math::round(value AS Float) AS Integer`**

Converts a `Float`, rounds to the nearest integer, breaking exact halves away from zero. Fails with `ErrOverflow` when the result lies
outside `Integer` range, including when `value` is a NaN or an infinity.
[[src/target/shared/code/builder_math.rs:emit_float_rounding_integer_range_check]]

**`math::round(value AS Fixed) AS Integer`**

Converts a `Fixed` using raw Q32.32 integer arithmetic, so the result is
identical on every target. Every `Fixed` converts to an in-range `Integer`, so
this overload never overflows.
[[src/target/shared/code/builder_fixed_math.rs:emit_fixed_rounding_to_integer]]

**`math::round(value AS Money) AS Integer`**

Converts a `Money` amount to its whole-unit count, to the nearest whole unit. A deliberate
dimension exit; never overflows.
[[src/target/shared/code/builder_math.rs:emit_money_rounding_to_integer]]

**`math::round(values AS List OF Float) AS List OF Integer`**
**`math::round(values AS List OF Fixed) AS List OF Integer`**

Applies `math::round` to every element and returns a new `List OF Integer` of the
same length. Each element equals the corresponding scalar result exactly. A
per-element failure is accumulated across the whole list and reported once after
every element has been processed, so the reported error does not depend on which
element triggered it; no list is returned in that case.
[[src/target/shared/code/builder_math.rs:lower_math_rounding_array]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `Float`, `Fixed`, or `Money` | The number to convert. `Integer` is rejected at compile time. For the `Float` overload, a value whose result falls outside `Integer` range — including a NaN or an infinity — fails. This parameter has no alternate name. [[src/builtins/math.rs:call_param_names]] |
| `values` | `List OF Float` or `List OF Fixed` | The array form: a homogeneous list, mapped element-wise. The empty list yields an empty list. [[src/builtins/math.rs:one_floatish_list]] |

## Return value

| Type | Description |
| --- | --- |
| `Integer` | The integer nearest to `value`, with an exact half resolved away from zero. Whole-number inputs return their exact integer value. For a `Money` argument, the dimensionless count of whole currency units. [[src/builtins/math.rs:resolve_call]] |
| `List OF Integer` | For the array forms: a new list of the same length, each element the scalar result for the corresponding input. [[src/builtins/math.rs:resolve_call]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050010` | `ErrOverflow` | The `Float` overload only: the result lies outside `Integer` range, or `value` is a NaN or an infinity. The `Fixed` and `Money` overloads cannot overflow. [[src/target/shared/code/error_constants.rs:ERR_OVERFLOW_CODE]] [[src/target/shared/code/builder_math.rs:emit_float_rounding_integer_range_check]] |

## Type checking

`math::round` takes exactly one argument. [[src/builtins/math.rs:arity]] It must
be a single `Float`, `Fixed`, or `Money`, or a `List OF Float` or
`List OF Fixed`. An `Integer` argument, a `Money` list, or any non-numeric value
such as a `String`, `Boolean`, `Byte`, `Scalar`, record, union, resource,
thread, or function value is a compile-time type error.
[[src/builtins/math.rs:expected_arguments]]

## Examples

A positive and a negative value:

```
IMPORT math
IMPORT io

SUB main()
  LET up AS Integer = math::round(3.5)
  LET down AS Integer = math::round(-2.5)
  io::print(toString(up))
  io::print(toString(down))
END SUB
```

A `Money` amount converted to whole units, and a list converted element-wise:

```
IMPORT math
IMPORT io
IMPORT collections

SUB main()
  LET units AS Integer = math::round(12.75m)
  io::print(toString(units))

  LET samples AS List OF Float = [1.5, -1.5]
  LET whole AS List OF Integer = math::round(samples)
  io::print(toString(collections::get(whole, 0)))
END SUB
```

## See also

- `mfb man math floor`
- `mfb man math ceil`
- `mfb man math abs`
- `mfb man math clamp`
- `mfb man math`
