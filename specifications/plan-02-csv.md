# plan-02 — Built-in CSV Package

Last updated: 2026-06-25

This document is the **normative definition and implementation plan** for a new
built-in `csv` package. It is modeled directly on the existing `json` package:
the public surface, the "source package" architecture (a thin Rust shim plus an
MFBASIC implementation file injected at compile time), the man-page wiring, the
documentation, and the golden test layout all parallel `json` one-for-one.

Where the two packages differ, this document calls the difference out
explicitly (see §9 *Divergences from json*). The short version: JSON values are
a heterogeneous, self-describing tree, which is why `json` needs the `Json`
UNION. CSV is a flat grid of **String** cells with no per-cell typing and no
invariant to protect, so the document type is simply **`List OF List OF String`**
— no wrapper record, no union, no numeric inference. That part of the json model
deliberately does *not* carry over (see §2 for the rationale).

The `json` implementation referenced throughout lives at:

- `src/builtins/json.rs` — the Rust shim
- `src/builtins/json_package.mfb` — the MFBASIC implementation
- `src/man/builtins/json/*.txt` — the man pages
- `specifications/standard_package.md` §12 — the user-facing doc

See also: `specifications/standard_package.md` (§3.1 scalar-index model),
`specifications/error_codes.md` (canonical error codes — `csv` allocates none of
its own; it reuses the shared standard-package codes, exactly as `json` does).

---

## 1. The Functions

The `csv` package exports exactly **two** functions — the text⇄grid codec —
mirroring `json::parse` / `json::stringify`:

| Function | Signature | Behavior |
|----------|-----------|----------|
| `csv::parse` | `FUNC parse(value AS String) AS List OF List OF String` | Parses CSV text into a grid of rows of cells. Fails with `ErrInvalidFormat` for malformed input (see §5). |
| `csv::stringify` | `FUNC stringify(value AS List OF List OF String) AS String` | Renders a grid back to CSV text, quoting fields only where required (see §6). |

`IMPORT csv` requires no manifest dependency — `csv` is a built-in package, like
`json`.

---

## 2. The Document Type — and why there is no `Csv` wrapper

A CSV document is exactly **`List OF List OF String`**: an ordered list of rows,
each an ordered list of String cells. The package defines **no new types**.

This is a deliberate departure from `json`. An early draft of this plan wrapped
the grid in `EXPORT TYPE Csv { rows AS List OF CsvRow }` / `EXPORT TYPE CsvRow
{ fields AS List OF String }`, mirroring how `json` introduces its types. That
wrapper was dropped because:

1. **It protects no invariant.** An opaque type earns its keep when the raw
   representation could be put into an illegal state the wrapper prevents. *Any*
   `List OF List OF String` is already a valid CSV grid — there is no
   normalization, no hidden field, nothing to guard. (Contrast `json`: the
   `Json` UNION is essential because JSON values are heterogeneous and
   self-describing. CSV cells are not.)
2. **MFBASIC has no type-alias form** (`mfbasic.md` §4 / `:1757`: `TYPE` declares
   only records, `UNION` unions, `ENUM` enums). So `Csv` could only ever be a
   *wrapper record*, forcing every consumer to unwrap `.rows` / `.fields` before
   using `len`, `collections::get`, `FOR EACH`, `transform`, etc. — and forcing
   the package to ship `toRows` / `fromRows` conversions to bridge back to the
   plain list. Using `List OF List OF String` directly makes that conversion the
   **identity**: zero functions, full `collections` composability.
3. **The only real upside is speculative.** A wrapper would give a stable place
   to grow header/dialect metadata without changing signatures — but those are
   explicit v1 non-goals (§10). When that day comes, a *new* named type (e.g. a
   `CsvTable` carrying headers + the grid) can be introduced additively without
   retrofitting one onto the v1 grid.

There is no header concept in v1: every parsed line is an ordinary row.

---

## 3. Cell / Grid Model

CSV is a flat grid of String cells. Unlike JSON:

- There is **no type inference**. `42`, `true`, and `` (empty) are all just the
  Strings `"42"`, `"true"`, and `""`. Callers that want numbers convert
  explicitly with `toFloat` / `toInteger`.
- There is **no null**. An absent cell does not exist; an empty cell is the
  empty String.
- Rows are **not required to be rectangular**. `csv::parse` preserves whatever
  field count each row had; callers indexing with `collections::get` validate
  against the actual row they land on.

---

## 4. CSV Grammar (Normative)

The dialect is RFC-4180-aligned, with the relaxations and decisions fixed below.
Where this section and RFC 4180 disagree, **this section wins**.

```
document   = [ record *( record-sep record ) [ record-sep ] ]
record     = field *( "," field )
field      = escaped / non-escaped
escaped    = DQUOTE *( TEXTDATA / "," / CR / LF / 2DQUOTE ) DQUOTE
non-escaped= *( any byte except DQUOTE, comma, CR, LF )
record-sep = CRLF / LF
DQUOTE     = %x22  ("")
```

Fixed decisions:

1. **Field delimiter** is the comma `,` only. (No configurable delimiter in v1.)
2. **Record separator** on input is either `LF` (`\n`) or `CRLF` (`\r\n`); a
   bare `CR` not followed by `LF` is an ordinary data byte inside the current
   field. On output, `csv::stringify` always uses `LF`.
3. **A single trailing record separator does not create an empty final row.**
   `"a\nb\n"` parses to two rows, not three. Two consecutive separators
   (`"a\n\nb"`) *do* produce an empty row in the middle — a row with exactly one
   empty field.
4. **Whitespace is significant.** Spaces and tabs around a field are preserved;
   nothing is trimmed.
5. **Quoting.** A field may be wrapped in double quotes. Inside a quoted field, a
   literal double quote is written by doubling it (`""`), and commas, `CR`, and
   `LF` are ordinary data. The opening quote must be the first byte of the field
   and the closing quote must be immediately followed by a delimiter, a record
   separator, or end-of-input.
6. **Empty input** (`""`) parses to a document with **zero rows**.
7. Cells are decoded as UTF-8 Strings using the same grapheme/scalar handling as
   `json::parse` (`strings::graphemes`).

---

## 5. Parse Semantics & Errors

`csv::parse(value AS String) AS List OF List OF String` scans `value` left to
right and produces a grid. It fails with `ErrInvalidFormat` (`77050003`) in
exactly these cases:

- A quoted field is opened with `"` but never closed before end-of-input.
- A closing `"` of a quoted field is followed by a byte that is neither a comma,
  a record separator, nor end-of-input (e.g. `"ab"c` — stray data after the
  close quote).

Every other byte sequence is accepted; rows of differing widths are not an
error.

`ErrInvalidFormat` is the **only** error the package raises. Out-of-range cell
access is the caller's concern: `collections::get` on the parsed grid already
fails with `ErrIndexOutOfRange` (`77050001`) on its own.

`csv` allocates **no new error codes**; it reuses the shared standard-package
registry (`specifications/error_codes.md`), the same way `json` reuses
`ErrInvalidFormat`/`ErrNotFound`.

---

## 6. Stringify Semantics

`csv::stringify(value AS List OF List OF String) AS String` renders
deterministically:

- Rows are joined with a single `LF`; there is **no trailing newline**.
- Within a row, fields are joined with `,`.
- A field is emitted **quoted** if and only if it contains a comma, a double
  quote, a `CR`, or an `LF`; otherwise it is emitted bare. Inside a quoted field
  every `"` is doubled.
- The empty document (an empty outer list) stringifies to the empty String.

**Round-trip guarantee:** for any grid `x`, `csv::parse(csv::stringify(x))`
yields a grid whose cell values equal those of `x`, with one normalization: a
trailing empty row produced only by separator placement is not reintroduced (per
§4 decision 3). This mirrors the parse/stringify round-trip discussion in the
`json` man page.

---

## 7. Implementation Plan

The work parallels the `json` package file-for-file. Each phase below names the
exact files and the exact wiring sites (traced against the current tree).

### Phase 1 — Rust shim: `src/builtins/csv.rs`

Create `src/builtins/csv.rs` modeled on `src/builtins/json.rs`. It exposes the
same surface (`pub(crate)` items):

- Const names: `PARSE = "csv.parse"`, `STRINGIFY = "csv.stringify"`, and internal
  targets `__csv_parse`, `__csv_stringify`.
- **No `is_builtin_type`** — the package introduces no types, so this function
  (and the `builtins::is_builtin_type` wiring `json` needs) is omitted entirely.
- `is_csv_call(name)` → matches the two public names.
- `call_param_names(name)`:
  - `parse` → `&[&["value", "text"]]`
  - `stringify` → `&[&["value"]]`
- `call_return_type_name(name)`: `parse` → `"List OF List OF String"`;
  `stringify` → `"String"`.
- `resolve_call(name, arg_types)`:
  - `parse` if `["String"]` → `"List OF List OF String"`
  - `stringify` if `["List OF List OF String"]` → `"String"`
- `expected_arguments(name)`: `"String"` / `"List OF List OF String"`.
- `arity(name)`: `parse`/`stringify` → `(1, 1)`.
- `implementation_name(name)` → the `__csv_*` targets.
- `source_file()` → `parse_source("<builtin-csv>", "builtins/csv.mfb",
  include_str!("csv_package.mfb"))`.
- `uses_package(ast)` → any import whose `package_name() == "csv"`.
- `augmented_project(ast)` → clone, and if `uses_package`, push `source_file()`.

### Phase 2 — MFBASIC implementation: `src/builtins/csv_package.mfb`

Create `src/builtins/csv_package.mfb` in the same idiom as
`json_package.mfb` (`IMPORT collections`, `IMPORT strings`; recursion +
`FOR EACH`; grapheme-by-grapheme scanning; private helpers prefixed `__csv_`).

No types are declared. Implement the two functions over the plain grid:

- `__csv_parse(value AS String) AS List OF List OF String` — scan graphemes,
  building rows and fields; handle bare vs. quoted fields, doubled quotes,
  `LF`/`CRLF` separators, and the trailing-separator rule (§4); `FAIL
  error(77050003, ...)` on the two malformed cases (§5).
- `__csv_stringify(value AS List OF List OF String) AS String` — join per §6,
  quoting only when a field contains `,`, `"`, `CR`, or `LF`.

**MFBASIC source-package constraints** (carry over the gotchas already learned
for the regex/json source packages): reserved words cannot be identifiers;
functions take ≤ 8 parameters; no direct field assignment (build records with
constructor syntax / thread state through helper return values); cross-file
visibility requires `EXPORT`; backslashes in string literals are escaped (`\\`).
Use a small node record (à la `__JsonStringNode { value, index }`) to thread
the scan cursor through recursive helpers.

### Phase 3 — Wire the shim into the compiler

Add `csv` alongside `json` at every site that currently chains `json`
(grepped against the tree — keep alphabetical/adjacent ordering with `json`):

- `src/builtins/mod.rs`:
  - `pub(crate) mod csv;`
  - add `"csv"` to `is_builtin_import`
  - add `.or_else(|| csv::call_return_type_name(name))` to `call_return_type_name`
  - add `csv::is_csv_call(name)` to `is_builtin_call`
  - add `.or_else(|| csv::call_param_names(name))` to `call_param_names`
  - **no `is_builtin_type` line** — unlike `json`, `csv` registers no types.
- `src/resolver.rs:42` — chain `csv::augmented_project` next to the `json`/`regex`
  augmentation.
- `src/typecheck.rs:117` — chain `csv::augmented_project`; add `csv::arity`
  (near `:4852`), `csv::resolve_call` + `csv::expected_arguments` (near `:4872`).
- `src/ir.rs:411` — chain `csv::augmented_project`; add `csv::resolve_call`
  (near `:2229`), `csv::expected_arguments` (near `:2391`),
  `csv::implementation_name` (near `:2725`).

> Note ordering of `augmented_project` calls: `json` then `regex` are applied in
> sequence (each takes the previous result). Insert `csv` into that chain
> consistently in `resolver.rs`, `typecheck.rs`, and `ir.rs` so all three agree.

### Phase 4 — Man pages

Create `src/man/builtins/csv/` with `package.txt`, `parse.txt`, and
`stringify.txt`, modeled on `src/man/builtins/json/*.txt` (NAME / SYNOPSIS /
DESCRIPTION / ERRORS, plus EXAMPLES for the function pages). Then wire
generation:

- `build.rs`: add `csv_dir`, `let csv_pages = man_pages(&csv_dir, "csv");`, a
  `rerun-if-changed` for `csv_dir` and its `package.txt`, `.chain(csv_pages.iter())`
  in both the rerun loop and the page chain, and
  `write_pages(&mut output, "CSV_FUNCTION_PAGES", csv_pages);`.
- `src/man/mod.rs`: add a `parse_package(include_str!("builtins/csv/package.txt"),
  "mfb man csv [function]")` entry to `PACKAGES`, and a
  `"csv" => Some(generated::CSV_FUNCTION_PAGES)` arm in `generated_pages`.

### Phase 5 — User documentation

- `specifications/standard_package.md`: add a new "Built-in CSV Package" section
  (place it after §12 *Built-in JSON Package*), stating that the document type is
  `List OF List OF String` (no package types) and that indexing uses
  `collections`, with the two-function table from §1 here, plus the format
  decisions from §4–§6 summarized.
- `specifications/error_codes.md`: no new codes; if the doc tracks "used by"
  notes, add `csv` to the `ErrInvalidFormat` row.
- Regenerate any aggregated docs covered by `plan-09-doc` if applicable.

### Phase 6 — Tests (golden)

Mirror the `tests/func_json_*` and `tests/json_*` fixtures. Each test dir holds
`project.json` (kind `executable`, entry `main`, `targets: ["native"]`),
`src/main.mfb`, and a `golden/` with `.ast`, `.ir`, `.run`, and `build.log`.
Add at least:

- `func_csv_parse_valid` — parse a mix of bare/quoted/empty/multiline-quoted
  fields, CRLF and LF separators, trailing newline; print round-tripped output.
- `func_csv_parse_invalid` / `func_csv_parse_invalid_runtime` — unterminated
  quote and stray-data-after-close-quote (`ErrInvalidFormat`).
- `func_csv_stringify_valid` — fields needing and not needing quoting; embedded
  quotes/commas/newlines; empty document.
- `csv_read_valid` / `csv_write_valid` — a parse→stringify→parse round-trip
  asserting the §6 guarantee, indexing parsed cells with `collections::get`.

Generate goldens with `scripts/sync-goldens.sh` and verify with
`scripts/test-accept.sh` on the host target.

---

## 8. Worked Examples

```basic
IMPORT csv
IMPORT io

FUNC main AS Integer
  LET doc AS List OF List OF String = csv::parse("name,age\r\nAda,36\nGrace,\"Hop,per\"")
  io::print(toString(len(doc)))                          ' 3

  ' Indexing is the collections package — no csv-specific accessors:
  LET header AS List OF String = collections::get(doc, 0)
  io::print(collections::get(header, 0))                 ' name
  LET third AS List OF String = collections::get(doc, 2)
  io::print(collections::get(third, 1))                  ' Hop,per  (comma was quoted)

  io::print(csv::stringify(doc))                          ' name,age\nAda,36\nGrace,"Hop,per"

  FOR EACH row IN doc
    io::print("cols=" & toString(len(row)))
  NEXT
  RETURN 0
END FUNC
```

Quoting on output (§6): the cell `Hop,per` contains a comma, so it is re-quoted;
`name`, `age`, `Ada`, `36`, `Grace` are emitted bare. The CRLF in the input is
normalized to LF on output. Because `csv::parse` returns the grid directly,
there is no document-to-`List OF List OF String` conversion step — they are the
same type, and `collections::get` already raises `ErrIndexOutOfRange` for a bad
index.

---

## 9. Divergences from `json`

| Aspect | `json` | `csv` |
|--------|--------|-------|
| Functions | `parse`, `stringify`, `get`, `getOr` (4) | `parse`, `stringify` (2) — codec only |
| Value model | Heterogeneous tree via UNION `Json` + 6 member types | Flat grid as plain `List OF List OF String`, **no package types** |
| Cell types | `JsonNull`/`Bool`/`Num`(Float)/`Str`/`Arr`/`Obj` | **String only** — no inference, no null |
| Cell access | package readers `get`/`getOr` (the tree is opaque) | `collections::get` on the grid (no package readers) |
| Errors raised | `ErrInvalidFormat`, `ErrNotFound` | `ErrInvalidFormat` only |
| `is_builtin_type` wiring | registers 7 type names | none |

Everything else — the source-package architecture, the shim API shape, the
compiler wiring sites, the man-page pipeline, and the golden-test layout — is
identical to `json`.

---

## 10. Non-Goals for v1

- Configurable delimiters / quote characters (always `,` and `"`).
- Header-aware parsing or column-name lookup (the grid is positional in v1).
- Type inference or per-cell typing (every cell is a `String`).
- Streaming / incremental parse of arbitrarily large inputs (parse consumes the
  whole String, matching `json::parse`).
- A configurable output dialect (output is always LF-separated, minimally
  quoted, no trailing newline).

A future plan may add a header helper (e.g. `csv::parseWithHeader` returning a
`List OF Map OF String TO String`, or a new `CsvTable` type bundling headers with
the grid) and/or a dialect-options record. Those would be introduced
*additively* — a new named type alongside the v1 grid — rather than retrofitting
an opaque wrapper onto `csv::parse`'s return value. Both are out of scope here.
