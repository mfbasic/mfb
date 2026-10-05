# CSV Dialect

The `csv` package is a pure-MFBASIC source package that converts between CSV
text and a grid of cells (`List OF List OF String`). The dialect is
RFC-4180-aligned by default: comma is the delimiter, fields may be double-quoted,
and the doubled quote character is the in-field escape for a literal quote. The
delimiter and quote character are configurable per call (plan-77 C3): `csv::parse`
and `csv::readRow` take optional `delimiter`/`quote` arguments and
`csv::stringify` takes optional `delimiter`/`quote`/`newline` arguments, each
defaulting to the RFC-4180 value (`,`, `"`, and LF). `delimiter` and `quote` must
each be a non-empty single character; only the first Unicode scalar is used.
Record separators on the parse side (LF / CRLF) are not configurable. There is no
comment syntax.
[[src/codegen/builtins/csv/helper_scan.rs:__csv_scan]] [[src/codegen/builtins/csv/func_stringify.rs:__csv_stringify]]

Input is either a `String` or an open `fs::File` (bug-809): `csv::parse` has one
overload for each, and `csv::readRow` reads one record from a `File`. All three
run the same tokenizer, `__csv_scan` (below).

This topic owns the parse/stringify *model* (grammar, separator handling, escape
rules, error conditions). The per-function API (`csv::parse`, `csv::readRow`,
`csv::stringify`) is owned by `./mfb man csv`.

## Data model

A CSV document is a rectangle of strings, but the package imposes **no** column
arity: each row carries exactly the cells the text produced. The exchange type is

```text
List OF List OF String        ' outer = rows, inner = fields, cell = String
```

Cells are plain strings with no type inference: every field is text, including
empty fields (`""`) and numeric-looking fields.

## Grammar

```text
document   := row ( separator row )* separator?
row        := field ( delimiter field )*
field      := quoted | bare
quoted     := quote ( char-except-quote | quote quote )* quote
bare       := char-except-delimiter-and-separator*
separator  := LF | CRLF
```

`delimiter` and `quote` are the dialect characters (default `,` and `"`).

Key dialect points: [[src/codegen/builtins/csv/func_parse.rs:__csv_parse]]

- **Configurable delimiter.** The field delimiter defaults to `,` (U+002C) but is
  overridable per call with `delimiter` (e.g. a tab or semicolon).
- **LF or CRLF record separators.** A record ends at `\n` or `\r\n`. A **bare CR**
  (`\r` not followed by `\n`) is **ordinary data**, copied into the field.
  [[src/codegen/builtins/csv/helper_separator_length.rs:__csv_separatorLength]]
- **Double-quote escaping.** Inside a quoted field, `""` decodes to a single `"`;
  a single `"` ends the quoted region. Outside a quote, commas, CR, and LF are
  structural; inside, they are data.
- **No trailing-empty-row.** A document that ends with one separator does **not**
  produce a final empty row.

## Parse algorithm

There is ONE tokenizer, `__csv_scan(file, text, delimCode, quoteCode,
maxRecords)`. It decodes `text` to its Unicode scalars (`encoding::utf32Encode`)
and runs a single forward scan over the scalar-code list with a small state set:
`inQuotes`, `fieldStarted`, `wasQuoted`, and `recordPending`. Whenever the list is
used up it refills it with the next line of `file` — read with the internal
`fs::readLineRaw`, which returns the line WITH its terminator (`LF` or `CRLF`, or
none for an unterminated last line) and `""` only at the end of input. `""` ends
the scan. The configured `delimiter`/`quote` are each converted to their first
scalar code once (`__csv_firstCode`). It accumulates the current field's scalars,
the current `row`, and the completed `rows`, and stops right after the record
that brings the count to `maxRecords` (`0` = no limit), before reading anything
further.
[[src/codegen/builtins/csv/helper_scan.rs:__csv_scan]] [[src/codegen/builtins/csv/helper_first_code.rs:__csv_firstCode]] [[src/codegen/builtins/fs/func_read_line_raw.rs:lower_fs_read_line_raw]]

The members are its two input adapters:

| member | `file` | `text` | `maxRecords` |
|---|---|---|---|
| `parse(String)` | an empty `fs::openData("", "r")` file — already at its end | the whole document | 0 |
| `parse(File)` | the caller's file | `""` | 0 |
| `readRow(File)` | the caller's file | the record's first raw line | 1 |

[[src/codegen/builtins/csv/func_parse.rs:__csv_parse]] [[src/codegen/builtins/csv/func_parse.rs:__csv_parseFile]] [[src/codegen/builtins/csv/func_read_row.rs:__csv_readRow]]

Because a raw line keeps its exact terminator bytes, a quoted field that spans
lines is rebuilt byte for byte (an embedded `CRLF` stays `CRLF`), and a record
read from a `File` is identical to the same record parsed from a `String`. A line
always ends at an `LF`, and an `LF` outside quotes ends the record, so a refill
only ever happens between records or inside an open quoted field — never in the
middle of an unquoted field.

The `String` adapter decodes the whole document once instead of copying it into a
`fs::openData` file and reading it back a line at a time. Both were measured on
50,000- and 200,000-row documents (macos-aarch64, best of 15 runs): the copy route
was 6–9% slower than the pre-bug-809 parse, the one-pass decode within 1.5%
(run-to-run noise at the time).

| State at cursor | Grapheme | Action |
|-----------------|----------|--------|
| `inQuotes` | `"` then `"` (doubled) | append one `"`, advance 2 |
| `inQuotes` | lone `"` | close quote, set `wasQuoted`, advance 1 |
| `inQuotes` | anything else | append to field (commas/CR/LF are data), advance 1 |
| not quoted | separator (len > 0) | flush field → row, flush row → rows, reset, advance by separator length |
| not quoted | `,` | flush field → row, set `recordPending`, advance 1 |
| not quoted | any char while `wasQuoted` | **error** — text after a closing quote |
| not quoted | `"` and `fieldStarted = FALSE` | enter `inQuotes`, advance 1 |
| not quoted | any other char | append to field, set `fieldStarted`, advance 1 |

A field may only open a quote at its **start** (`fieldStarted = FALSE`); a `"`
appearing after bare data is treated as ordinary data, not a quote opener. A
separator closes a record, and with `maxRecords` reached the scan stops there.

### Trailing record

After the loop, a final field/row is appended only if `fieldStarted` **or**
`recordPending` **or** `wasQuoted` is set — i.e. there is genuinely pending
content. This is what suppresses the trailing-empty-row when the document ends on
a clean separator (all three flags are cleared by the separator arm). A scan
stopped by `maxRecords` is always just past a separator, so it appends nothing
here either.
[[src/codegen/builtins/csv/helper_scan.rs:__csv_scan]]

### Separator length

The scan works on Unicode scalars, so a CRLF is always two entries (`13`, `10`),
never one. `__csv_separatorLength` gives the number of scalars a record separator
at the cursor occupies, and the cursor advances by exactly that:
[[src/codegen/builtins/csv/helper_separator_length.rs:__csv_separatorLength]]

| Scalar at index | Next scalar | Separator length |
|-----------------|-------------|------------------|
| `\n` (10) | — | 1 |
| `\r` (13) | `\n` | 2 (CRLF) |
| `\r` (13) | not `\n`, or none | 0 (bare CR is data) |
| anything else | — | 0 |

A raw line from a `File` ends at its LF, so a CRLF is never split across two
refills: its CR is always followed by its LF in the same line.

On the stringify side, the CR is the one-character carriage-return (U+000D) string, written
directly as a `"\r"` string literal — the lexer decodes the `\r` escape.
[[src/codegen/builtins/csv/helper_needs_quote.rs:__csv_needsQuote]]

## Errors

`__csv_scan` fails with `error(77050003, "invalid CSV format")` in two cases:
[[src/codegen/builtins/csv/helper_scan.rs:__csv_scan]]

- **Text after a closing quote** within the same field (e.g. `"a"b`) — detected by
  the `wasQuoted` arm.
- **Unterminated quote** — `inQuotes` still set when the input ends (the last
  refill returned `""`).

`csv::readRow` reads the record's first line itself and fails
`error(77020003, …)` (`ErrEndOfFile`) when it is `""` — no record is left — which
is `fs::readLine`'s end-of-input contract. Reading a `File` can also raise what
`fs::readLineRaw` raises: `ErrEncoding` for a line that is not UTF-8,
`ErrReadFailed`, `ErrResourceClosed`, `ErrResourceMoved`, `ErrOutOfMemory`.
[[src/codegen/builtins/csv/func_read_row.rs:__csv_readRow]]

## Stringify algorithm

`__csv_stringify` is the inverse for the common case but is **not** a perfect
round-trip of separators: rows are joined with a single **LF**, with **no
trailing newline**, regardless of how the input was separated.
[[src/codegen/builtins/csv/func_stringify.rs:__csv_stringify]] [[src/codegen/builtins/csv/func_stringify.rs:__csv_stringifyRow]]

Fields are joined with the `delimiter` (default `,`) and rows with the `newline`
(default LF). A field is quoted **only when it must be** — `__csv_needsQuote`
returns true when the field contains the delimiter, the quote character, a CR, or
an LF. When quoted, the field is wrapped in the quote character and every interior
occurrence of it is doubled (`__csv_quoteField`, via `strings::replace`).
[[src/codegen/builtins/csv/helper_needs_quote.rs:__csv_needsQuote]]
[[src/codegen/builtins/csv/helper_quote_field.rs:__csv_quoteField]]

```text
a,b          ' no special chars  → a,b
a "x"        ' contains a quote   → "a ""x"""
a,b (in one) ' contains a comma   → "a,b"
line1\nline2 ' contains LF        → "line1\nline2"  (quoted, LF kept verbatim)
```

A field containing only a bare CR is still quoted (CR triggers `needsQuote`), but
note that bare CR is *data* on the parse side, so stringify→parse preserves it.

## Streaming: `csv::readRow`

`csv::readRow(file [, delimiter, quote])` reads exactly one record starting at the
file's current position and leaves the file just past it: it is `__csv_scan` with
`maxRecords = 1`. The reader state is the `File` itself — its host position and
`fs::readLine`'s per-file read buffer, advanced in place — so a loop over an
n-record document reads each byte once and costs time linear in n, holding one
record at a time. A loop `WHILE NOT fs::eof(f)` over `readRow` yields exactly the
rows `parse` produces for the same text and dialect. Because the read buffer is
the one `fs::readLine` uses, `readRow`, `fs::readLine` and `parse(File)` can be
mixed on one file and each starts where the previous one stopped.
[[src/codegen/builtins/csv/func_read_row.rs:__csv_readRow]]

The pre-bug-809 streaming API (`csv::parseStream` returning a `CsvReader` value,
`readRow(CsvReader) AS CsvRow`) threaded the whole decoded input through every
call; a record with a flat `List` field stores it inline and is copied on every
store, so an n-row loop copied the input n times — measured ×4.47 time per
doubling (5,000 rows 1.2 s, 50,000 rows 137.5 s). It was removed, not kept
alongside; `rt_csv_read_row_scaling` pins the linear byte count.

## Round-trip notes

- Cell **values** round-trip (quoting is added/removed transparently).
- Record separators do **not** round-trip exactly: any CRLF in the input becomes
  LF on stringify, and a trailing separator is dropped (parse drops it; stringify
  never emits one).
- Empty input parses to an empty grid; an empty grid stringifies to `""`.

## See Also

* ./mfb man csv — the `csv::parse` / `csv::readRow` / `csv::stringify` function API
* ./mfb spec stdlib fs-memory-files — `fs::openData`, `fs::seek` and the raw line reader
* ./mfb spec stdlib json — the sibling text-format package and its escape model
* ./mfb spec unicode strings-model — `strings::graphemes` and grapheme clustering
* ./mfb spec language types — `List OF List OF String` and collection typing
