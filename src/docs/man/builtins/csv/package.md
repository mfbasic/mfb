# csv

Parse and serialize CSV text as a grid of String cells

## Synopsis

```
IMPORT csv
csv::parse(text)
csv::stringify(value)
```

## Description

The `csv` package converts between CSV text and a grid of rows of String cells.
`csv::parse` turns a UTF-8 `String` holding CSV text into a
`List OF List OF String`, and `csv::stringify` renders such a grid back into CSV
text. `csv` is a built-in package: `IMPORT csv` needs no manifest dependency. [[src/builtins/csv.rs:call_return_type_name]]

The package defines no new types. A CSV document is exactly a
`List OF List OF String`: an ordered list of rows, each an ordered list of String
cells. There is no wrapper record and no union, so the parsed grid composes
directly with the `collections` package and `FOR EACH`. Cells are read
positionally with `collections::get`; there are no package-specific accessors,
and there is no header concept — every parsed line is an ordinary row. [[src/builtins/csv.rs:GRID_TYPE]]

Cells are plain Strings. There is no type inference and no null: `42`, `true`,
and an empty field are just the Strings `"42"`, `"true"`, and `""`. Callers that
want numbers convert explicitly with `toFloat` or `toInteger`. Rows are not
required to be rectangular: `csv::parse` preserves whatever field count each row
had.

The dialect is RFC-4180-aligned. The field delimiter is always a comma. On
input, a record separator is a line feed (LF) or a carriage-return/line-feed
pair (CRLF); a bare CR not followed by LF is ordinary data. A field may be
wrapped in double quotes, inside which a literal double quote is written by
doubling it (`""`) and commas, CR, and LF are ordinary data. Whitespace is
significant and never trimmed. A single trailing record separator does not
create an empty final row, but two consecutive separators do produce an empty
row in the middle. Empty input parses to zero rows. [[src/builtins/csv_package.mfb:__csv_parse]]

`csv::stringify` renders deterministically: rows are joined with a single LF
with no trailing newline, fields within a row are joined with a comma, and a
field is quoted only when it contains a comma, a double quote, a CR, or an LF.
For any grid `x`, `csv::parse(csv::stringify(x))` yields a grid whose cells
equal those of `x`, except that a trailing empty row produced only by separator
placement is not reintroduced and a CRLF separator is normalized to LF. [[src/builtins/csv_package.mfb:__csv_needsQuote]]

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050003` | `ErrInvalidFormat` | raised by `parse` when a quoted field is opened but never closed before the end of input, or when the closing quote of a quoted field is followed by a byte that is neither a comma, a record separator, nor the end of input [[src/target/shared/code/error_constants.rs:ERR_INVALID_FORMAT_CODE]] |
