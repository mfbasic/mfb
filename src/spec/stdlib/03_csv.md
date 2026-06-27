# CSV Dialect

The `csv` package is a pure-MFBASIC source package that converts between CSV
text and a grid of cells (`List OF List OF String`). The dialect is
RFC-4180-aligned: comma is the only delimiter, fields may be double-quoted, and
the doubled quote `""` is the in-field escape for a literal `"`. There is no
configurable dialect — no alternate delimiter, quote char, or comment syntax.
[[src/builtins/csv_package.mfb:__csv_parse]] [[src/builtins/csv_package.mfb:__csv_stringify]]

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
row        := field ( "," field )*
field      := quoted | bare
quoted     := '"' ( char-except-quote | '""' )* '"'
bare       := char-except-comma-and-separator*
separator  := LF | CRLF
```

Key dialect points: [[src/builtins/csv_package.mfb:__csv_parse]]

- **Comma only.** `,` (U+002C) is the sole field delimiter. No tab/semicolon
  variants.
- **LF or CRLF record separators.** A record ends at `\n` or `\r\n`. A **bare CR**
  (`\r` not followed by `\n`) is **ordinary data**, copied into the field.
  [[src/builtins/csv_package.mfb:__csv_separatorLength]]
- **Double-quote escaping.** Inside a quoted field, `""` decodes to a single `"`;
  a single `"` ends the quoted region. Outside a quote, commas, CR, and LF are
  structural; inside, they are data.
- **No trailing-empty-row.** A document that ends with one separator does **not**
  produce a final empty row.

## Parse algorithm

`__csv_parse` first splits the input into graphemes (`strings::graphemes`) and
then runs a single forward scan over the grapheme list with a small state set:
`inQuotes`, `fieldStarted`, `wasQuoted`, and `recordPending`. It accumulates the
current `field`, the current `row`, and the completed `rows`.
[[src/builtins/csv_package.mfb:__csv_parse]]

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
[[src/builtins/csv_package.mfb:__csv_parse]]

### CRLF peek caveat

The runtime grapheme splitter does **not** guarantee `\r\n` arrives as one
grapheme cluster — it may yield `\r` and `\n` as two separate graphemes. The
scanner therefore never assumes a single cursor step per record separator and
instead asks `__csv_separatorLength` for the step count:
[[src/builtins/csv_package.mfb:__csv_separatorLength]]

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

The CR grapheme itself is built at runtime by `__csv_crChar`, which encodes byte
13 via `toString([toByte(13)])`. The lexer only decodes the `\"`, `\\`, `\n`, and
`\t` string escapes, so a `"\r"` literal would lex to the letter `r`; CR must be
constructed from its byte instead. [[src/builtins/csv_package.mfb:__csv_crChar]]

## Errors

`__csv_parse` fails with `error(77050003, "invalid CSV format")` in two cases:
[[src/builtins/csv_package.mfb:__csv_parse]]

- **Text after a closing quote** within the same field (e.g. `"a"b`) — detected by
  the `wasQuoted` arm.
- **Unterminated quote** — `inQuotes` still set at end of input.

## Stringify algorithm

`__csv_stringify` is the inverse for the common case but is **not** a perfect
round-trip of separators: rows are joined with a single **LF**, with **no
trailing newline**, regardless of how the input was separated.
[[src/builtins/csv_package.mfb:__csv_stringify]] [[src/builtins/csv_package.mfb:__csv_stringifyRow]]

Fields are joined with `,`. A field is quoted **only when it must be** —
`__csv_needsQuote` returns true when the field contains a comma, a `"`, a CR, or
an LF. When quoted, the field is wrapped in `"` and every interior `"` is doubled
(`__csv_quoteField`). [[src/builtins/csv_package.mfb:__csv_needsQuote]]
[[src/builtins/csv_package.mfb:__csv_quoteField]]

```text
a,b          ' no special chars  → a,b
a "x"        ' contains a quote   → "a ""x"""
a,b (in one) ' contains a comma   → "a,b"
line1\nline2 ' contains LF        → "line1\nline2"  (quoted, LF kept verbatim)
```

A field containing only a bare CR is still quoted (CR triggers `needsQuote`), but
note that bare CR is *data* on the parse side, so stringify→parse preserves it.

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
