# toFloat

Convert text, an `Integer`, `Fixed`, or `Money` value to a 64-bit `Float`.

## Synopsis

```
toFloat(value AS String) AS Float
toFloat(value AS Integer) AS Float
toFloat(value AS Fixed) AS Float
toFloat(value AS Money) AS Float
```

## Package

general

## Imports

None. `general` functions are always available without an `IMPORT` statement. [[src/builtins/general.rs:is_general_call]]

## Description

`toFloat` converts a supported value to a 64-bit IEEE 754 binary64 `Float`. Its
behavior and whether it can fail depend on the argument type, which selects the
overload. [[src/builtins/general.rs:resolve_call]]

The `String` overload parses decimal `Float` text. An optional single leading sign —
`-` or `+` — is accepted, followed by decimal digits, an optional `.` fractional
part, and an optional decimal exponent introduced by `e` or `E` with its own optional
sign. At least one digit is required, and the entire string must form one well-formed
decimal number, so an empty string, a lone sign, surrounding whitespace, or any other
stray character is rejected. Parsing must yield a finite value: malformed text fails
with `ErrInvalidFormat`, and text whose magnitude is too large to represent — which
would round to infinity — fails with `ErrOverflow`. [[src/target/shared/code/builder_conversions.rs:emit_parse_decimal_string_to_double]] [[src/target/shared/code/builder_conversions.rs:lower_to_float]]

The `Integer` overload converts a signed 64-bit `Integer` to the nearest representable
`Float`, rounding to nearest when the `Integer` has more significant bits than the
53-bit `Float` mantissa can hold. Every `Integer` maps to a finite `Float`, so this
conversion always succeeds. [[src/target/shared/code/builder_conversions.rs:lower_to_float]]

The `Fixed` overload converts a `Fixed` value to the nearest representable `Float`.
The `Money` overload divides the raw scaled amount by `100000` (`raw / 100000.0`),
yielding the value in whole currency units as a `Float`. Every `Fixed` and `Money`
value maps to a finite `Float`, so both conversions always succeed. [[src/target/shared/code/builder_conversions.rs:lower_to_float]]

`toFloat` has no side effects beyond producing the result `Float`; it never mutates
its argument.

## Overloads

**`toFloat(value AS String) AS Float`**

Parses a finite `Float` from decimal text, with an optional single leading `-` or `+`
sign, optional fractional part, and optional `e`/`E` exponent.

**`toFloat(value AS Integer) AS Float`**

Converts an `Integer` to the nearest representable `Float`. Infallible.

**`toFloat(value AS Fixed) AS Float`**

Converts a `Fixed` value to the nearest representable `Float`. Infallible.

**`toFloat(value AS Money) AS Float`**

Returns a `Money` value in whole currency units (`raw / 100000.0`) as the nearest
representable `Float`. Infallible.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `String` | Text holding a decimal `Float`, optionally prefixed with a single `-` or `+` sign and including an optional fractional part and `e`/`E` exponent. The whole string must be valid; no leading, trailing, or interior extra characters are permitted. |
| `value` | `Integer` | A signed 64-bit value to convert to the nearest representable `Float`. |
| `value` | `Fixed` | A `Fixed` value to convert to the nearest representable `Float`. |
| `value` | `Money` | A `Money` value whose whole-unit amount (`raw / 100000.0`) is returned as a `Float`. |

## Return value

| Type | Description |
| --- | --- |
| `Float` | The `Float` equivalent of `value`. For `String` input the parsed finite `Float` is returned. For `Integer`, `Fixed`, and `Money` input the nearest representable `Float` is returned, which may differ slightly from the source value when it cannot be represented exactly. |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050003` | `ErrInvalidFormat` | A `String` value is empty or not well-formed decimal `Float` text (bad sign, no digit, or a stray character). [[src/target/shared/code/builder_conversions.rs:emit_parse_decimal_string_to_double]] [[src/target/shared/code/error_constants.rs:ERR_INVALID_FORMAT_CODE]] |
| `77050010` | `ErrOverflow` | Parsing a `String` value yields a magnitude too large to represent, which would round to infinity. [[src/target/shared/code/builder_conversions.rs:emit_double_overflow_check]] [[src/target/shared/code/error_constants.rs:ERR_OVERFLOW_CODE]] |

The `Integer`, `Fixed`, and `Money` overloads raise no errors. [[src/target/shared/code/builder_conversions.rs:lower_to_float]]

## Type checking

`toFloat` accepts exactly one argument that is a `String`, `Integer`, `Fixed`, or
`Money` value; any other argument type or arity is a compile-time error. Convert
unsupported values to one of these types explicitly first. [[src/builtins/general.rs:resolve_call]] [[src/builtins/general.rs:arity]]

## Examples

Parse decimal text:

```
SUB main()
  LET value AS Float = toFloat("1.5")
END SUB
```

Parse text with an exponent:

```
SUB main()
  LET value AS Float = toFloat("6.022e23")
END SUB
```

Convert an Integer:

```
SUB main()
  LET value AS Float = toFloat(42)
END SUB
```

Convert a Fixed value:

```
SUB main()
  LET fixed AS Fixed = toFixed("2.25")
  LET value AS Float = toFloat(fixed)
END SUB
```

## See also

- `mfb man general toInt`
- `mfb man general toFixed`
- `mfb man general toByte`
- `mfb man general toMoney`
- `mfb man general toString`
- `mfb man general isNumeric`
