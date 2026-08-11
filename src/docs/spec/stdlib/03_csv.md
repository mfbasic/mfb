# CSV Dialect

The `csv` package is a pure-MFBASIC source package that converts between CSV
text and a grid of cells (`List OF List OF String`). The dialect is
RFC-4180-aligned by default: comma is the delimiter, fields may be double-quoted,
and the doubled quote character is the in-field escape for a literal quote. The
delimiter and quote character are configurable per call (plan-77 C3): `csv::parse`
takes optional `delimiter`/`quote` arguments and `csv::stringify` takes optional
`delimiter`/`quote`/`newline` arguments, each defaulting to the RFC-4180 value
(`,`, `"`, and LF). `delimiter` and `quote` must each be a non-empty single
character; only the first Unicode scalar is used. Record separators on the parse
side (LF / CRLF) are not configurable. There is no comment syntax.
[[src/codegen/builtins/csv/func_parse.rs:__csv_parse]] [[src/codegen/builtins/csv/func_stringify.rs:__csv_stringify]]

This topic owns the parse/stringify *model* (grammar, separator handling, escape
rules, error conditions). The per-function API (`csv::parse`, `csv::stringify`)
is owned by `./mfb man csv`.

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
  [[src/codegen/builtins/csv/package.mfb:__csv_separatorLength]]
- **Double-quote escaping.** Inside a quoted field, `""` decodes to a single `"`;
  a single `"` ends the quoted region. Outside a quote, commas, CR, and LF are
  structural; inside, they are data.
- **No trailing-empty-row.** A document that ends with one separator does **not**
  produce a final empty row.

## Parse algorithm

`__csv_parse` decodes the input to its Unicode scalars in one pass
(`encoding::utf32Encode`) and then runs a single forward scan over the scalar-code
list with a small state set: `inQuotes`, `fieldStarted`, `wasQuoted`, and
`recordPending`. The configured `delimiter`/`quote` are each converted to their
first scalar code once (`__csv_firstCode`). It accumulates the current field's
scalars, the current `row`, and the completed `rows`.
[[src/codegen/builtins/csv/func_parse.rs:__csv_parse]] [[src/codegen/builtins/csv/package.mfb:__csv_firstCode]]

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
appearing after bare data is treated as ordinary data, not a quote opener.

### Trailing record

After the loop, a final field/row is appended only if `fieldStarted` **or**
`recordPending` **or** `wasQuoted` is set — i.e. there is genuinely pending
content. This is what suppresses the trailing-empty-row when the document ends on
a clean separator (all three flags are cleared by the separator arm).
[[src/codegen/builtins/csv/func_parse.rs:__csv_parse]]

### CRLF peek caveat

The runtime grapheme splitter does **not** guarantee `\r\n` arrives as one
grapheme cluster — it may yield `\r` and `\n` as two separate graphemes. The
scanner therefore never assumes a single cursor step per record separator and
instead asks `__csv_separatorLength` for the step count:
[[src/codegen/builtins/csv/package.mfb:__csv_separatorLength]]

| Grapheme at index | Next grapheme | Separator length |
|-------------------|---------------|------------------|
| `\n` | — | 1 |
| merged `\r\n` cluster | — | 1 |
| `\r` | `\n` | 2 (peek-merged CRLF) |
| `\r` | not `\n` | 0 (bare CR is data) |
| anything else | — | 0 |

Because the separator is consumed by its measured length, a CRLF split across two
graphemes and a CRLF merged into one grapheme both advance the cursor past the
whole separator and yield identical results.

The CR grapheme is the one-character carriage-return (U+000D) string, written
directly as a `"\r"` string literal — the lexer decodes the `\r` escape.
[[src/codegen/builtins/csv/package.mfb:__csv_needsQuote]]

## Errors

`__csv_parse` fails with `error(77050003, "invalid CSV format")` in two cases:
[[src/codegen/builtins/csv/func_parse.rs:__csv_parse]]

- **Text after a closing quote** within the same field (e.g. `"a"b`) — detected by
  the `wasQuoted` arm.
- **Unterminated quote** — `inQuotes` still set at end of input.

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
[[src/codegen/builtins/csv/package.mfb:__csv_needsQuote]]
[[src/codegen/builtins/csv/package.mfb:__csv_quoteField]]

```text
a,b          ' no special chars  → a,b
a "x"        ' contains a quote   → "a ""x"""
a,b (in one) ' contains a comma   → "a,b"
line1\nline2 ' contains LF        → "line1\nline2"  (quoted, LF kept verbatim)
```

A field containing only a bare CR is still quoted (CR triggers `needsQuote`), but
note that bare CR is *data* on the parse side, so stringify→parse preserves it.

## Streaming parse

`csv::parseStream(value [, delimiter, quote])` returns a `CsvReader` value holding
the decoded input scalars, a scan cursor, and the dialect codes, without parsing
any rows. `csv::readRow(reader)` parses exactly one record from the cursor and
returns a `CsvRow { fields AS List OF String, reader AS CsvReader, done AS Boolean }`
— the record's cells, a reader advanced past it, and `done = TRUE` when the reader
was already at end of input. Reading is functional: the passed reader is never
mutated; each call threads the returned reader into the next.
[[src/codegen/builtins/csv/func_read_row.rs:__csv_next]]

The per-record scan is the identical state machine `__csv_parse` runs, so a
`WHILE row.done = FALSE` loop over `readRow` yields exactly the rows `parse`
produces for the same input and dialect — including quoted fields, doubled quotes,
CR/LF and CRLF separators, and the trailing-empty-row suppression (a reader whose
cursor sits just past the final separator returns `done` rather than an empty
row). An equivalence test pins this over a corpus. The benefit is memory: a caller
processes arbitrarily large input one row at a time without ever building the
whole `List OF List OF String`. [[src/codegen/builtins/csv/func_parse_stream.rs:__csv_parseStream]]

## Round-trip notes

- Cell **values** round-trip (quoting is added/removed transparently).
- Record separators do **not** round-trip exactly: any CRLF in the input becomes
  LF on stringify, and a trailing separator is dropped (parse drops it; stringify
  never emits one).
- Empty input parses to an empty grid; an empty grid stringifies to `""`.

## See Also

* ./mfb man csv — the `csv::parse` / `csv::stringify` function API
* ./mfb spec stdlib json — the sibling text-format package and its escape model
* ./mfb spec unicode strings-model — `strings::graphemes` and grapheme clustering
* ./mfb spec language types — `List OF List OF String` and collection typing
