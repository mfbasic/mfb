### 1. "Empty row" means two different things across the pages, and the round-trip promise breaks on it
UNIT:      man-pkg:csv
PAGE:      package-wide (overview, parse, stringify)
CATEGORY:  consistency
CLAIM:     Overview and parse: "two consecutive separators do produce an empty row in the middle". Stringify: "An empty row stringifies to an empty line, so a two-element outer list containing two empty rows produces a lone LF." Overview and stringify: "For any grid x, csv::parse(csv::stringify(x)) yields a grid whose cells equal those of x, except that a trailing empty row produced only by separator placement is not reintroduced".
VERDICT:   inconsistent
EVIDENCE:  Probe /tmp/plan-125-scratch/B-recheck/man-pkg-csv/p1 (built with `mfb build -q .`, run `./build/mfb_project.out`): `parse("a\n\nb")` → `[1:<a>] [1:<>] [1:<b>]` (the "empty row" is one empty cell, never a zero-cell row); `parse("\n")` → one row `[1:<>]`; `stringify([[],[]])` → `"\n"`, so `parse(stringify([[],[]]))` → one row `[1:<>]`; `parse(stringify([[],["a"]]))` → `[1:<>] [1:<a>]` (a cell appears that x did not have); `parse(stringify([["a"],[""]]))` → `[1:<a>]` (a trailing one-empty-cell row, which is not "separator placement", is dropped). Scanner: `helper_scan.rs:__csv_scan` always appends `__csv_fieldValue` before pushing a row, so a parsed row has at least one cell.
SUGGESTED: Pick one term and state the shape. Parse/overview: "an empty line parses to a row holding one empty String; csv::parse never returns a row with zero cells." Stringify: "a row with no cells and a row with one empty cell both stringify to an empty line." Round-trip (both pages): "csv::parse(csv::stringify(x)) equals x when every row of x has at least one cell and the last row is not a single empty String; a zero-cell row reads back as one empty cell, and a final row that is empty or holds one empty String is dropped."

### 2. The CRLF clause in the round-trip promise is stated in the wrong direction
UNIT:      man-pkg:csv
PAGE:      overview and stringify
CATEGORY:  overview-mismatch
CLAIM:     "For any grid x, csv::parse(csv::stringify(x)) yields a grid whose cells equal those of x, except that ... a CRLF separator is normalized to LF." (stringify page: "a CRLF separator in the original text is normalized to LF on output.")
VERDICT:   misleading
EVIDENCE:  A grid x has no separators, so nothing in parse(stringify(x)) can be a CRLF separator. The normalization exists only in the text→grid→text direction: probe p1 `stringify(parse("a,b\r\nc,d\r\n"))` → `a,b\nc,d`. A cell that itself contains CRLF is quoted and survives intact (`helper_needs_quote.rs:__csv_needsQuote` quotes on `\r` or `\n`).
SUGGESTED: Move the clause to the text side: "Going the other way, csv::stringify(csv::parse(text)) reproduces text except that CRLF record separators come back as LF and a single trailing separator is dropped." Drop it from the grid round-trip sentence.

### 3. Stringify says the dialect must be a single character; parse says stringify writes the whole text
UNIT:      man-pkg:csv
PAGE:      stringify vs parse
CATEGORY:  consistency
CLAIM:     Stringify: "delimiter and quote must each be a non-empty single character; an empty delimiter or quote raises ErrInvalidFormat." Parse: "parsing uses only its first Unicode scalar; csv::stringify writes the whole text, so pass exactly one scalar when the same dialect must read back what it wrote."
VERDICT:   inconsistent
EVIDENCE:  Probe p1: `stringify([["a","b"],["c|d","e"]], "||")` → `a||b` / `c|d||e` (multi-character delimiter accepted and written whole, no error); `parse("a||b", "||")` → 3 cells `<a><><b>`. `func_stringify.rs:__csv_stringify` only calls `__csv_firstCode` (non-empty check) and passes the full `delimiter` string to `__csv_stringifyRow`/`__csv_needsQuote`.
SUGGESTED: Stringify page: "delimiter and quote must be non-empty (an empty one raises ErrInvalidFormat). Only the first Unicode scalar is meaningful to csv::parse and csv::readRow, which stringify does not enforce: it writes the whole text, so pass exactly one scalar if the output must be read back." Same wording on the overview's dialect paragraph.

### 4. The round-trip promise is silently untrue for a custom newline, and the overview's quoting rule ignores the dialect
UNIT:      man-pkg:csv
PAGE:      overview and stringify
CATEGORY:  overview-mismatch
CLAIM:     Overview: "a field is quoted only when it contains a comma, a double quote, a CR, or an LF" and "For any grid x, csv::parse(csv::stringify(x)) yields a grid whose cells equal those of x".
VERDICT:   misleading
EVIDENCE:  Probe p1: `parse(stringify([["a"],["b"]], ",", "\"", "|"))` → one row `[1:<a|b>]`; with newline `"\r"` → `[1:<ab>]`; a field containing the newline text is not quoted: `stringify([["a|x"],["b"]], ",", "\"", "|")` → `a|x|b`; `stringify([["a"],["b"]], ",", "\"", "")` → `ab` with no error. Parse page: only LF/CRLF are record separators "regardless of dialect". Quoting is on delimiter/quote per `helper_needs_quote.rs:__csv_needsQuote`, not on comma/double quote.
SUGGESTED: Overview quoting sentence: "a field is quoted only when it contains the delimiter, the quote character, a CR, or an LF". Round-trip sentence (both pages): qualify with "with the default newline (or CRLF); csv::parse recognises only LF and CRLF as record separators, so a grid written with any other newline does not read back, and a cell containing that newline text is not quoted." Stringify page: state that an empty newline is accepted and simply concatenates rows.

### 5. Nothing in the unit says what happens to a UTF-8 byte-order mark
UNIT:      man-pkg:csv
PAGE:      package-wide
CATEGORY:  coverage
CLAIM:     No page mentions a BOM. A developer opening a CSV exported by Excel or Windows tools will arrive with this question.
VERDICT:   missing
EVIDENCE:  Probe p1: `parse("\u{FEFF}a,b\n1,2")` → first cell is `<\uFEFFa>` (3 bytes of BOM kept inside the cell); `grep -i bom /tmp/plan-125-scratch/B-recheck/man-pkg-csv/all.txt` → no output.
SUGGESTED: Add to the overview's "Cells are plain Strings" paragraph and to parse: "A leading byte-order mark is not stripped; it stays at the front of the first cell. Strip it yourself (strings::trimPrefix or equivalent) before comparing header names."

### 6. A quote that is not the first character of a field is never explained
UNIT:      man-pkg:csv
PAGE:      parse and readRow
CATEGORY:  coverage
CLAIM:     Parse: "the opening quote must be the first character of the field". The page lists what raises ErrInvalidFormat (unclosed quote, text after a closing quote) but never says what a quote inside an unquoted field does.
VERDICT:   missing
EVIDENCE:  Probe p1: `parse("a\"b,c")` → `[2:<a"b><c>]` (ordinary data, no error); `stringify([["a\"b"]])` → `"a""b"`. Scanner `helper_scan.rs:__csv_scan`: a quote only opens quoting when `fieldStarted = FALSE`, otherwise falls to the ordinary-data branch.
SUGGESTED: Parse and readRow: "A quote character that appears after the field has started (for example a"b or a space before the quote) is ordinary data, not an error. csv::stringify always quotes such a cell, so it round-trips."

### 7. Stringify's own page contradicts its declaration on the argument count
UNIT:      man-pkg:csv
PAGE:      stringify
CATEGORY:  consistency
CLAIM:     "The sole argument is named value, so it can be supplied positionally or as the keyword argument value :=."
VERDICT:   wrong
EVIDENCE:  The Declaration on the same page lists four parameters (`value`, `delimiter`, `quote`, `newline`); `func_stringify.rs:register` has four `Parameter` entries. The parse page uses the accurate phrasing "A String argument may also be supplied by the name text."
SUGGESTED: "The grid is the parameter value and may be passed positionally or as value :=; delimiter, quote and newline may likewise be passed by name."

What I checked beyond these: the readRow loop and header-then-parse examples run as printed (probe p2: three records with `eof=TRUE` afterwards, with and without a trailing newline; header+parse gives 2 columns, 2 rows; openData with `;` gives 2 records); readRow and parse agree on empty lines; an empty delimiter raises ErrInvalidFormat 7-705-0003 as stated; the `types` page correctly reports no public types; the function table and reading order (parse, readRow, stringify) are discoverable from the overview.
