# stringify

Encode a grid of String cells as RFC-4180-aligned CSV text.

## Synopsis

```
csv::stringify(value AS List OF List OF String) AS String
```

## Package

csv

## Imports

```
IMPORT csv
```

`csv` is a built-in package, so `IMPORT csv` needs no manifest dependency. [[src/builtins/csv.rs:call_return_type_name]]

## Description

`csv::stringify` renders a grid — a `List OF List OF String` of rows of String
cells — into a single CSV text. Rows are joined with one line feed (LF) with no
trailing newline, and the fields within a row are joined with a comma. Rows and
fields are emitted in list order, and the grid is not required to be rectangular:
each row keeps whatever field count it had. [[src/builtins/csv_package.mfb:__csv_stringify]]

A field is emitted quoted if and only if it contains a comma, a double quote, a
carriage return (CR), or a line feed (LF); otherwise it is emitted bare.
Whitespace is significant and never trimmed. Inside a quoted field every double
quote is doubled (`""`), and commas, CR, and LF are carried through as ordinary
data. Fields are processed grapheme by grapheme as UTF-8, so a multi-byte scalar
is never split. [[src/builtins/csv_package.mfb:__csv_encodeField]] [[src/builtins/csv_package.mfb:__csv_quoteField]]

An empty outer list stringifies to the empty String. An empty row stringifies to
an empty line, so a two-element outer list containing two empty rows produces a
lone LF. Cells are written verbatim with no type interpretation: the Strings
`"42"` and `""` are emitted as the text `42` and an empty field. [[src/builtins/csv_package.mfb:__csv_stringifyRow]]

For any grid `x`, `csv::parse(csv::stringify(x))` yields a grid whose cells equal
those of `x`, with one normalization: a trailing empty row produced only by
separator placement is not reintroduced, and a CRLF separator in the original
text is normalized to LF on output.

The argument may also be passed by the name `value`. `csv::stringify` does not
mutate `value` and has no side effects. [[src/builtins/csv.rs:call_param_names]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `List OF List OF String` | The grid of rows of String cells to serialize. May also be supplied by the name `value`. It is never modified. [[src/builtins/csv.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | The CSV text for `value`, with rows separated by LF and no trailing newline. An empty outer list yields the empty String. The result can be read back with `csv::parse`. [[src/builtins/csv.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Serialize a grid, quoting only the cell that needs it:

```
IMPORT csv
LET text AS String = csv::stringify([["name", "age"], ["Grace", "Hop,per"]])
```

Pass the argument by name:

```
IMPORT csv
LET out AS String = csv::stringify(value := [["a", "b"]])
```

## See also

- `mfb man csv parse`
- `mfb man csv`
- `mfb man collections append`
