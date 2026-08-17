# parseStream

Open a streaming reader over UTF-8 CSV text.

## Synopsis

```
csv::parseStream(value AS String) AS CsvReader
csv::parseStream(value AS String, delimiter AS String, quote AS String) AS CsvReader
```

## Package

csv

## Imports

```
IMPORT csv
```

`csv` is a built-in package, so `IMPORT csv` needs no manifest dependency. [[src/codegen/builtins/csv/mod.rs:CSV]]

## Description

`csv::parseStream` returns a `CsvReader` — a value holding the decoded input and a
scan cursor — without parsing any rows yet. Each subsequent `csv::readRow` parses
exactly one record and returns it with the reader advanced, so a document is
processed one row at a time and the whole `List OF List OF String` grid is never
materialized. The rows a `parseStream`/`readRow` loop yields are identical to
`csv::parse(value)`. [[src/codegen/builtins/csv/func_parse_stream.rs:__csv_parseStream]]

The optional `delimiter` and `quote` select the input dialect exactly as for
`csv::parse` (defaults `,` and `"`); each must be a non-empty single character.
The output-only dialect option (`newline`) does not apply to reading. [[src/codegen/registry/mod.rs:default_argument_padding]]

The argument may also be supplied by the name `text`. `csv::parseStream` does not
mutate `value` and has no side effects. [[src/codegen/registry/mod.rs:call_param_names]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `String` | The UTF-8 CSV text to stream. May also be passed by the name `text`. It is never modified. [[src/codegen/registry/mod.rs:call_param_names]] |
| `delimiter` | `String` | Optional. The single character that separates fields. Defaults to `,`. [[src/codegen/registry/mod.rs:default_argument_padding]] |
| `quote` | `String` | Optional. The single character that wraps a field and, doubled, escapes itself. Defaults to `"`. [[src/codegen/registry/mod.rs:default_argument_padding]] |

## Return value

| Type | Description |
| --- | --- |
| `CsvReader` | A reader positioned at the start of `value`. Pass it to `csv::readRow` to obtain the first record. [[src/codegen/builtins/csv/package.mfb:CsvReader]] |

## Errors

No errors. (A malformed field is reported by `csv::readRow` when the record containing it is read.)

## Examples

Process a CSV document row by row without building the whole grid:

```
IMPORT csv
IMPORT collections
IMPORT io

SUB main()
  MUT row AS CsvRow = csv::readRow(csv::parseStream("a,b\nc,d"))
  WHILE row.done = FALSE
    io::print(collections::get(row.fields, 0))
    row = csv::readRow(row.reader)
  END WHILE
END SUB
```

## See also

- `mfb man csv readRow`
- `mfb man csv parse`
- `mfb man csv`
