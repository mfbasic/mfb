# stringify

Encode a grid of String cells as RFC-4180-aligned CSV text.

## Synopsis

```
csv::stringify(value AS List OF List OF String) AS String
csv::stringify(value AS List OF List OF String, delimiter AS String, quote AS String, newline AS String) AS String
```

## Package

csv

## Imports

```
IMPORT csv
```

`csv` is a built-in package, so `IMPORT csv` needs no manifest dependency. [[src/codegen/builtins/csv/mod.rs:CSV]]

## Description

`csv::stringify` renders a grid — a `List OF List OF String` of rows of String
cells — into a single CSV text. Rows are joined with one line feed (LF) with no
trailing newline, and the fields within a row are joined with a comma. Rows and
fields are emitted in list order, and the grid is not required to be rectangular:
each row keeps whatever field count it had. [[src/codegen/builtins/csv/func_stringify.rs:__csv_stringify]]

A field is emitted quoted if and only if it contains the delimiter, the quote
character, a carriage return (CR), or a line feed (LF); otherwise it is emitted
bare. Whitespace is significant and never trimmed. Inside a quoted field every
quote character is doubled, and delimiters, CR, and LF are carried through as
ordinary data. The whole String is preserved verbatim, so a multi-byte scalar is
never split. [[src/codegen/builtins/csv/package.mfb:__csv_encodeField]] [[src/codegen/builtins/csv/package.mfb:__csv_quoteField]]

The optional `delimiter`, `quote`, and `newline` arguments select a dialect:
`delimiter` replaces the comma between fields, `quote` replaces the double quote
used to wrap and escape, and `newline` replaces the LF written between rows. Each
defaults to the RFC-4180 value (`,`, `"`, and LF) when omitted, so the
one-argument form is unchanged. `delimiter` and `quote` must each be a non-empty
single character. [[src/codegen/builtins/csv/mod.rs:default_argument_padding]]

An empty outer list stringifies to the empty String. An empty row stringifies to
an empty line, so a two-element outer list containing two empty rows produces a
lone LF. Cells are written verbatim with no type interpretation: the Strings
`"42"` and `""` are emitted as the text `42` and an empty field. [[src/codegen/builtins/csv/func_stringify.rs:__csv_stringifyRow]]

For any grid `x`, `csv::parse(csv::stringify(x))` yields a grid whose cells equal
those of `x`, with one normalization: a trailing empty row produced only by
separator placement is not reintroduced, and a CRLF separator in the original
text is normalized to LF on output.

The sole argument is named `value`, so it can be supplied positionally or as the
keyword argument `value :=`. `csv::stringify` does not mutate `value` and has no
side effects. [[src/codegen/builtins/csv/mod.rs:call_param_names]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `List OF List OF String` | The grid of rows of String cells to serialize. It is never modified. [[src/codegen/builtins/csv/mod.rs:call_param_names]] |
| `delimiter` | `String` | Optional. The single character written between fields. Defaults to `,`. [[src/codegen/builtins/csv/mod.rs:default_argument_padding]] |
| `quote` | `String` | Optional. The single character used to wrap a field and, doubled, to escape itself. Defaults to `"`. [[src/codegen/builtins/csv/mod.rs:default_argument_padding]] |
| `newline` | `String` | Optional. The text written between rows. Defaults to a line feed. [[src/codegen/builtins/csv/mod.rs:default_argument_padding]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | The CSV text for `value`, with rows separated by LF and no trailing newline. An empty outer list yields the empty String. The result can be read back with `csv::parse`. [[src/codegen/builtins/csv/mod.rs:CSV]] |

## Errors

No errors.

## Examples

Serialize a grid, quoting only the cell that needs it:

```
IMPORT csv

SUB main()
  LET text AS String = csv::stringify([["name", "age"], ["Grace", "Hop,per"]])
END SUB
```

Pass the argument by name:

```
IMPORT csv

SUB main()
  LET out AS String = csv::stringify(value := [["a", "b"]])
END SUB
```

## See also

- `mfb man csv parse`
- `mfb man csv`
- `mfb man collections append`
