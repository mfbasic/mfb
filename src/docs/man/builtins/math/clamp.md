# clamp

Restrict a numeric value to an inclusive range.

## Synopsis

```
math::clamp(value AS Integer, low AS Integer, high AS Integer) AS Integer
math::clamp(value AS Float, low AS Float, high AS Float) AS Float
math::clamp(value AS Fixed, low AS Fixed, high AS Fixed) AS Fixed
math::clamp(value AS Money, low AS Money, high AS Money) AS Money
math::clamp(values AS List OF Integer, low AS Integer, high AS Integer) AS List OF Integer
math::clamp(values AS List OF Float, low AS Float, high AS Float) AS List OF Float
math::clamp(values AS List OF Fixed, low AS Fixed, high AS Fixed) AS List OF Fixed
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

`math::clamp` restricts `value` to the inclusive range `[low, high]`. When
`value` is below `low`, `low` is returned; when it is above `high`, `high` is
returned; otherwise `value` itself is returned unchanged. Both bounds are
inclusive, so a value exactly equal to a bound is returned as is.
[[src/target/shared/code/builder_math.rs:lower_math_clamp]]

The bounds are validated first: `low` must not be greater than `high`. An empty
range fails with `ErrInvalidArgument` rather than returning a value.
[[src/target/shared/code/builder_math.rs:lower_math_clamp]]

All three arguments must already be the same numeric type, and the return type
is that type: three `Integer`s yield an `Integer`, three `Float`s a `Float`,
three `Fixed`s a `Fixed`, and three `Money` amounts a `Money`. There is no
mixed-type or promoting overload. [[src/builtins/math.rs:resolve_call]]

`Money` stays in its dimension: a clamped amount is still an amount. `clamp` is
one of the four `math::` functions (`abs`, `min`, `max`, `clamp`) that accept and
return `Money`. [[src/builtins/math.rs:is_numeric]]

The comparisons are plain ordered comparisons — signed 64-bit for `Integer`,
`Fixed`, and `Money`, and ordered floating-point for `Float`. `clamp` is
implemented as its own bounds test rather than as a composition of `min` and
`max`, so when `value` lies inside the range it is returned *unchanged*, bit for
bit; a `Float` `value` of `-0.0` inside a `[+0.0, high]` range therefore comes
back as `-0.0`. [[src/target/shared/code/builder_math.rs:lower_math_clamp]]

The array overloads clamp every element of `values` against two **scalar**
bounds whose type is the list's element type, and return a new `List` of the
same element type and length; the input list is not mutated.
[[src/builtins/math.rs:clamp_list]] There is **no** `List OF Money` overload.
[[src/builtins/math.rs:is_numeric_list]]

## Overloads

**`math::clamp(value AS Integer, low AS Integer, high AS Integer) AS Integer`**
**`math::clamp(value AS Fixed, low AS Fixed, high AS Fixed) AS Fixed`**
**`math::clamp(value AS Money, low AS Money, high AS Money) AS Money`**

Signed 64-bit bounds check and select. Fails with `ErrInvalidArgument` when
`low > high`.

**`math::clamp(value AS Float, low AS Float, high AS Float) AS Float`**

Ordered floating-point bounds check and select. Fails with `ErrInvalidArgument`
when `low > high`. [[src/target/shared/code/builder_math.rs:lower_math_clamp]]

**`math::clamp(values AS List OF Integer, low AS Integer, high AS Integer) AS List OF Integer`**
**`math::clamp(values AS List OF Float, low AS Float, high AS Float) AS List OF Float`**
**`math::clamp(values AS List OF Fixed, low AS Fixed, high AS Fixed) AS List OF Fixed`**

Clamps each element into `[low, high]` and returns a new list of the same
element type and length. The bounds stay scalar — they are not lists. Each
element equals the corresponding scalar result exactly.
[[src/target/shared/code/builder_math.rs:lower_math_clamp_array]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `Integer`, `Float`, `Fixed`, `Money`, or a `List OF` one of the first three | The value (or list of values) to restrict. This parameter has no alternate name. [[src/builtins/math.rs:call_param_names]] |
| `low` | The element type of `value` | The inclusive lower bound. Also accepted under the name `minimum`. Must not be greater than `high`. [[src/builtins/math.rs:call_param_names]] |
| `high` | The element type of `value` | The inclusive upper bound. Also accepted under the name `maximum`. Must not be less than `low`. [[src/builtins/math.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| Same as `value` | `low` when `value < low`, `high` when `value > high`, and `value` unchanged otherwise. A value equal to either bound is returned unchanged. The array forms return a new list of the same element type and length. [[src/builtins/math.rs:resolve_call]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `low` is greater than `high`, so the range is empty. Checked before any element is clamped, in both the scalar and the array forms. [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] [[src/target/shared/code/builder_math.rs:lower_math_clamp]] |

## Type checking

`math::clamp` takes exactly three arguments. [[src/builtins/math.rs:arity]] Either
all three share one numeric type (`Integer`, `Float`, `Fixed`, or `Money`), or
the first is a `List OF T` for a numeric `T` and the other two are scalars of
that same `T`. Mixing numeric types, passing list bounds, or passing a
non-numeric value such as a `String`, `Boolean`, `Byte`, `Scalar`, record,
union, resource, thread, or function value is a compile-time type error.
[[src/builtins/math.rs:expected_arguments]]

## Examples

Clamp an `Integer` above the range down to the upper bound, and a `Float` into
the unit range:

```
IMPORT math
IMPORT io

SUB main()
  LET value AS Integer = math::clamp(12, 0, 10)
  LET ratio AS Float = math::clamp(1.4, 0.0, 1.0)
  io::print(toString(value))
  io::print(toString(ratio))
END SUB
```

Named bounds, and clamping a whole list against scalar bounds:

```
IMPORT math
IMPORT io
IMPORT collections

SUB main()
  LET capped AS Integer = math::clamp(12, minimum := 0, maximum := 10)
  io::print(toString(capped))

  LET readings AS List OF Float = [-1.0, 0.5, 4.0]
  LET bounded AS List OF Float = math::clamp(readings, 0.0, 1.0)
  io::print(toString(collections::get(bounded, 2)))
END SUB
```

## See also

- `mfb man math min`
- `mfb man math max`
- `mfb man math abs`
- `mfb man math`
