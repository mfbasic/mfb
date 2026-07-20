# min

The smaller of two numeric values of the same type.

## Synopsis

```
math::min(a AS Integer, b AS Integer) AS Integer
math::min(a AS Float, b AS Float) AS Float
math::min(a AS Fixed, b AS Fixed) AS Fixed
math::min(a AS Money, b AS Money) AS Money
math::min(a AS List OF Integer, b AS List OF Integer) AS List OF Integer
math::min(a AS List OF Float, b AS List OF Float) AS List OF Float
math::min(a AS List OF Fixed, b AS List OF Fixed) AS List OF Fixed
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

`math::min` returns the smaller of `a` and `b`. When the two compare equal, that
common value is returned.

Both arguments must already be the same numeric type, and the return type is
that type: two `Integer`s yield an `Integer`, two `Float`s a `Float`, two
`Fixed`s a `Fixed`, and two `Money` amounts a `Money`. There is no mixed-type or
promoting overload — `math::min(1, 1.0)` does not compile.
[[src/builtins/math.rs:resolve_call]]

`Money` stays in its dimension: the smaller of two amounts is itself an amount.
`min` is one of the four `math::` functions (`abs`, `min`, `max`, `clamp`) that
accept and return `Money`. [[src/builtins/math.rs:is_numeric]]

The `Integer`, `Fixed`, and `Money` overloads are a signed 64-bit compare and
select. The `Float` overload uses the hardware minimum-number instruction, so
`+0.0` and `-0.0` — which compare equal — resolve to `-0.0`, and the call never
rounds, overflows, or checks a domain.
[[src/target/shared/code/builder_math.rs:lower_math_min_max]]

The array overloads pair the two lists element-wise and return a new `List` of
the same element type and length; neither input is mutated. The two lists must
have the same length. [[src/target/shared/code/builder_math.rs:lower_math_min_max_array]]
There is **no** `List OF Money` overload. [[src/builtins/math.rs:is_numeric_list]]

## Overloads

**`math::min(a AS Integer, b AS Integer) AS Integer`**
**`math::min(a AS Fixed, b AS Fixed) AS Fixed`**
**`math::min(a AS Money, b AS Money) AS Money`**

Signed 64-bit compare and select. Never fails.

**`math::min(a AS Float, b AS Float) AS Float`**

Hardware minimum-number select. Never fails; of `+0.0` and `-0.0` it yields
`-0.0`. [[src/target/shared/code/builder_math.rs:lower_math_min_max]]

**`math::min(a AS List OF Integer, b AS List OF Integer) AS List OF Integer`**
**`math::min(a AS List OF Float, b AS List OF Float) AS List OF Float`**
**`math::min(a AS List OF Fixed, b AS List OF Fixed) AS List OF Fixed`**

Element-wise minimum of two same-length lists of the same element type,
returning a new list. Each element equals the corresponding scalar result
exactly. Lists of differing length fail with `ErrInvalidArgument`.
[[src/target/shared/code/builder_math.rs:lower_math_min_max_array]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `a` | `Integer`, `Float`, `Fixed`, `Money`, or the matching `List OF` form | The first value to compare. Also accepted under the name `left`. [[src/builtins/math.rs:call_param_names]] |
| `b` | Same type as `a` | The second value to compare. Also accepted under the name `right`. Must be exactly the same type as `a`. [[src/builtins/math.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| Same as the arguments | The smaller of `a` and `b`. When they are equal, that value. For the array forms, a new list of the same element type and length holding the element-wise minimum. [[src/builtins/math.rs:resolve_call]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | Array overloads only: the two lists have different lengths. The scalar overloads raise no errors. [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] [[src/target/shared/code/builder_math.rs:lower_math_min_max_array]] |

## Type checking

`math::min` takes exactly two arguments. [[src/builtins/math.rs:arity]] They must
share one numeric type: two `Integer`s, two `Float`s, two `Fixed`s, two `Money`
amounts, or two lists of the same numeric element type. Mixing numeric types,
mixing a scalar with a list, or passing a non-numeric value such as a `String`,
`Boolean`, `Byte`, `Scalar`, record, union, resource, thread, or function value
is a compile-time type error. [[src/builtins/math.rs:expected_arguments]]

## Examples

Smaller of two `Integer`s, and of two `Float`s:

```
IMPORT math
IMPORT io

SUB main()
  LET value AS Integer = math::min(2, 4)
  LET lower AS Float = math::min(-3.5, 1.0)
  io::print(toString(value))
  io::print(toString(lower))
END SUB
```

Named arguments and the element-wise array form:

```
IMPORT math
IMPORT io
IMPORT collections

SUB main()
  LET cheapest AS Money = math::min(left := 4.50m, right := 3.25m)
  io::print(toString(cheapest))

  LET highs AS List OF Float = [3.0, 9.0]
  LET lows AS List OF Float = [5.0, 1.0]
  LET floors AS List OF Float = math::min(highs, lows)
  io::print(toString(collections::get(floors, 1)))
END SUB
```

## See also

- `mfb man math max`
- `mfb man math clamp`
- `mfb man math abs`
- `mfb man math`
