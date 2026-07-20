# isWhitespace

Test whether a Unicode scalar is whitespace.

## Synopsis

```
strings::isWhitespace(scalar AS Scalar) AS Boolean
```

## Package

strings

## Imports

```
IMPORT strings
```

`strings` is a built-in package, so no manifest dependency is required.
`isWhitespace` is one of seven `strings` members implemented in MFBASIC source
rather than in native codegen; the companion is injected automatically when a
program imports `strings` and references the scalar seam. [[src/builtins/strings.rs:implementation_name]] [[src/builtins/strings.rs:uses_package]]

## Description

`strings::isWhitespace` returns `TRUE` when `scalar` is a Unicode whitespace
scalar and `FALSE` otherwise. The set is defined as the union of three rules:
[[src/builtins/strings_package.mfb:__strings_isWhitespace]]

- any scalar whose Unicode general category is `Zs` (space separator), `Zl`
  (line separator), or `Zp` (paragraph separator) — this covers `U+0020`,
  `U+00A0`, `U+1680`, `U+2000`–`U+200A`, `U+2028`, `U+2029`, `U+202F`, `U+205F`,
  and `U+3000`;
- the C0 controls `U+0009` through `U+000D` — tab, line feed, vertical tab, form
  feed, and carriage return;
- `U+0085` NEXT LINE.

Whitespace is thus *not* a single general category: the separator categories
alone omit tab and newline, which is why the control range and `U+0085` are added
explicitly. The resulting set is exactly the Unicode `White_Space` property, and
it matches the set `strings::trim`, `strings::trimStart`, and `strings::trimEnd`
remove. [[src/target/shared/code/private/unicode.rs:emit_unicode_whitespace_branch]]

Classification is deterministic and locale-independent. The function is total: it
returns a `Boolean` for every `Scalar` and never fails.

`isWhitespace` classifies a *single* scalar. To ask a question about a whole
string, walk it with `strings::toScalars` and fold the results yourself.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `scalar` | `Scalar` | The Unicode scalar to classify. Any `Scalar` is accepted. [[src/builtins/strings.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Boolean` | `TRUE` when `scalar` is a separator (`Zs`, `Zl`, `Zp`), is in `U+0009`–`U+000D`, or is `U+0085`; `FALSE` otherwise. [[src/builtins/strings.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Classify individual scalars:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(toString(strings::isWhitespace(`\t`)))
  io::print(toString(strings::isWhitespace(` `)))
  io::print(toString(strings::isWhitespace(`x`)))
  RETURN 0
END FUNC
```

Test whether a string is entirely blank:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  MUT blank AS Boolean = TRUE
  FOR EACH sc IN strings::toScalars("  \t ")
    IF NOT strings::isWhitespace(sc) THEN
      blank = FALSE
    END IF
  NEXT
  io::print(toString(blank))
  RETURN 0
END FUNC
```

## See also

- `mfb man strings trim`
- `mfb man strings trimChars`
- `mfb man strings isLetter`
- `mfb man strings toScalars`
- `mfb man strings`
