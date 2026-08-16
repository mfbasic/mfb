# parse

Parse UTF-8 CSV text into a grid of String cells.

## Synopsis

```
csv::parse(value AS String) AS List OF List OF String
csv::parse(value AS String, delimiter AS String, quote AS String) AS List OF List OF String
```

## Package

csv

## Imports

```
IMPORT csv
```

`csv` is a built-in package, so `IMPORT csv` needs no manifest dependency. [[src/codegen/builtins/csv/mod.rs:CSV]]

## Description

`csv::parse` scans `value` left to right and returns the resulting document as a
`List OF List OF String`: an ordered list of rows, each an ordered list of String
cells. Internally the text is decoded to its Unicode scalars in one pass
(`encoding::utf32Encode`) and scanned scalar by scalar, so the scanner never
splits a multi-byte code point or a `\r\n` pair incorrectly; each field is
accumulated in a scalar buffer and re-encoded to a String with
`encoding::utf32Decode`. Every structural CSV character (comma, quote, CR, LF) is
ASCII, so the resulting grid is byte-identical to a grapheme-based scan. [[src/codegen/builtins/csv/func_parse.rs:__csv_parse]]

The dialect is RFC-4180-aligned. The field delimiter defaults to a comma (scalar
`44`) but can be overridden with the optional `delimiter` argument; the quote
character defaults to the double quote (`34`) but can be overridden with the
optional `quote` argument. Each must be a non-empty single character, and only its
first Unicode scalar is used. A record separator is a line feed (LF, `10`) or a
carriage-return/line-feed pair (CRLF, `13` then `10`) regardless of dialect; a
bare CR not followed by LF is ordinary data inside the current field. A field may
be wrapped in the quote character: the opening quote must be the first character
of the field, inside a quoted field a literal quote is written by doubling it, and
delimiters, CR, and LF are ordinary data. The closing quote must be immediately
followed by the delimiter, a record separator, or the end of input. Whitespace is
significant and never trimmed. [[src/codegen/builtins/csv/package.mfb:__csv_separatorLength]]

Cells are plain Strings with no type inference and no null: `42`, `true`, and an
empty field parse to the Strings `"42"`, `"true"`, and `""`. Callers that want
numbers convert explicitly with `toFloat` or `toInteger`. Rows are not required
to be rectangular; each row keeps whatever field count it had. A single trailing
record separator does not create an empty final row, so `"a\nb\n"` parses to two
rows, while two consecutive separators do produce an empty row in the middle.
Empty input parses to zero rows. There is no header concept — every parsed line
is an ordinary row, and cells are read positionally with `collections::get`. [[src/codegen/builtins/csv/func_parse.rs:__csv_parse]]

The argument may also be supplied by the name `text`. `csv::parse` does not
mutate `value` and has no side effects. [[src/codegen/registry/mod.rs:call_param_names]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `String` | The UTF-8 CSV text to parse. May also be passed by the name `text`. It is never modified. [[src/codegen/registry/mod.rs:call_param_names]] |
| `delimiter` | `String` | Optional. The single character that separates fields. Defaults to `,`. [[src/codegen/registry/mod.rs:default_argument_padding]] |
| `quote` | `String` | Optional. The single character that wraps a field and, doubled, escapes itself. Defaults to `"`. [[src/codegen/registry/mod.rs:default_argument_padding]] |

## Return value

| Type | Description |
| --- | --- |
| `List OF List OF String` | The grid of rows of String cells, in document order. Empty input yields an empty list; a single trailing record separator does not add an empty final row. [[src/codegen/builtins/csv/mod.rs:register]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050003` | `ErrInvalidFormat` | A quoted field is opened but never closed before the end of input; the closing quote of a quoted field is followed by a grapheme that is neither the delimiter, a record separator, nor the end of input; or a supplied `delimiter`/`quote` is the empty String. [[src/codegen/builtins/csv/func_parse.rs:__csv_parse]] [[src/codegen/builtins/csv/package.mfb:__csv_firstCode]] [[src/codegen/builtins/errorcode/mod.rs:ErrInvalidFormat]] |

## Examples

Parse a two-column document with a quoted cell:

```
IMPORT csv

SUB main()
  LET doc AS List OF List OF String = csv::parse("name,age\nAda,36")
END SUB
```

Pass the argument by name:

```
IMPORT csv

SUB main()
  LET rows AS List OF List OF String = csv::parse(text := "a,b,c")
END SUB
```

## See also

- `mfb man csv stringify`
- `mfb man csv`
- `mfb man collections get`
