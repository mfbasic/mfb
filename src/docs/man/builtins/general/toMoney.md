# toMoney

Convert text, an `Integer`, `Float`, `Fixed`, or `Byte` value to a `Money` value.

## Synopsis

```
toMoney(value AS String) AS Money
toMoney(value AS Integer) AS Money
toMoney(value AS Float) AS Money
toMoney(value AS Fixed) AS Money
toMoney(value AS Byte) AS Money
```

## Package

general

## Imports

None. `general` functions are always available without an `IMPORT` statement. [[src/builtins/general.rs:is_general_call]]

## Description

`toMoney` converts a supported value to `Money`, MFBASIC's exact base-10
fixed-point financial type. A `Money` value is a signed 64-bit integer scaled by
`100000` — five decimal places — so its representable range runs from
`-92233720368547.75808` through `92233720368547.75807`, and every decimal amount
within that range is represented exactly. Because MFBASIC has no implicit numeric
conversions, `toMoney` is the only way to cross into the `Money` dimension from
another type. Its behavior and whether it can fail depend on the argument type,
which selects the overload. [[src/target/shared/code/builder_conversions.rs:lower_to_money]] [[src/builtins/general.rs:resolve_call]]

The `String` overload parses decimal text to a 64-bit float, then scales by
`100000` and rounds to the nearest `Money` raw value under the current rounding
mode (see `money::setRounding`). An optional single leading sign — `-` or `+` — is
accepted, followed by one or more decimal digits and an optional fractional part
introduced by `.`, plus an optional scientific exponent (`e` or `E`, itself
optionally signed). The whole string must form one well-formed decimal number with
at least one digit, so an empty string, a lone sign, a second `.`, surrounding
whitespace, or any stray character is rejected with `ErrInvalidFormat`. A magnitude
too large for the `Money` range fails with `ErrOverflow` rather than wrapping. [[src/target/shared/code/builder_conversions.rs:lower_to_money]] [[src/target/shared/code/builder_conversions.rs:emit_parse_decimal_string_to_double]]

The `Integer` and `Byte` overloads multiply the value by `100000` to place it at
the `Money` scale exactly. A `Byte` (`0` through `255`) is always in range, so it
never fails. An `Integer` whose scaled value overflows the signed 64-bit `Money`
range fails with `ErrOverflow`. [[src/target/shared/code/builder_conversions.rs:lower_to_money]] [[src/target/shared/code/builder_numeric.rs:emit_checked_integer_multiply]]

The `Float` overload converts a finite 64-bit IEEE 754 binary64 value to `Money`,
scaling by `100000` and rounding to five decimal places under the current rounding
mode. The `Float` must be finite: NaN and infinity are rejected with
`ErrInvalidFormat`, and a finite value whose magnitude lies outside the `Money`
range fails with `ErrOverflow`. Because `Float` is inexact, the result reflects the
nearest five-place amount to the `Float`'s approximate value. [[src/target/shared/code/builder_conversions.rs:lower_to_money]] [[src/target/shared/code/builder_money_math.rs:emit_float_finite_or_invalid]] [[src/target/shared/code/builder_money_math.rs:emit_round_double_to_money_raw]]

The `Fixed` overload rescales the binary Q32.32 value to the base-10 `Money` scale
exactly (`fixed_raw * 100000 / 2^32`, via a 128-bit intermediate); a `Fixed` whose
magnitude exceeds the `Money` range fails with `ErrOverflow`. [[src/target/shared/code/builder_conversions.rs:lower_to_money]] [[src/target/shared/code/builder_numeric.rs:emit_fixed_multiply]]

`toMoney` has no side effects beyond producing the result `Money` value; it never
mutates its argument.

## Overloads

**`toMoney(value AS String) AS Money`**

Parses signed decimal text, with an optional fractional part and scientific
exponent, and rounds to five decimal places under the current rounding mode.

**`toMoney(value AS Integer) AS Money`**

Scales a signed 64-bit `Integer` to the `Money` scale (`value * 100000`), failing
on overflow.

**`toMoney(value AS Float) AS Money`**

Converts a finite `Float`, rounding to five places under the current rounding mode.

**`toMoney(value AS Fixed) AS Money`**

Rescales a binary `Fixed` to the exact base-10 `Money` scale.

**`toMoney(value AS Byte) AS Money`**

Scales an unsigned 8-bit `Byte` to the `Money` scale (`value * 100000`), always in
range. Infallible.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `String` | Text holding a decimal number: an optional single `-` or `+` sign, one or more digits, an optional `.` fractional part, and an optional signed `e`/`E` exponent. The whole string must be valid; no leading, trailing, or interior extra characters are permitted. |
| `value` | `Integer` | A signed 64-bit whole-currency-unit amount. Its scaled value (`value * 100000`) must fall within the `Money` range. |
| `value` | `Float` | A finite IEEE 754 binary64 amount. NaN and infinity are not accepted. |
| `value` | `Fixed` | A binary fixed-point amount within the `Money` range. |
| `value` | `Byte` | An unsigned 8-bit whole-currency-unit amount (`0` through `255`). |

## Return value

| Type | Description |
| --- | --- |
| `Money` | The `Money` equivalent of `value`. `Integer`, `Byte`, and `Fixed` input within range convert exactly; `String` rounds excess precision to five places under the current mode; `Float` input is inexact and rounded under the current mode. |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050003` | `ErrInvalidFormat` | A `String` value is not well-formed decimal text, or a `Float` value is NaN or infinite. [[src/target/shared/code/builder_codegen_primitives.rs:emit_invalid_format_return]] [[src/target/shared/code/error_constants.rs:ERR_INVALID_FORMAT_CODE]] |
| `77050010` | `ErrOverflow` | The value is outside the representable `Money` range: a `String` or `Float` whose magnitude is too large, an `Integer` whose scaled value overflows 64 bits, or a `Fixed` too large for the `Money` range. [[src/target/shared/code/builder_codegen_primitives.rs:emit_overflow_return]] [[src/target/shared/code/error_constants.rs:ERR_OVERFLOW_CODE]] |

The `Byte` overload raises no errors. [[src/target/shared/code/builder_conversions.rs:lower_to_money]]

## Type checking

`toMoney` accepts only `String`, `Integer`, `Float`, `Fixed`, and `Byte` values;
any other argument type or arity is a compile-time error. Convert unsupported values
to one of these types explicitly first. [[src/builtins/general.rs:resolve_call]] [[src/builtins/general.rs:arity]]

## Examples

Parse decimal text:

```
LET price AS Money = toMoney("3.50")
```

Convert a whole-unit Integer:

```
LET total AS Money = toMoney(4)
```

Convert a Float rate result:

```
LET amount AS Money = toMoney(1.5)
```

Rescale a Fixed value:

```
LET rate AS Fixed = toFixed("12.75")
LET charged AS Money = toMoney(rate)
```

## See also

- `mfb man general toInt`
- `mfb man general toFloat`
- `mfb man general toFixed`
- `mfb man general toString`
- `mfb man money round`
- `mfb man money setRounding`
