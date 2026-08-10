# csv

Parse and serialize CSV text as a grid of String cells

## Synopsis

```
IMPORT csv
csv::parse(text [, delimiter, quote])
csv::stringify(value [, delimiter, quote, newline])
csv::parseStream(text [, delimiter, quote])   ' -> CsvReader
csv::readRow(reader)                           ' -> CsvRow
```

## Description

The `csv` package converts between CSV text and a grid of rows of String cells.
`csv::parse` turns a UTF-8 `String` holding CSV text into a
`List OF List OF String`, and `csv::stringify` renders such a grid back into CSV
text. `csv` is a built-in package: `IMPORT csv` needs no manifest dependency. [[src/builtins/csv.rs:CSV]]

A whole-document CSV is exactly a `List OF List OF String`: an ordered list of
rows, each an ordered list of String cells. The parsed grid composes directly
with the `collections` package and `FOR EACH`; cells are read positionally with
`collections::get`; there is no header concept — every parsed line is an ordinary
row. [[src/builtins/csv.rs:GRID_TYPE]]

For large inputs there is a streaming alternative that never materializes the
whole grid: `csv::parseStream` returns a `CsvReader` holding the input and a scan
cursor, and each `csv::readRow` parses exactly one record and returns a `CsvRow`
(`fields AS List OF String`, `reader AS CsvReader` advanced past the record, and
`done AS Boolean`). A caller loops `WHILE row.done = FALSE` (see `mfb man csv
readRow`). The rows are identical to `csv::parse`'s. [[src/builtins/csv_package.mfb:__csv_next]]

Cells are plain Strings. There is no type inference and no null: `42`, `true`,
and an empty field are just the Strings `"42"`, `"true"`, and `""`. Callers that
want numbers convert explicitly with `toFloat` or `toInteger`. Rows are not
required to be rectangular: `csv::parse` preserves whatever field count each row
had.

The dialect is RFC-4180-aligned by default, but the field delimiter and quote
character are configurable: `parse`/`parseStream` take optional `delimiter` and
`quote`, and `stringify` also takes an optional output `newline`, each defaulting
to `,`, `"`, and LF. On input, a record separator is a line feed (LF) or a
carriage-return/line-feed pair (CRLF); a bare CR not followed by LF is ordinary
data. A field may be wrapped in the quote character, inside which a literal quote
is written by doubling it and delimiters, CR, and LF are ordinary data.
Whitespace is significant and never trimmed. A single trailing record separator does not
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
| `77050003` | `ErrInvalidFormat` | raised by `parse` when a quoted field is opened but never closed before the end of input, or when the closing quote of a quoted field is followed by a byte that is neither a comma, a record separator, nor the end of input [[src/builtins/errorcode.rs:ErrInvalidFormat]] |
