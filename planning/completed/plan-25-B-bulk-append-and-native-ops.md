# plan-25-B: Bulk in-place append + native list-op bulk memcpy

Last updated: 2026-07-05
Effort: large (3h–1d)

Two genuine algorithmic/codegen slownesses that are independent of the arena fix
(plan-25-A):

1. **Bulk append is O(N²).** `result = collections::append(list, otherList)`
   (list-into-list) falls off the in-place fast path and does a full
   value-semantic rebuild every iteration. This is why `flatten` costs **3644 ms**
   (200 flattens each doing 100 bulk-appends of a growing result) and
   `append_batch` costs 0.83 ms vs Python's 0.005.
2. **Native list ops build results element-by-element with no bulk memcpy.**
   `mid`, `insert`, `removeAt`, `transform`, `filter`, `replace` write per-entry
   lookup-table metadata with 6+ individual stores and copy payloads a *byte at a
   time*, where a single `memcpy`/word-loop would do. This raises each op's
   intrinsic cost (and feeds the arena churn that A cleans up).

It complements:

- `./mfb spec package collections` (the value-semantic contract these ops keep).
- `./mfb spec memory collections` (`src/docs/spec/memory` layout of the
  entry lookup table + data region these ops must preserve byte-for-byte).
- `planning/plan-25-A-arena-large-block.md` (A restores post-churn speed; B
  lowers the intrinsic per-op cost — they stack).

## 1. Goal

- `collections::append(list, sublist)` used as `list = collections::append(list,
  sublist)` on a uniquely-owned MUT list appends the sublist's elements in place
  (amortized O(1) per element), not a full rebuild.
- `mid` / `insert` / `removeAt` / `transform` / `filter` / `replace` copy the
  unchanged entry-table spans and payload bytes with bulk `emit_block_copy_advance`
  (8-byte word loop) rather than per-entry field reconstruction and per-byte
  loops.
- Byte-identical output collections (same entry table, same data region) — only
  the *instructions that build them* change.

### Non-goals (explicit constraints)

- No change to value/copy semantics: these remain pure functions returning new,
  unaliased collections (the in-place append case is the already-blessed
  uniquely-owned-MUT optimization, extended to a list RHS).
- No change to collection layout / ABI / golden data bytes.
- Does not fix the arena churn (that is plan-25-A) — B lowers base cost only.

## 2. Current State

**Bulk-append gate.** `try_inplace_append_assign`
(`src/target/shared/code/builder_inplace_assign.rs:19-75`) commits to the in-place
path only when the appended item's static type equals the list's *element* type
(`:54-57`). A bulk `append(list, otherList)` has item type `List OF T` ≠ element
type `T`, so it returns `false` and control falls to `lower_value_owned`
(`builder_control.rs:257`) → `lower_collection_append` → full concatenating
rebuild that copies the entire accumulated list each call. In a loop that is
O(N²). Confirmed: `flatten` (`collections_package.mfb:198-207`) and
`append_batch` both hit this.

**Native per-element cost** (all in `src/target/shared/code/`):

- `mid` — `builder_search.rs:786` `lower_list_mid`: per-element entry metadata
  (`:1015-1055`) + a **byte-at-a-time** payload loop (`:1059-1067`).
- `insert` — `builder_collection_mutate.rs:434` `lower_list_insert_collection`:
  data region uses `emit_block_copy_advance` (good) but the entry splice
  (`:645-704`) reconstructs each entry with 6× stores.
- `removeAt` — `builder_collection_mutate.rs:2578` `lower_list_remove_at` →
  `emit_copy_collection_entries` (`:2896-2989`) rebuilds each prefix/suffix entry
  field-by-field.
- `transform` — `builder_collection_queries.rs:895`
  `lower_collection_transform_call`: per-element `lower_list_append_in_place`
  (`:969`), each doing full entry setup + payload copy.
- `filter` — `builder_collection_queries.rs:981`: same per-kept-element append.
- `replace` — `builder_strings.rs:304` `lower_list_replace`: per-entry stores +
  byte-loop String payload copy (`:598-606`).

## 3. Design Overview

- **B1 — bulk in-place append.** Add a sibling to `try_inplace_append_assign`
  that fires when `arg0 == name`, the local is a uniquely-owned MUT list, and
  `static_type_name(args[1])` equals the *list* type (not element). Lower it as:
  ensure capacity for `count(self)+count(rhs)`, then bulk-copy the RHS entry table
  (offset-adjusted) and RHS data region with `emit_block_copy_advance`. Amortized
  O(1) per appended element.
- **B2 — bulk entry/payload copy in native ops.** Where an op copies a
  *contiguous unchanged span* of entries, replace the per-entry field loop with a
  single bulk copy of the entry bytes, then a tight fix-up loop over just the one
  field that shifts (the value offset, by a uniform delta). Replace per-byte
  payload loops with `emit_block_copy_advance`.

Correctness risk: B1's offset adjustment (RHS entries reference data at a base
that shifts when concatenated) and B2's entry-offset fix-up must reproduce the
existing byte layout exactly. Gate both behind the golden `.ncode` diff and a
value-equality runtime proof.

## Phases

### Phase 1 — B1: bulk in-place append

- [ ] Add `try_inplace_bulk_append_assign` (or extend the existing helper with a
      list-RHS branch) in `builder_inplace_assign.rs`; wire into the assignment
      dispatch in `builder_control.rs`.
- [ ] Lower: capacity-ensure for combined count, bulk entry-table copy with
      per-entry value-offset += `dataLen(self)`, bulk data-region
      `emit_block_copy_advance`.
- [ ] Tests: `tests/func_collection_append_valid/**` extended with the list-into-
      list overload building a large result in a loop (proves O(N) not O(N²) via a
      timed runtime proof), plus `_invalid`.

Acceptance: `flatten`/`append_batch` runtime proof shows O(N) scaling; the
resulting list is `==` to the old rebuild path (value-equality test); golden
`.ncode` re-blessed and acceptance green.
Commit: —

### Phase 2 — B2: bulk memcpy in mid / removeAt / transform / filter

- [ ] `lower_list_mid` (`builder_search.rs`): replace `:1059-1067` byte loop with
      `emit_block_copy_advance`; bulk-copy the entry span, fix value offsets in a
      tight loop.
- [ ] `emit_copy_collection_entries` (`builder_collection_mutate.rs:2896`): bulk
      entry-span copy + offset fix-up; benefits `removeAt`.
- [ ] `lower_collection_transform_call` / `lower_collection_filter_call`
      (`builder_collection_queries.rs`): pre-size the output once, then per element
      only run the callback + write one entry/payload (no re-grow, no full setup).

Acceptance: `mid`/`removeAt`/`transform`/`filter` runtime results byte-identical
to today; measured base cost drops; acceptance green.
Commit: —

### Phase 3 — B2 cont.: insert + replace

- [ ] `lower_list_insert_collection` entry splice (`:645-704`): bulk-copy inserted
      span, uniform offset fix-up.
- [ ] `lower_list_replace` (`builder_strings.rs:304`): bulk entry copy + payload
      `emit_block_copy_advance`.

Acceptance: value-identical results; acceptance green.
Commit: —

## Layout / ABI Impact

None. Output collection bytes (header, entry table, data region) are unchanged;
only the emitted instruction sequences differ. No spec topic changes beyond a note
that these ops use bulk copy internally (optional).

## Validation Plan

- Function tests: every touched op's `_valid`/`_invalid`, all overloads.
- Runtime proof: timed O(N)-scaling test for bulk append; value-equality tests
  comparing new vs a reference list for mid/insert/removeAt/transform/filter/
  replace.
- Acceptance: `scripts/test-accept.sh` (byte-identical output collections;
  allocator/codegen `.ncode` re-blessed).

## Theorized gains (median, full-run; stack on top of plan-25-A)

| bench            | now (ms) | driver                          | Δ (intrinsic) |
|------------------|---------:|---------------------------------|--------------:|
| flatten          | 3644.3   | B1 O(N²)→O(N) bulk append       | −95%          |
| append_batch     |   0.832  | B1                              | −90%          |
| prepend          |   1.887  | B1/B2 bulk head-splice          | −40%          |
| mid              | (post-A) | B2 word-copy payload            | −40%          |
| insert           | (post-A) | B2 entry-span bulk copy         | −45%          |
| removeAt         | (post-A) | B2 entry-span bulk copy         | −45%          |
| transform        | (post-A) | B2 pre-size + single write      | −50%          |
| filter           | (post-A) | B2 pre-size                     | −45%          |
| replace          | (post-A) | B2 payload word-copy            | −40%          |
| chunks / window  | (post-A) | use `__collections_slice`→append| −20% (via B1) |
| groupby          | (post-A) | B2 transform + bulk bucket app. | −30%          |

## Summary

B removes the last genuinely-quadratic list path (bulk append → `flatten`) and
halves the intrinsic cost of the six native per-element ops by using the
word-copy helper the codebase already has. Combined with plan-25-A's arena fix,
every list benchmark reaches its true algorithmic cost.

---

## Completion note (2026-07-06) — DONE, one deviation

Implemented and shipped with the full acceptance suite green (1039 tests, 0
mismatches). Commit on `main`.

- **Phase 1 (B1 bulk in-place append):** done. `try_inplace_bulk_append_assign`
  + `lower_list_bulk_append_in_place`. Amortized O(count(sublist)) per call.
- **Phase 2:** `mid` now word-copies payloads via `emit_copy_collection_entries`
  (was byte-at-a-time); `transform`/`filter` pre-size via `lower_reserved_list`.
- **Phase 3:** `insert` B-splice uses the bulk `emit_bulk_copy_entries_shift`;
  `replace` payload loops use `emit_block_copy_advance`.

**DEVIATION — `removeAt` bulk entry-span is UNSOUND, left on the original path.**
The plan assumed a list's data region is packed in entry order so removeAt could
bulk-copy a compacted data block. It is NOT: `insert`/`prepend`/`set` append the
new element's payload to the data *tail*, so `entry[0].valueOffset` can exceed
later entries' offsets. A subset re-pack (removeAt, and mid) must read each
surviving entry's own `valueOffset`; a contiguous block copy reads garbage / out
of bounds (it corrupted the regex engine's capture lists — the only
`collections::set`-on-list user). `insert` and B1 stay sound because they copy the
*whole* data region verbatim and only shift offsets uniformly. `removeAt` already
word-copied payloads via `emit_copy_collection_entries` (plan-25-A era), so no B2
gain was available there anyway. `mid` was switched to that same sound helper.
Regression coverage: `tests/regression-list-unordered-data-rt`,
`tests/regression-bulk-append-inplace-rt`.

## Follow-up (2026-07-06) — removeAt got its bulk optimization after all

The "removeAt is unsound to bulk" conclusion above was too strong. removeAt
removes exactly one entry, which punches a *single contiguous hole* in the data
region, so the payload compaction is always two verbatim block copies
(before-hole, after-hole shifted left by holeLen) — no per-payload copy —
regardless of data order. My first attempt's real bug was the entry `valueOffset`
fix-up: it shifted by *LUT position* (prefix unchanged, suffix −holeLen) when it
must shift by *data position* (`valueOffset > holeOffset` ⟹ −holeLen). The LUT is
always in list order; the data region is not. `emit_offset_compaction_fixup` does
the per-entry conditional (cheap arithmetic, no memory move). removeAt now does
2 entry-span + 2 data-block copies + one fix-up pass. `mid` genuinely can't use
this (it gathers a scattered subset → many holes) and stays on the per-entry
word-copy (`emit_copy_collection_entries`). Full acceptance still green (1039).
