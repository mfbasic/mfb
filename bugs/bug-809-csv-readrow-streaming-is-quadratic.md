# bug-809: `csv::readRow` streaming is quadratic in the row count

Last updated: 2026-10-04

Effort: medium (1h–3h)

Severity: MEDIUM

Class: Performance

Status: Blocked — needs an owner decision (API/representation change or a compiler move feature); see Phase 1 findings
Regression Test: proposed `tests/runtime/rt_csv_read_row_scaling.rs` (CPU-time
scaling assertion, following the `rt_self_update_cost.rs` pattern noted in
`.ai/collections.md`)

`csv::readRow`'s whole purpose, per its own man page, is to let "a document ...
processed one row at a time" without "the whole `List OF List OF String` grid"
ever being materialized — the selling point over `csv::parse` for a large
input. Measured end to end, parsing N rows with a `parseStream`/`readRow` loop
costs time proportional to N², not N: doubling the row count roughly
quadruples the wall time.

References:

- `src/docs/spec/stdlib/*csv*` / `mfb man csv readRow`, `mfb man csv
  parseStream` — the streaming/one-row-at-a-time contract this defeats.
- `.ai/collections.md` ("List memory management", "copy-insertion") — the
  general rule that a flat `List` is copied on a store unless the compiler can
  prove the source is read for the last time; the likely mechanism behind this
  bug, below.
- Found during bug-807 follow-up work (bug-staging-7) reproducing findings
  reported against `http`/`canvas`/`audio`/`csv`/`mfb audit`.

## Failing Reproduction

```sh
mkdir -p /tmp/mfb-f4/src
cat > /tmp/mfb-f4/project.json <<'JSON'
{"name": "f4", "version": "0.1.0", "mfb": "1.0", "kind": "executable", "sources": [{"root": "src", "role": "main", "include": ["**/*.mfb"]}], "entry": "main", "targets": ["native"]}
JSON
cat > /tmp/mfb-f4/src/main.mfb <<'BASIC'
IMPORT io
IMPORT csv
IMPORT datetime

FUNC makeCsv(rows AS Integer) AS String
  MUT out AS String = "a,b,c\n"
  MUT i AS Integer = 0
  WHILE i < rows
    out = out & toString(i) & ",value" & toString(i) & ",x\n"
    i = i + 1
  END WHILE
  RETURN out
END FUNC

FUNC timeRows(rows AS Integer) AS Nothing
  LET text = makeCsv(rows)
  LET t0 = datetime::monotonicNanos()
  MUT reader = csv::parseStream(text)
  LET header = csv::readRow(reader)
  reader = header.reader
  MUT n AS Integer = 0
  MUT done AS Boolean = FALSE
  WHILE done = FALSE
    LET row = csv::readRow(reader)
    reader = row.reader
    done = row.done
    IF done = FALSE THEN
      n = n + 1
    END IF
  END WHILE
  LET t1 = datetime::monotonicNanos()
  io::print(toString(rows) & " rows: " & toString(n) & " parsed, " & toString((t1 - t0) / 1000000) & " ms")
END FUNC

FUNC main() AS Nothing
  timeRows(5000)
  timeRows(10000)
  timeRows(20000)
END FUNC
BASIC
target/release/mfb build /tmp/mfb-f4
/tmp/mfb-f4/build/f4.out
```

- Observed (macos-aarch64, `bug-staging-7` release build at
  `/tmp/bs7-target/release/mfb`, no `--debug`):

  ```
  5000 rows: 5000 parsed, 2249 ms
  10000 rows: 10000 parsed, 10724 ms
  20000 rows: 20000 parsed, 51160 ms
  ```

  Growth factor for a 2× row-count step: 10724/2249 ≈ 4.77×, and
  51160/10724 ≈ 4.77× — consistently close to 4×, the signature of an O(n²)
  algorithm (an O(n) algorithm would show ≈2×).

- Expected: wall time roughly linear in row count (≈2× per doubling), matching
  the "processed one row at a time" streaming contract.

## Root Cause

Not fully proven by an IR/assembly trace, but strongly indicated by measurement
plus the documented copy semantics:

`csv::CsvReader` carries the entire decoded input as `chars AS List OF
Integer` (`src/codegen/builtins/csv/func_read_row.rs` body, `__csv_next`).
Every `csv::readRow` call takes a `reader` parameter, reads
`LET chars AS List OF Integer = reader.chars`, and returns a brand-new
`CsvReader[chars, count, index, delimCode, quoteCode]` embedding that same
`chars` value. Per `.ai/collections.md` ("In-place mutation... copy-insertion"),
a flat `List` store is copied unless the compiler can prove the source binding
is read for the last time (plan-134's move optimization); `reader` here is read
for several other fields in the same function body (`count`, `delimCode`,
`quoteCode`, `index`) before `chars` is threaded into the constructed return
value, which is the shape that normally defeats the last-read/move proof. If
`chars` is copied (its full O(n) backing buffer) on each of the O(n) `readRow`
calls needed to walk an n-row document, the total cost is O(n²) — exactly the
measured growth factor.

Likely site: `src/codegen/engine/value/builder_values.rs:783`
(`lower_value_owned`), the general owned-value lowering that `copy_flat_block`
routes through for an aliasing source, as described in
`.ai/collections.md`'s "copy-insertion" note. Confirming this requires an
`--ir`/`--ncode` dump of `__csv_next` and is left to Phase 1.

## Goal

`csv::readRow` costs O(1) amortized per call (not counting the cost of the
single record it parses), so a parseStream/readRow loop over an n-row document
costs O(n) total, matching its documented streaming contract.

### Non-goals (must NOT change)

- No change to the records `csv::readRow` yields (byte-identical to
  `csv::parse` for the same input), or to `csv::parse` itself.
- No change to `CsvReader`'s observable fields (`chars`, `count`, `index`,
  `delimCode`, `quoteCode`) — only how `chars` is threaded through
  `readRow`'s return may change (e.g. proving the move, or representing the
  reader so `chars` is shared rather than copied).

## Blast Radius

- `csv::readRow` (`src/codegen/builtins/csv/func_read_row.rs`) — the fix site.
- Any program looping `parseStream`/`readRow` over a large document — this is
  the package's advertised use case for large input, so the fix is a pure
  performance win with no documented behavior change expected.
- If the fix is a codegen-level last-read/move proof improvement (rather than a
  `csv`-local change), it may also speed up other record-field-threading loops
  with the same shape; verify no `.ncode`/`.ir` golden outside `csv/*` moves
  unless that broader fix is explicitly chosen.

## Phases

### Phase 1 — confirm the mechanism

Dump `--ir`/`--ncode` for a minimal `__csv_next` call and confirm whether
`chars` is copied or moved into the returned `CsvReader`. Add
`rt_csv_read_row_scaling.rs`: assert equal CPU time (not wall time) for R runs
of n rows vs. 1 run of R·n rows, per the `rt_self_update_cost.rs` pattern
referenced in `.ai/collections.md`. Confirm red.

### Phase 2 — the fix

Either prove the move (last-read optimization sees through the other field
reads before `chars`'s use) or restructure `CsvReader`/`__csv_next` so `chars`
is never deep-copied per call.

### Phase 3 — validation

The new scaling test green; re-run the reproduction above and confirm the
growth factor is ≈2× per doubling, not ≈4×; `scripts/test-accept.sh` for any
`csv` fixture.

## Phase 1 findings (2026-10-04, bug-staging-8)

**Measured baseline** (macos-aarch64, release build of `e841dc05f`, the
reproduction above with `makeCsv` built by `strings::join` so setup stays linear,
`ROWS` from the environment; machine load average 11–17, another gate running):

| rows | readRow loop |
| --- | --- |
| 5,000 | 4,430 ms |
| 10,000 | 17,118 ms (×3.86) |
| 20,000 | 65,098 ms (×3.80) |
| 50,000 | 222,038 ms |

**The hypothesis is right about the copy and wrong about where it can be avoided.**
`--debug` arena counters (`arena.0.alloc_bytes`) put the cost at ~2.4 copies of the
whole `chars` list per `readRow` call in the man-page loop (`row =
csv::readRow(row.reader)`: 283 MB at 1,000 rows, 1,266 MB at 2,000, against a
118/254 KB `chars`), and ~3.7 in the reproduction's loop (`reader = row.reader`
adds one). Removing `LET chars AS List OF Integer = reader.chars` would remove
one of them, but **not the one that makes it quadratic**:

- A flat collection field is stored **inline** in its record's block
  (`record_field_is_inlined`: a memcpy-copyable `List` lives in the record's
  trailing data region, plan-02 §4.2–§4.4). `CsvRow` therefore carries the whole
  `CsvReader`, `chars` included, inside its own block, and every `CsvRow[...]` that
  `readRow` returns is a fresh block holding a byte copy of `chars`.
- No store of a flat value moves: `lower_value_stored` lets the constructor writer
  byte-copy a flat payload, and the plan-134-C last-use move only applies to graph
  (recursive) types. Measured with a user-level replica (`TYPE R { chars AS List OF
  Integer, … }`, 100k elements, `hit_large_bin` per iteration): `Row[[], r, FALSE]`
  from a parameter — 1 copy; `LET row = Row[[], r, FALSE]` then `r = row.reader`
  with `r` an owned local at its last use — 2 copies; `R[chars, …]` / `WITH r { … }`
  inside a constructor — 2 copies.
- The only zero-copy shape that exists today is plan-147's hand-over with an
  in-place `RETURN WITH r { index := … }` on an owned local, `r = next(r)` —
  measured 0 large allocations per iteration. `readRow` cannot use it: its result is
  a `CsvRow` that EMBEDS the reader, and its argument at the documented call site is
  a field read (`row.reader`), which hand-over refuses (H1).

So any `readRow` that returns a `CsvRow` containing the advanced `CsvReader` costs
O(len(chars)) per call under the current layout, whatever its body does. A
`csv`-local body change can only shave the constant (≈2.4 → ≈1.4 copies).

**Options (owner decision needed):**

1. *API/representation:* thread the reader as its own value — e.g. a
   `csv::readRow(reader) AS CsvRow` whose `CsvRow` carries only `fields`/`done`
   plus the new `index`, or a reader-returning `csv::advance` — so the loop is
   `reader = f(reader)`, which plan-147 already makes zero-copy. Changes the
   documented `CsvRow`/loop shape (a Non-goal above).
2. *Compiler:* a destination/move feature for `x = f(x.field)` where the result
   embeds the argument — the caller hands over the field's span, the callee
   updates it in place (the bug-784 `$into` destination-variant pattern, widened).
   Broad codegen change; moves `.ncode` far outside `csv`.
3. *Layout:* store large flat collection fields as pointers instead of inline.
   Broadest; changes every record layout.

No code was changed for this bug; no regression test was added (a scaling test
committed red would break the suite). The reproduction above, with `ROWS` taken
from the environment, is the RED check for whichever option is chosen.
