# isDigit

Test whether a Unicode scalar is a decimal digit.

## Synopsis

```
strings::isDigit(scalar AS Scalar) AS Boolean
```

## Package

strings

## Imports

```
IMPORT strings
```

`strings` is a built-in package, so no manifest dependency is required. `isDigit`
is one of seven `strings` members implemented in MFBASIC source rather than in
native codegen; the companion is injected automatically when a program imports
`strings` and references the scalar seam. [[src/builtins/strings.rs:implementation_name]] [[src/builtins/strings.rs:uses_package]]

## Description

`strings::isDigit` returns `TRUE` when `scalar` has the Unicode general category
`Nd` (decimal number) and `FALSE` otherwise.
[[src/builtins/strings_package.mfb:__strings_isDigit]]

The test is exactly `Nd`, no wider. It therefore accepts ASCII `0`–`9` and the
decimal digits of other scripts, such as the Arabic-Indic and Devanagari digits,
but it rejects other numeric scalars whose category is `Nl` (letter number, for
example Roman numerals) or `No` (other number, for example superscripts and
fractions). A scalar that "looks numeric" is not necessarily a digit by this
definition.

Classification reads the Unicode general-category table embedded in the compiler,
and is deterministic and locale-independent. The function is total: it returns a
`Boolean` for every `Scalar` and never fails.

`isDigit` classifies a *single* scalar. To ask a question about a whole string,
walk it with `strings::toScalars` and fold the results yourself; that decision is
deliberately left to the caller. Note also that a digit test is not a number
parser — use `toInt` or `toFloat` to convert text to a number.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `scalar` | `Scalar` | The Unicode scalar to classify. Any `Scalar` is accepted. [[src/builtins/strings.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Boolean` | `TRUE` when `scalar` has general category `Nd`; `FALSE` otherwise. [[src/builtins/strings.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Classify individual scalars:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(toString(strings::isDigit(`7`)))
  io::print(toString(strings::isDigit(`x`)))
  RETURN 0
END FUNC
```

Count the digits in a string:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  MUT digits AS Integer = 0
  FOR EACH sc IN strings::toScalars("a1 b2! c3")
    IF strings::isDigit(sc) THEN
      digits = digits + 1
    END IF
  NEXT
  io::print(toString(digits))
  RETURN 0
END FUNC
```

## See also

- `mfb man strings isLetter`
- `mfb man strings isWhitespace`
- `mfb man strings toScalars`
- `mfb man general toInt`
- `mfb man strings`
