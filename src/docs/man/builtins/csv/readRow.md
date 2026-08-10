# readRow

Read the next record from a streaming CSV reader.

## Synopsis

```
csv::readRow(reader AS CsvReader) AS CsvRow
```

## Package

csv

## Imports

```
IMPORT csv
```

`csv` is a built-in package, so `IMPORT csv` needs no manifest dependency. [[src/builtins/csv.rs:CSV]]

## Description

`csv::readRow` parses exactly one record starting at `reader`'s cursor and returns
a `CsvRow` with three fields: `fields` (the record's cells, a `List OF String`),
`reader` (a new `CsvReader` advanced past the record, to pass to the next
`csv::readRow`), and `done` (`TRUE` when the reader was already at end of input,
in which case `fields` is empty). Reading is purely functional — the input
`reader` is not modified; each call returns the advanced reader to thread into the
next call. [[src/builtins/csv_package.mfb:__csv_next]]

The records `readRow` yields, in order, are identical to those `csv::parse`
produces for the same input and dialect, including the RFC-4180 rules for quoting,
doubled quotes, CR/LF and CRLF record separators, and the suppression of a
trailing empty row. The dialect is fixed when the reader is opened by
`csv::parseStream`. [[src/builtins/csv_package.mfb:__csv_next]]

`csv::readRow` has no side effects.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `reader` | `CsvReader` | A reader from `csv::parseStream` or a previous `csv::readRow`. It is never modified. [[src/builtins/csv.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `CsvRow` | `{ fields AS List OF String, reader AS CsvReader, done AS Boolean }`. When `done` is `TRUE` there was no more input and `fields` is empty. [[src/builtins/csv_package.mfb:CsvRow]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050003` | `ErrInvalidFormat` | the record being read has a quoted field that is never closed before end of input, or text after a closing quote. [[src/builtins/csv_package.mfb:__csv_next]] [[src/builtins/errorcode.rs:ErrInvalidFormat]] |

## Examples

Count the rows of a large CSV without materializing the grid:

```
IMPORT csv
IMPORT io

SUB main()
  MUT count AS Integer = 0
  MUT row AS CsvRow = csv::readRow(csv::parseStream("1,a\n2,b\n3,c"))
  WHILE row.done = FALSE
    count = count + 1
    row = csv::readRow(row.reader)
  END WHILE
  io::print("rows=" & toString(count))
END SUB
```

## See also

- `mfb man csv parseStream`
- `mfb man csv parse`
- `mfb man csv`
