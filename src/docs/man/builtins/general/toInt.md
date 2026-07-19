# toInt

Convert text, a `Byte`, `Scalar`, `Float`, `Fixed`, or `Money` value to a signed 64-bit `Integer`.

## Synopsis

```
toInt(value AS String) AS Integer
toInt(text AS String, base AS Integer) AS Integer
toInt(value AS Byte) AS Integer
toInt(value AS Scalar) AS Integer
toInt(value AS Float) AS Integer
toInt(value AS Fixed) AS Integer
toInt(value AS Money) AS Integer
```

## Package

general

## Imports

None. `general` functions are always available without an `IMPORT` statement. [[src/builtins/general.rs:is_general_call]]

## Description

`toInt` converts a supported value to a signed 64-bit `Integer`. Its behavior and
whether it can fail depend on the argument type, which selects the overload. [[src/builtins/general.rs:resolve_call]]

The one-argument `String` overload parses base-10 integer text. An optional single
leading sign — `-` or `+` — is accepted, followed by one or more decimal digits;
the entire string must consist of a valid sign and digits, so an empty string, a
lone sign, surrounding whitespace, a decimal point, or any other character is
rejected. Parsing fails with `ErrInvalidFormat` on malformed text and with
`ErrOverflow` on a value outside the signed 64-bit range. [[src/target/shared/code/builder_conversions.rs:emit_string_to_int_value]]

The two-argument `String` overload parses `text` in an explicit radix. `base` must
be between `2` and `36` inclusive; a base outside that range fails. Digits are `0`–`9`
then `a`–`z` (case-insensitive) up to `base`, with no `0x` or `0o` prefix (callers
strip their own). The same optional single leading sign and 64-bit range bounds as
the base-10 overload apply, so `toInt(text)` and `toInt(text, 10)` parse identically.
`base` is a second built-in arity, not a defaulted parameter. [[src/target/shared/code/builder_conversions.rs:emit_string_to_int_value_base]]

The `Byte` overload widens an unsigned 8-bit `Byte` (`0` through `255`) to `Integer`.
The `Scalar` overload yields the `Scalar`'s Unicode code point as an `Integer`. Both
are width-preserving moves whose value already fits `Integer`, so they always succeed
and never fail at run time. [[src/target/shared/code/builder_conversions.rs:lower_to_int]] [[src/target/shared/code/module_analysis.rs:value_may_return_invalid_format]]

The `Float` overload converts a 64-bit IEEE 754 binary64 value to `Integer` by
truncating toward zero, discarding any fractional part. A `Float` that is NaN or
infinite fails with `ErrInvalidFormat`, and one whose truncated magnitude exceeds the
signed 64-bit range fails with `ErrOverflow`. [[src/target/shared/code/builder_conversions.rs:emit_float_to_int_value]]

The `Fixed` overload converts a `Fixed` value to `Integer` by truncating toward zero,
discarding the fractional component. The `Money` overload returns the whole-unit count
(`raw / 100000`), likewise truncated toward zero. Every `Fixed` and `Money` value
truncates into the `Integer` range, so both conversions always succeed. [[src/target/shared/code/builder_conversions.rs:emit_fixed_to_int_value]] [[src/target/shared/code/builder_conversions.rs:emit_money_to_int_value]]

`toInt` has no side effects beyond producing the result `Integer`; it never mutates
its argument.

## Overloads

**`toInt(value AS String) AS Integer`**

Parses base-10 integer text with an optional single leading `-` or `+` sign.

**`toInt(text AS String, base AS Integer) AS Integer`**

Parses `text` in `base` (`2` through `36`), digits `0`–`9` then `a`–`z`
case-insensitively.

**`toInt(value AS Byte) AS Integer`**

Widens a `Byte` (`0` through `255`) to `Integer`. Infallible.

**`toInt(value AS Scalar) AS Integer`**

Returns the `Scalar`'s Unicode code point as an `Integer`. Infallible.

**`toInt(value AS Float) AS Integer`**

Converts a `Float` to `Integer` by truncating toward zero.

**`toInt(value AS Fixed) AS Integer`**

Converts a `Fixed` value to `Integer` by truncating toward zero. Infallible.

**`toInt(value AS Money) AS Integer`**

Returns the whole-unit count of a `Money` value, truncated toward zero. Infallible.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` / `text` | `String` | Text holding an integer, optionally prefixed with a single `-` or `+` sign. The whole string must be valid; no leading, trailing, or interior extra characters are permitted. |
| `base` | `Integer` | Present only on the two-argument `String` overload. The radix to parse `text` in, from `2` through `36` inclusive. Digit characters are `0`–`9` then `a`–`z` (case-insensitive) up to `base`. |
| `value` | `Byte` | An unsigned 8-bit value in the range `0` through `255` to widen to `Integer`. |
| `value` | `Scalar` | A Unicode scalar value whose code point is returned as an `Integer`. |
| `value` | `Float` | An IEEE 754 binary64 value to truncate toward zero. |
| `value` | `Fixed` | A `Fixed` value to truncate toward zero. |
| `value` | `Money` | A `Money` value whose whole-unit count is returned, truncated toward zero. |

## Return value

| Type | Description |
| --- | --- |
| `Integer` | The `Integer` equivalent of the argument. For `Float`, `Fixed`, and `Money` inputs the fractional part is discarded by truncating toward zero, so `3.9` yields `3` and `-3.9` yields `-3`. For a `String` the parsed integer is returned; for a `Byte` the widened value; and for a `Scalar` its code point. |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050003` | `ErrInvalidFormat` | A `String` value is empty or not well-formed integer text for the requested base; for the two-argument form, `base` is outside `2` through `36`; or a `Float` value is NaN or infinite. [[src/target/shared/code/builder_conversions.rs:emit_string_to_int_value_base]] [[src/target/shared/code/error_constants.rs:ERR_INVALID_FORMAT_CODE]] |
| `77050010` | `ErrOverflow` | The value is outside the signed 64-bit `Integer` range, such as integer text that is too large or a `Float` whose truncated magnitude exceeds the range. [[src/target/shared/code/builder_conversions.rs:emit_float_to_int_value]] [[src/target/shared/code/error_constants.rs:ERR_OVERFLOW_CODE]] |

The `Byte`, `Scalar`, `Fixed`, and `Money` overloads raise no errors. [[src/target/shared/code/builder_conversions.rs:lower_to_int]]

## Type checking

`toInt` accepts only `String`, `Byte`, `Scalar`, `Float`, `Fixed`, and `Money`
values, plus the two-argument `(String, Integer)` radix form; any other argument
type or arity is a compile-time error. Convert unsupported values to one of these
types explicitly first. [[src/builtins/general.rs:resolve_call]] [[src/builtins/general.rs:arity]]

## Examples

Parse base-10 text:

```
LET value AS Integer = toInt("42")
```

Parse hexadecimal text:

```
LET value AS Integer = toInt("ff", 16)
```

Widen a Byte:

```
LET b AS Byte = 65
LET wide AS Integer = toInt(b)
```

Truncate a Float toward zero:

```
LET value AS Integer = toInt(3.9)
```

Truncate a Fixed value:

```
LET amount AS Fixed = toFixed("12.75")
LET whole AS Integer = toInt(amount)
```

## See also

- `mfb man general toFloat`
- `mfb man general toFixed`
- `mfb man general toByte`
- `mfb man general toMoney`
- `mfb man general toString`
- `mfb man general isNumeric`
