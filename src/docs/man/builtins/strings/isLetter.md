# isLetter

Test whether a Unicode scalar is a letter.

## Synopsis

```
strings::isLetter(scalar AS Scalar) AS Boolean
```

## Package

strings

## Imports

```
IMPORT strings
```

`strings` is a built-in package, so no manifest dependency is required.
`isLetter` is one of seven `strings` members implemented in MFBASIC source rather
than in native codegen; the companion is injected automatically when a program
imports `strings` and references the scalar seam. [[src/builtins/strings.rs:implementation_name]] [[src/builtins/strings.rs:uses_package]]

## Description

`strings::isLetter` returns `TRUE` when `scalar` is a Unicode letter and `FALSE`
otherwise. A scalar counts as a letter when its Unicode general category is one
of `Lu` (uppercase letter), `Ll` (lowercase letter), `Lt` (titlecase letter),
`Lm` (modifier letter), or `Lo` (other letter) — that is, any `L*` category.
[[src/builtins/strings_package.mfb:__strings_isLetter]]

Classification reads the Unicode general-category table embedded in the compiler,
so it covers the whole code-point space rather than just ASCII: `中` and `é` are
letters, while `5`, `-`, and a space are not. The test is deterministic and
locale-independent, with no language-specific tailoring.

The function is total: it returns a `Boolean` for every `Scalar` and never fails.

`isLetter` classifies a *single* scalar. To ask a question about a whole string,
walk it with `strings::toScalars` and fold the results yourself; that decision is
deliberately left to the caller, since "is this string all letters" has several
reasonable definitions.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `scalar` | `Scalar` | The Unicode scalar to classify. Any `Scalar` is accepted. [[src/builtins/strings.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Boolean` | `TRUE` when `scalar` has general category `Lu`, `Ll`, `Lt`, `Lm`, or `Lo`; `FALSE` otherwise. [[src/builtins/strings.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Classify individual scalars, including non-ASCII ones:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(toString(strings::isLetter(`A`)))
  io::print(toString(strings::isLetter(`中`)))
  io::print(toString(strings::isLetter(`5`)))
  RETURN 0
END FUNC
```

Fold the predicate over a whole string:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  MUT allLetters AS Boolean = TRUE
  FOR EACH sc IN strings::toScalars("héllo")
    IF NOT strings::isLetter(sc) THEN
      allLetters = FALSE
    END IF
  NEXT
  io::print(toString(allLetters))
  RETURN 0
END FUNC
```

## See also

- `mfb man strings isDigit`
- `mfb man strings isUpper`
- `mfb man strings isLower`
- `mfb man strings isWhitespace`
- `mfb man strings toScalars`
- `mfb man strings`
