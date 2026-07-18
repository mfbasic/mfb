# parse

Parse UTF-8 CSV text into a grid of String cells.

## Synopsis

```
csv::parse(value AS String) AS List OF List OF String
```

## Package

csv

## Imports

```
IMPORT csv
```

`csv` is a built-in package, so `IMPORT csv` needs no manifest dependency. [[src/builtins/csv.rs:call_return_type_name]]

## Description

`csv::parse` scans `value` left to right and returns the resulting document as a
`List OF List OF String`: an ordered list of rows, each an ordered list of String
cells. Internally the text is decoded to its Unicode scalars in one pass
(`encoding::utf32Encode`) and scanned scalar by scalar, so the scanner never
splits a multi-byte code point or a `\r\n` pair incorrectly; each field is
accumulated in a scalar buffer and re-encoded to a String with
`encoding::utf32Decode`. Every structural CSV character (comma, quote, CR, LF) is
ASCII, so the resulting grid is byte-identical to a grapheme-based scan. [[src/builtins/csv_package.mfb:__csv_parse]]

The dialect is RFC-4180-aligned. The field delimiter is always a comma (scalar
`44`). A record separator is a line feed (LF, `10`) or a carriage-return/line-feed
pair (CRLF, `13` then `10`); a bare CR not followed by LF is ordinary data inside
the current field. A field may be wrapped in double quotes (`34`): the opening
quote must be the first character of the field, inside a quoted field a literal
double quote is written by doubling it (`""`), and commas, CR, and LF are ordinary
data. The closing quote must be immediately followed by a comma, a record
separator, or the end of input. Whitespace is significant and never trimmed. [[src/builtins/csv_package.mfb:__csv_separatorLength]]

Cells are plain Strings with no type inference and no null: `42`, `true`, and an
empty field parse to the Strings `"42"`, `"true"`, and `""`. Callers that want
numbers convert explicitly with `toFloat` or `toInteger`. Rows are not required
to be rectangular; each row keeps whatever field count it had. A single trailing
record separator does not create an empty final row, so `"a\nb\n"` parses to two
rows, while two consecutive separators do produce an empty row in the middle.
Empty input parses to zero rows. There is no header concept — every parsed line
is an ordinary row, and cells are read positionally with `collections::get`. [[src/builtins/csv_package.mfb:__csv_parse]]

The argument may also be supplied by the name `text`. `csv::parse` does not
mutate `value` and has no side effects. [[src/builtins/csv.rs:call_param_names]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `String` | The UTF-8 CSV text to parse. May also be passed by the name `text`. It is never modified. [[src/builtins/csv.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `List OF List OF String` | The grid of rows of String cells, in document order. Empty input yields an empty list; a single trailing record separator does not add an empty final row. [[src/builtins/csv.rs:GRID_TYPE]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050003` | `ErrInvalidFormat` | A quoted field is opened but never closed before the end of input, or the closing quote of a quoted field is followed by a grapheme that is neither a comma, a record separator, nor the end of input. [[src/builtins/csv_package.mfb:__csv_parse]] [[src/target/shared/code/error_constants.rs:ERR_INVALID_FORMAT_CODE]] |

## Examples

Parse a two-column document with a quoted cell:

```
IMPORT csv
LET doc AS List OF List OF String = csv::parse("name,age\nAda,36")
```

Pass the argument by name:

```
IMPORT csv
LET rows AS List OF List OF String = csv::parse(text := "a,b,c")
```

## See also

- `mfb man csv stringify`
- `mfb man csv`
- `mfb man collections get`
