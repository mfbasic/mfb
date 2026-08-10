# displayWidth

Measure the terminal column width of a string.

## Synopsis

```
strings::displayWidth(value AS String) AS Integer
```

## Package

strings

## Imports

```
IMPORT strings
```

`strings` is a built-in package, so no manifest dependency is required. [[src/builtins/strings.rs:is_strings_call]]

## Description

`strings::displayWidth` returns the number of terminal columns `value` occupies
when printed to a fixed-width (monospace) terminal. It is the sum, over the
string's extended grapheme clusters, of each cluster's display width.
[[src/target/shared/code/builder_strings_builtins.rs:lower_strings_display_width]]

Each cluster contributes `0`, `1`, or `2` columns. A cluster's width is the width
of its first non-zero-width scalar: `0` for a cluster made only of zero-width
scalars (a lone combining mark, a zero-width space, or a zero-width joiner), `2`
for a cluster led by an East Asian Wide or Fullwidth character or an
emoji-presentation symbol, and `1` for everything else. The per-scalar width
comes from the vendored utf8proc `charwidth` table.
[[src/target/shared/code/private/unicode.rs:emit_unicode_property_charwidth]]

The string is first segmented into extended grapheme clusters using the same
UAX #29 boundary rules as `strings::graphemes`, so a base letter with combining
marks, a regional-indicator flag, or a zero-width-joiner emoji family each counts
as one cluster laid out in its lead scalar's width.
[[src/target/shared/code/private/unicode.rs:emit_grapheme_break_branch]]

Display width is therefore a fourth measure, distinct from `len(value)` (Unicode
scalar values), `strings::byteLen(value)` (UTF-8 bytes), and
`strings::graphemesCount(value)` (grapheme clusters). For CJK text, emoji, or
combining sequences all four can differ: `"日本語"` is three clusters and three
scalars but six display columns, while `"café"` written with a combining accent
(`"cafe"` plus `U+0301`) is four clusters and four display columns but five
scalars.

East Asian **Ambiguous**-width characters are treated as width `1` (narrow), the
modern terminal default. The empty string yields `0`. `value` is not mutated and
the call never fails.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `String` | The string whose display width is measured. Any `String` is accepted, including the empty string. [[src/builtins/strings.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Integer` | The total number of terminal columns `value` occupies, a non-negative `Integer`. `0` for the empty string. [[src/builtins/strings.rs:STRINGS]] |

## Errors

No errors.

## Examples

Wide CJK ideographs occupy two columns each:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(toString(strings::displayWidth("日本語")))
  io::print(toString(strings::displayWidth("abc")))
  RETURN 0
END FUNC
```

Zero-width and combining scalars do not add columns:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  LET nfd AS String = "cafe" & "́"
  io::print(toString(strings::displayWidth(nfd)))
  io::print(toString(len(nfd)))
  RETURN 0
END FUNC
```

## See also

- `mfb man strings graphemesCount`
- `mfb man strings graphemes`
- `mfb man strings byteLen`
- `mfb man general len`
- `mfb man unicode`
- `mfb man strings`
