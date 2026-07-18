# toFixed

Convert text, an `Integer`, `Float`, or `Money` value to a `Fixed` value.

## Synopsis

```
toFixed(value AS String) AS Fixed
toFixed(value AS Integer) AS Fixed
toFixed(value AS Float) AS Fixed
toFixed(value AS Money) AS Fixed
```

## Package

general

## Imports

None. `general` functions are always available without an `IMPORT` statement. [[src/builtins/general.rs:is_general_call]]

## Description

`toFixed` converts a supported value to `Fixed`, MFBASIC's deterministic binary
fixed-point type. A `Fixed` value is stored as a signed 64-bit Q32.32 number: a
signed 32-bit integer part in the high half and a 32-bit fractional part in the low
half, so its representable values run from `-2147483648.0` up to just below
`2147483648.0` with a fractional resolution of `1/2^32`. Its behavior and whether it
can fail depend on the argument type, which selects the overload. [[src/target/shared/code/builder_conversions.rs:lower_to_fixed]] [[src/builtins/general.rs:resolve_call]]

The `String` overload parses decimal fixed-point text. An optional single leading
sign — `-` or `+` — is accepted, followed by one or more decimal digits and an
optional single fractional part introduced by `.`, and an optional scientific
exponent (`e` or `E`, itself optionally signed). The whole string must form one
well-formed decimal number with at least one digit, so an empty string, a lone sign,
a second `.`, surrounding whitespace, or any other stray character is rejected with
`ErrInvalidFormat`. The parsed number is rounded to the nearest representable `Fixed`
value (ties away from zero), and a magnitude too large for the `Fixed` range fails
with `ErrOverflow` rather than wrapping. [[src/target/shared/code/builder_conversions.rs:emit_parse_decimal_string_to_double]]

The `Integer` overload places a signed 64-bit `Integer` into the `Fixed` integer part
exactly. Because that integer part is only 32 bits wide, an `Integer` outside the
range `-2147483648` through `2147483647` cannot be represented and fails with
`ErrOverflow`. [[src/target/shared/code/builder_conversions.rs:emit_integer_to_fixed_value]]

The `Float` overload converts a finite 64-bit IEEE 754 binary64 value to the nearest
representable `Fixed` value (ties away from zero). The `Float` must be finite: NaN and
infinity are rejected with `ErrInvalidFormat`, and a finite value whose magnitude lies
outside the `Fixed` range fails with `ErrOverflow`. [[src/target/shared/code/builder_conversions.rs:emit_float_bits_to_fixed_value]]

The `Money` overload rescales a `Money` value's base-10 raw amount into Q32.32
(`raw * 2^32 / 100000`) exactly; a `Money` value too large for the `Fixed` 32-bit
integer part fails with `ErrOverflow`. [[src/target/shared/code/builder_conversions.rs:lower_to_fixed]]

Because `Fixed` is binary fixed-point, the stored value reflects the nearest
representable point and may differ slightly from the original decimal spelling used to
create it. `toFixed` has no side effects beyond producing the result `Fixed`; it never
mutates its argument.

## Overloads

**`toFixed(value AS String) AS Fixed`**

Parses signed decimal fixed-point text, with an optional fractional part and
scientific exponent, and rounds to the nearest representable `Fixed` value.

**`toFixed(value AS Integer) AS Fixed`**

Places a signed 64-bit `Integer` into the `Fixed` integer part exactly, provided it
fits the 32-bit integer range.

**`toFixed(value AS Float) AS Fixed`**

Converts a finite `Float` to the nearest representable `Fixed` value.

**`toFixed(value AS Money) AS Fixed`**

Rescales a `Money` value into `Fixed`, exactly when it fits the `Fixed` range.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `String` | Text holding a decimal fixed-point number: an optional single `-` or `+` sign, one or more digits, an optional `.` fractional part, and an optional signed `e`/`E` exponent. The whole string must be valid; no leading, trailing, or interior extra characters are permitted. |
| `value` | `Integer` | A signed 64-bit value to place in the `Fixed` integer part. Must fall within `-2147483648` through `2147483647`. |
| `value` | `Float` | A finite IEEE 754 binary64 value to convert to the nearest representable `Fixed` value. NaN and infinity are not accepted. |
| `value` | `Money` | A `Money` value to rescale into `Fixed`. Must fit the `Fixed` range. |

## Return value

| Type | Description |
| --- | --- |
| `Fixed` | The `Fixed` equivalent of `value`, rounded to the nearest representable point (ties away from zero). For an in-range `Integer` the conversion is exact; for `String` and `Float` input the result may differ slightly from the source when it cannot be represented exactly. |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050003` | `ErrInvalidFormat` | A `String` value is not well-formed decimal fixed-point text, or a `Float` value is NaN or infinite. [[src/target/shared/code/builder_conversions.rs:emit_invalid_format_return]] [[src/target/shared/code/error_constants.rs:ERR_INVALID_FORMAT_CODE]] |
| `77050010` | `ErrOverflow` | The value is outside the representable `Fixed` range: a `String` or `Float` whose magnitude is too large, an `Integer` outside `-2147483648` through `2147483647`, or a `Money` value too large for the `Fixed` integer part. [[src/target/shared/code/builder_conversions.rs:emit_overflow_return]] [[src/target/shared/code/error_constants.rs:ERR_OVERFLOW_CODE]] |

## Type checking

`toFixed` accepts only `String`, `Integer`, `Float`, and `Money` values; any other
argument type or arity is a compile-time error. Convert unsupported values to one of
these types explicitly first. [[src/builtins/general.rs:resolve_call]] [[src/builtins/general.rs:arity]]

## Examples

Parse decimal text:

```
LET amount AS Fixed = toFixed("2.25")
```

Parse text with a scientific exponent:

```
LET small AS Fixed = toFixed("1.5e-2")
```

Convert an Integer:

```
LET amount AS Fixed = toFixed(10)
```

Convert a Float:

```
LET amount AS Fixed = toFixed(1.5)
```

## See also

- `mfb man general toInt`
- `mfb man general toFloat`
- `mfb man general toMoney`
- `mfb man general toString`
- `mfb man general isNumeric`
