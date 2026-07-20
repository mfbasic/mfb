# toScalar

Convert an `Integer` code point, a one-scalar `String`, or a `Byte` to a `Scalar`.

## Synopsis

```
toScalar(value AS Integer) AS Scalar
toScalar(value AS String) AS Scalar
toScalar(value AS Byte) AS Scalar
```

## Package

general

## Imports

None. `general` functions are always available without an `IMPORT` statement. [[src/builtins/general.rs:is_general_call]]

## Description

`toScalar` produces a `Scalar` — a single 32-bit Unicode scalar value — from a
Unicode code point (`Integer`), a `String` holding exactly one scalar, or a `Byte`.
Which overload is selected, and whether the conversion can fail, depends on the
argument type. [[src/builtins/general.rs:resolve_call]] [[src/target/shared/code/builder_conversions.rs:lower_to_scalar]]

The `Integer` overload interprets `value` as a Unicode code point. A valid code
point is in the range `0` through `1114111` (`U+10FFFF`) and is not a surrogate —
`55296` through `57343` (`U+D800` through `U+DFFF`). A negative value, a value above
`1114111`, or a value inside the surrogate band cannot name a scalar and fails with
`ErrInvalidArgument` rather than substituting a replacement character. [[src/target/shared/code/builder_conversions.rs:lower_to_scalar]]

The `String` overload returns the single scalar of a string that contains exactly
one Unicode scalar. A `String` is guaranteed valid UTF-8, so the decoder enforces
only "exactly one scalar": an empty string, or a string holding more than one
scalar, fails with `ErrInvalidArgument`. [[src/target/shared/code/builder_conversions.rs:emit_string_to_scalar_value]]

The `Byte` overload is infallible: every byte `0` through `255` is a valid,
non-surrogate code point (`U+0000` through `U+00FF`), so the widening never fails.
It is the inverse of `toByte(Scalar)`. [[src/target/shared/code/builder_conversions.rs:lower_to_scalar]]

`toScalar` is the narrowing counterpart of `toInt(Scalar)`, which yields the code
point, and `toString(Scalar)`, which yields the one-scalar UTF-8 string. It has no
side effects beyond producing the result and never mutates its argument.

## Overloads

**`toScalar(value AS Integer) AS Scalar`**

Interprets `value` as a Unicode code point, failing with `ErrInvalidArgument` when
`value` is negative, above `1114111`, or in the surrogate band `55296` through
`57343`.

**`toScalar(value AS String) AS Scalar`**

Returns the single scalar of a one-scalar string, failing with `ErrInvalidArgument`
when the string is empty or holds more than one scalar.

**`toScalar(value AS Byte) AS Scalar`**

Widens a `Byte` to the `Scalar` with the same code point. Never fails.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `Integer` | A Unicode code point in `0` through `1114111` (`U+10FFFF`), excluding the surrogate band `55296` through `57343` (`U+D800` through `U+DFFF`). |
| `value` | `String` | A string holding exactly one Unicode scalar; an empty or multi-scalar string is rejected. |
| `value` | `Byte` | Any byte `0` through `255`; always valid. |

## Return value

| Type | Description |
| --- | --- |
| `Scalar` | The `Scalar` naming the given code point: `toScalar(65)` is `` `A` ``, `toScalar("中")` is the CJK ideograph, and `toScalar(toByte(122))` is `` `z` ``. |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | An `Integer` value is negative, above `1114111`, or a surrogate (`55296` through `57343`), or a `String` value is empty or contains more than one scalar. The `Byte` overload never raises. [[src/target/shared/code/builder_conversions.rs:lower_to_scalar]] [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] |

## Type checking

`toScalar` accepts only `Integer`, `String`, and `Byte` values, each in the
one-argument form; any other argument type or arity is a compile-time error.
Convert unsupported values to one of these types explicitly first. [[src/builtins/general.rs:resolve_call]] [[src/builtins/general.rs:arity]]

## Examples

Convert a code point:

```
SUB main()
  LET a AS Scalar = toScalar(65)
END SUB
```

Convert a one-scalar string, handling the fallible cases:

```
IMPORT io

SUB main()
  LET s AS Scalar = toScalar("中") TRAP(err)
    io::print("not a single scalar")
    RECOVER `?`
  END TRAP
END SUB
```

Widen a Byte without failure:

```
SUB main()
  LET b AS Scalar = toScalar(toByte(122))
END SUB
```

## See also

- `mfb man general toInt`
- `mfb man general toString`
- `mfb man general toByte`
- `mfb man general typeName`
