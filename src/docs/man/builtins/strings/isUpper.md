# isUpper

Test whether a Unicode scalar is an uppercase letter.

## Synopsis

```
strings::isUpper(scalar AS Scalar) AS Boolean
```

## Package

strings

## Imports

```
IMPORT strings
```

`strings` is a built-in package, so no manifest dependency is required. `isUpper`
is one of seven `strings` members implemented in MFBASIC source rather than in
native codegen; the companion is injected automatically when a program imports
`strings` and references the scalar seam. [[src/builtins/strings.rs:implementation_name]] [[src/builtins/strings.rs:uses_package]]

## Description

`strings::isUpper` returns `TRUE` when `scalar` has the Unicode general category
`Lu` (uppercase letter) and `FALSE` otherwise.
[[src/builtins/strings_package.mfb:__strings_isUpper]]

The test is exactly `Lu`, no wider. Titlecase letters (category `Lt`, such as the
digraph `ǅ`) are **not** reported as uppercase, and neither are uncased letters,
digits, punctuation, or symbols. `isUpper` is a category test, not a
"has-no-lowercase-mapping" test.

Classification reads the Unicode general-category table embedded in the compiler,
so it covers the whole code-point space rather than just ASCII, and is
deterministic and locale-independent. The function is total: it returns a
`Boolean` for every `Scalar` and never fails.

`isUpper` classifies a *single* scalar. To ask a question about a whole string,
walk it with `strings::toScalars` and fold the results yourself. To change case
rather than test it, use `strings::upper`; for caseless comparison, use
`strings::caseFold`.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `scalar` | `Scalar` | The Unicode scalar to classify. Any `Scalar` is accepted. [[src/builtins/strings.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Boolean` | `TRUE` when `scalar` has general category `Lu`; `FALSE` otherwise, including for titlecase and uncased scalars. [[src/builtins/strings.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Classify individual scalars:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(toString(strings::isUpper(`Q`)))
  io::print(toString(strings::isUpper(`q`)))
  io::print(toString(strings::isUpper(`7`)))
  RETURN 0
END FUNC
```

Count the uppercase letters in a string:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  MUT caps AS Integer = 0
  FOR EACH sc IN strings::toScalars("MFBasic")
    IF strings::isUpper(sc) THEN
      caps = caps + 1
    END IF
  NEXT
  io::print(toString(caps))
  RETURN 0
END FUNC
```

## See also

- `mfb man strings isLower`
- `mfb man strings isLetter`
- `mfb man strings upper`
- `mfb man strings caseFold`
- `mfb man strings toScalars`
- `mfb man strings`
