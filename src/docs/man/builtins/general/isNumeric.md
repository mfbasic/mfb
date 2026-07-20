# isNumeric

Test whether a string holds text that parses as a base-10 number.

## Synopsis

```
isNumeric(value AS String) AS Boolean
```

## Package

general

## Imports

None. `general` functions are always available without an `IMPORT` statement. [[src/builtins/general.rs:is_general_call]]

## Description

`isNumeric` reports whether `value` holds text that the native decimal parser
accepts as a real number. It returns `TRUE` when the entire string parses and
`FALSE` otherwise. The check never raises an error, so it is safe as a guard
before calling a conversion such as `toFloat` that would trap on malformed
text. [[src/target/shared/code/builder_conversions.rs:lower_is_numeric]]

The accepted grammar is a single base-10 real number spanning the whole string,
with no surrounding whitespace: [[src/target/shared/code/builder_conversions.rs:emit_parse_decimal_string_to_double]]

- An optional leading sign, either `-` or `+`.
- A run of decimal digits `0`–`9`, with at most one decimal point `.`. Digits
  may appear before the point, after it, or both, so `5`, `5.`, and `.5` all
  parse. At least one digit must be present overall.
- An optional exponent introduced by `e` or `E`, itself an optional `-`/`+` sign
  followed by at least one digit. An exponent may only follow at least one
  mantissa digit, so `e5` is rejected but `1e10` and `1.5E-3` parse.

Any other content makes `isNumeric` return `FALSE`. That includes the empty
string, a lone sign or lone `.`, surrounding or embedded whitespace, thousands
separators, a second decimal point, an exponent with no digits, and non-finite
names such as `NaN` or `Infinity` (their letters are not digits). A value whose
magnitude is too large to represent as a 64-bit double — for example `1e309`,
which parses to infinity — also yields `FALSE`, because the parsed result is
range-checked and a non-finite result is rejected. Underflow to zero (for
example `1e-400`) still yields `TRUE`. [[src/target/shared/code/builder_conversions.rs:emit_double_overflow_check]]

`isNumeric` reads only `value`; it has no side effects and never mutates its
argument.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `String` | The text to test. Any `String` is accepted; its contents alone determine the result. The whole string must parse — trailing or leading characters that are not part of the number cause `FALSE`. |

## Return value

| Type | Description |
| --- | --- |
| `Boolean` | `TRUE` when the entire string parses as a finite base-10 real number; `FALSE` for any other text, including the empty string and values that overflow to a non-finite double. |

## Errors

No errors.

## Examples

Guard a parse so a malformed string never traps:

```
SUB main()
  IF isNumeric("42") THEN
    LET value AS Float = toFloat("42")
  END IF
END SUB
```

Signs and exponents are accepted; stray characters are not:

```
SUB main()
  LET ok AS Boolean = isNumeric("-1.5e3")
  LET bad AS Boolean = isNumeric("12.x")
END SUB
```

## See also

- `mfb man general toInt`
- `mfb man general toFloat`
- `mfb man general toFixed`
