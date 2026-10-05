# bug-829: csv::stringify → csv::parse drops a trailing single-empty-cell row and turns a zero-cell row into a one-cell row

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Correctness (owner-confirmed: code fix)

Status: Open
Regression Test: tests/rt-behavior/csv/csv-roundtrip-empty-rows-rt (proposed)

The spec promises "Cell **values** round-trip (quoting is added/removed
transparently)" with only separator normalisation as the exception; the man page
says "For any grid x, csv::parse(csv::stringify(x)) yields a grid whose cells equal
those of x, with one normalization: a trailing empty row produced only by separator
placement is not reintroduced". Three grids violate it:

- `[["a"], [""]]` stringifies to `a\n`, which parses to `[["a"]]` — a real row with
  one (empty) cell is lost.
- `[[""]]` stringifies to `""`, which parses to `[]` — the grid vanishes.
- `[["a"], [], ["b"]]` stringifies to `a\n\nb`, which parses to `[["a"], [""], ["b"]]`
  — the zero-cell row comes back with one cell.

The first two are fixable in `stringify` the way Python's `csv` module does it:
a row consisting of exactly one empty field is written as `""` (quoted), so it
cannot be mistaken for a bare separator. The third (a zero-cell row) has no CSV
spelling today.

**Owner decision (2026-10-05): quote. Exact round trip is required.** The fix is in
code, not in the docs: `stringify` writes a row consisting of exactly one empty field
as `""`, and `parse(stringify(x))` must equal `x` for **every** grid, including grids
with zero-cell rows. Once a lone empty field is always quoted, a bare empty line is
no longer ambiguous and is the spelling of a zero-cell row, so `parse` must read an
empty line as `[]` (not `[""]`). The fix must also give `[]` (empty grid) and `[[]]`
(one zero-cell row) distinct spellings. The spec and man round-trip claims lose the
"one normalization" exception once this lands.

**Owner refinement (2026-10-05): trim before parse, after stringify.** Blank leading
and trailing lines are not data and can be lost. So `stringify` trims leading and
trailing blank lines from its output, and `parse` trims them from its input before
parsing. Only **interior** empty lines carry meaning (a zero-cell row). A leading or
trailing zero-cell row is not preserved, and `[]` and `[[]]` need not be distinct.
The trim removes only blank lines (line separators); it never strips spaces or other
characters inside the first or last row, and it never removes a quoted `""` line,
which is a row holding one empty cell.

References:

- src/docs/spec/stdlib/03_csv.md §"Round-trip notes": "Cell **values** round-trip";
  "Empty input parses to an empty grid; an empty grid stringifies to `""`."
- `mfb man csv stringify` (round-trip paragraph quoted above).

## Failing Reproduction

```
IMPORT io
IMPORT csv
IMPORT collections

FUNC main() AS Nothing
  LET a AS List OF List OF String = [["a"], [""]]
  LET b AS List OF List OF String = [[""]]
  LET c AS List OF List OF String = [["a"], [], ["b"]]
  io::print("a rows in=2 out=" & toString(len(csv::parse(csv::stringify(a)))))
  io::print("b rows in=1 out=" & toString(len(csv::parse(csv::stringify(b)))))
  io::print("c row1 cells in=0 out=" & toString(len(collections::get(csv::parse(csv::stringify(c)), 1))))
END FUNC
```
`/tmp/hardening/mfb build /tmp/hardening/B02/min3 && /tmp/hardening/B02/min3/build/min3.out`

- Observed:
```
a rows in=2 out=1
b rows in=1 out=0
c row1 cells in=0 out=1
```
- Expected (if fixed in code): `out=2`, `out=1`; `c` documented as a normalisation.

Contrast: `[[""], ["x"]]` round-trips (`\nx` → `[[""],["x"]]`), as does any row
with ≥2 cells or a non-empty cell.

## Root Cause

`src/codegen/builtins/csv/helper_needs_quote.rs:__csv_needsQuote` quotes only for
delimiter/quote/CR/LF, so a lone empty field is written as nothing, and the row
text is indistinguishable from the "no trailing empty row" separator rule in
`src/codegen/builtins/csv/helper_scan.rs:__csv_scan` (trailing record appended only
if `fieldStarted`/`recordPending`/`wasQuoted`). A zero-cell row
(`helper_stringify_row.rs:__csv_stringifyRow` of `[]`) and a one-empty-cell row both
produce the empty line.

## Goal

- `parse(stringify(x))` equals `x` for every grid `x` whose first and last rows are
  not zero-cell rows, including `[[""]]`, `[["a"], [""]]`, `[[""], ["a"]]`,
  `[["a"], [], ["b"]]`, and `[]`.
- Leading and trailing zero-cell rows are dropped: `parse(stringify([[], ["a"], []]))`
  is `[["a"]]`, and `[[]]` round-trips to `[]`.
- `parse` trims blank leading/trailing lines from its input before parsing, so
  `parse("\n\na,b\n\n")` is `[["a", "b"]]`.
- The spec (`mfb spec stdlib csv`) and `mfb man csv stringify` state the exact
  round trip with no exceptions.

### Non-goals (must NOT change)

- Parse-side dialect other than the empty-line rule above (bare CR data, the
  trailing-separator rule).
- Forbidden wrong fix: narrowing the documented round-trip claim instead of fixing
  the code — the owner rejected that option.

## Blast Radius

- `__csv_needsQuote` / `__csv_stringifyRow` — the only writer.
