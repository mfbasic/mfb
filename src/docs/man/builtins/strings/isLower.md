# isLower

Test whether a Unicode scalar is a lowercase letter.

## Synopsis

```
strings::isLower(scalar AS Scalar) AS Boolean
```

## Package

strings

## Imports

```
IMPORT strings
```

`strings` is a built-in package, so no manifest dependency is required. `isLower`
is one of seven `strings` members implemented in MFBASIC source rather than in
native codegen; the companion is injected automatically when a program imports
`strings` and references the scalar seam. [[src/builtins/strings.rs:implementation_name]] [[src/builtins/strings.rs:uses_package]]

## Description

`strings::isLower` returns `TRUE` when `scalar` has the Unicode general category
`Ll` (lowercase letter) and `FALSE` otherwise.
[[src/builtins/strings_package.mfb:__strings_isLower]]

The test is exactly `Ll`, no wider. Modifier letters (category `Lm`) and other
letters (`Lo`, which covers uncased scripts such as Han and Arabic) are **not**
reported as lowercase, and neither are digits, punctuation, or symbols.
`isLower` is a category test, not a "has-no-uppercase-mapping" test.

Classification reads the Unicode general-category table embedded in the compiler,
so it covers the whole code-point space rather than just ASCII, and is
deterministic and locale-independent. The function is total: it returns a
`Boolean` for every `Scalar` and never fails.

`isLower` classifies a *single* scalar. To ask a question about a whole string,
walk it with `strings::toScalars` and fold the results yourself. To change case
rather than test it, use `strings::lower`; for caseless comparison, use
`strings::caseFold`.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `scalar` | `Scalar` | The Unicode scalar to classify. Any `Scalar` is accepted. [[src/builtins/strings.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Boolean` | `TRUE` when `scalar` has general category `Ll`; `FALSE` otherwise, including for uncased letters such as `中`. [[src/builtins/strings.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Classify individual scalars:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(toString(strings::isLower(`q`)))
  io::print(toString(strings::isLower(`Q`)))
  io::print(toString(strings::isLower(`中`)))
  RETURN 0
END FUNC
```

Count the lowercase letters in a string:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  MUT small AS Integer = 0
  FOR EACH sc IN strings::toScalars("MFBasic")
    IF strings::isLower(sc) THEN
      small = small + 1
    END IF
  NEXT
  io::print(toString(small))
  RETURN 0
END FUNC
```

## See also

- `mfb man strings isUpper`
- `mfb man strings isLetter`
- `mfb man strings lower`
- `mfb man strings caseFold`
- `mfb man strings toScalars`
- `mfb man strings`
