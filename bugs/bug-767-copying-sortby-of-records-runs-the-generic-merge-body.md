# bug-767: `LET s = collections::sortBy(xs, keyFn)` over a record list (or with a non-integer key) runs the generic `.mfb` merge — one element copy per element per pass — 22× slower than the in-place arm the compiler already has; and `keyFn` is called for a one-element list on one lowering but not the others

Last updated: 2026-09-28
Effort: large (3h–1d)
Severity: MEDIUM
Class: Other (performance) + Correctness (sub-issue B: observable `keyFn` call count differs by lowering)

Status: Open
Regression Test: `tests/runtime/rt_sortby_copying_native.rs` (to be added, Phase 1)

The copying form of `collections::sortBy` — the result bound to a new name, the source left intact, which is how
`elem rec_sortby` and most programs call it — has a native lowering only for **8-byte fixed-width items with
`Integer`/`Fixed`/`Money` keys** (`sort_by_fast_path`). Every other shape — any record element, any `Float`, `Byte`,
`String` or `Scalar` key — runs the source-generic `__collections_sortBy` body, which per merge pass copies both whole
lists and, per element per pass, allocates a copy of the element (`get`) that `set` copies again and then frees:
**10.04 arena allocations per element per sort at n = 1000**. Yet the **self-update** form `s = sortBy(s, keyFn)` over
exactly the same list and key already has a native index-permutation merge (plan-142-C) that handles every element
and key type with **one** allocation per call. Writing `MUT s AS List OF Item = items` / `s = collections::sortBy(s,
keyf)` is measured **22× faster** than `LET s AS List OF Item = collections::sortBy(items, keyf)` for the same result.

| Row (mfb -O1 / C -O0 / Python, ms, min column of `benchmark/baseline/*.log`) | Grade (`./benchmark/rank.py`) |
|---|---|
| `elem rec_sortby` 22.814 / 0.938 / 2.677 (20 sorts of 1000 `Item`s by a `Float` field) | **D (24.3×), slower than Python (8.5×)** |
| sibling, same mechanism for `sort`: `elem flt_sort` 16.509 / 10.268 (`collections::sort` of a `List OF Float`) | C |

**The single correct behavior a fix produces:** a copying `sortBy` (and `sort`) of any list the in-place arm accepts
costs what `MUT s = xs` followed by the in-place `s = sortBy(s, keyFn)` costs — one tight copy of `xs` plus the
native index merge and one permutation pass: **a constant number of arena allocations per call, independent of n**
(measured 1 per sort for the in-place spelling), and `rec_sortby` within **2× C -O0 (≤ 1.9 ms)** (the in-place
spelling measures 49.8 µs per sort, ≈ 1.0 ms per row — measured, see below). The sorted order, stability, the
number and order of `keyFn` calls, and every error are identical to today's.

**Sub-issue B (correctness, found here):** for a one-element list the native Integer-key path calls `keyFn` once,
while the generic body and the in-place arm call it zero times. The man page (`mfb man collections sortBy`) says
"`keyFn` is not called for an empty list and is called once for a one-element list". The same program therefore
observes a different number of `keyFn` calls depending on the element/key types. After the fix, every lowering
calls `keyFn` exactly as the Open Decision settles (recommended: as the man page says).

References:

- `mfb man collections sortBy` (the `DESC` in `src/codegen/builtins/collections/func_sort_by.rs`): O(n log n)
  comparisons, stable, `keyFn` once per element in index order before any comparison, `value` not modified; the
  one-element sentence above.
- `mfb spec memory collections`, Self-updates, Cost ("sort, sortBy … the elements it tests"); the plan-142-C arm
  `src/codegen/collection/assign/builder_inplace_sort.rs` module doc ("Both copying lowerings … run one algorithm:
  a stable bottom-up merge that takes the right run's head only when it is strictly less than the left's").
- `mfb spec language memory-semantics` §14.1, §14.6 (a copy is a copy; the compiler may skip one no program can
  observe).
- `planning/plan-125-findings/C-phase4/man-page-collections-sortBy.md` §1 — where the one-element sentence came from:
  a probe of the native Integer path only.
- Found while triaging the benchmark D/F rows (cluster D). Sibling: bug-766 (record update copies).

## Failing Reproduction

`/tmp/dbA-cases/gen.sh <case> <P>` (the bug-766 probe; `items` = 1000 `Item[id, name := "item<i>", score := 0.5]`,
`bodies` = 1000 `Body[x, vx, mass]` all-`Float`; `keyf(it) = it.score`, `keyi(it) = it.id`, `keyb(b) = b.vx`,
`keybi(b) = toInt(b.vx)`). `--debug` builds give `arena.0.alloc_calls`; allocs/sort = Δ between P = 10 and P = 20
divided by 10. µs/sort from a build without `--debug` at P = 50 (one run, ±20 %).

| Case | Body (inside `FOR pass`) | allocs/sort | µs/sort |
|---|---|---|---|
| **`sortby` (= `rec_sortby`)** | `LET s AS List OF Item = collections::sortBy(items, keyf)` | **10 042** | **1 093.9** |
| `sortby_ikey` | same with the `Integer` key `keyi` | 10 042 | 1 563.7 (debug) |
| `sortby_body` | `LET s AS List OF Body = collections::sortBy(bodies, keyb)` (fixed-width record) | 10 042 | 742.2 |
| `sortby_bodyi` | same with `Integer` key `keybi` | 10 042 | — |
| `sortby_intf` | `List OF Integer` (built in the pass) sorted by `LAMBDA(v) -> toFloat(v)` | 52 | 207.0 |
| **contrast `sortby_inplace`** | `MUT s AS List OF Item = items` / `s = collections::sortBy(s, keyf)` | **1** (4 052 → 4 062) | **49.8** |
| contrast `sortby_native` | `List OF Integer` sorted by `LAMBDA(v) -> v` (the native fast path) | — | 33.1 (incl. ~9.0 µs building the list, `buildints`) |
| `fsort_copy` | `LET s AS List OF Float = collections::sort(fl)` (1000 Floats built in the pass) | 4 341 total at P=10 (not differenced) | 155.4 |
| contrast `fsort_inplace` | `fl = collections::sort(fl)` | 4 142 total at P=10 (not differenced) | 35.9 |

`sortby` P = 10 → 104 461 allocs, P = 20 → 204 881: **10 042 per sort of 1000 elements** (≈ ⌈log₂ 1000⌉ = 10 per
element). Both `sortby` results print the same checksum as the contrast.

- Observed: 10 042 allocations and 1.09 ms per copying sort of 1000 records; 22× the in-place spelling.
- Expected: a constant number of allocations per call and ≈ the in-place spelling's 49.8 µs.

Sub-issue B (`/tmp/dbA-one`, built `-O 1`):

```
IMPORT io
IMPORT collections
TYPE Item
  id AS Integer
  name AS String
END TYPE
FUNC kf(it AS Item) AS Float
  io::print("kf called " & toString(it.id))
  RETURN toFloat(it.id)
END FUNC
FUNC ki(v AS Integer) AS Integer
  io::print("ki called " & toString(v))
  RETURN v
END FUNC
FUNC main() AS Integer
  LET one AS List OF Item = [Item[id := 7, name := "a"]]
  LET s AS List OF Item = collections::sortBy(one, kf)
  io::print("copy rec len " & toString(len(s)))
  MUT m AS List OF Item = [Item[id := 8, name := "b"]]
  m = collections::sortBy(m, kf)
  io::print("inplace rec len " & toString(len(m)))
  LET ones AS List OF Integer = [5]
  LET t AS List OF Integer = collections::sortBy(ones, ki)
  io::print("native int len " & toString(len(t)))
  RETURN 0
END FUNC
```

Observed:

```
copy rec len 1
inplace rec len 1
ki called 5
native int len 1
```

Expected (per the man page): `kf called 7`, `kf called 8` and `ki called 5` all printed — or none, if the Open
Decision goes the other way; never a mix.

## Root Cause

### A — the copying form's native path is gated to 8-byte items and integer keys

`src/codegen/builtins/collections/func_sort_by.rs:sort_by_fast_path` accepts only `item ∈ {Integer, Float, Fixed,
Money}` (or `String` when the source is re-evaluation-safe) **and** `key ∈ {Integer, Fixed, Money}`; everything else
returns `Ok(None)` and the call lowers the `.mfb` `BODY` `__collections_sortBy` (`_mfb_ifn_collections_5FsortBy_24Item_24Float`
in `/tmp/dbA-cases/sortby-10/p.ncode`: 10 898 instructions, 40 `_mfb_arena_alloc` sites). Per merge pass the body
does `MUT itemsDst AS List OF T = items` and `MUT keysDst AS List OF U = keys` (whole-list copies), and per element
moved `itemsDst = collections::set(itemsDst, k, collections::get(items, j))`. In the `.ncode` each such statement is
`list_get_done` → `bl _mfb_arena_alloc` → `flat_copy_wloop` (the `get`'s owned copy of the record,
`src/codegen/memory/owned.rs:materialize_owned_element`) → `set_inplace_*` (copy into the slot; for a variable-width
`Item` whose `name` length differs from the slot's, the relocate/`set_repack_*` path) → `bl _mfb_arena_free`. That is
n·⌈log₂ n⌉ allocations, 2·n·⌈log₂ n⌉ record copies, and — because `Item` payloads differ in size — holes and repacks
in `itemsDst` every pass. `sortby_intf` (8-byte items, `Float` key, same body) costs 52 allocs and 207 µs: the body
alone is ~8× the native path (207 vs 33.1 − 9.0 µs); the record `get`/`set` round trip adds another ~5×.

### Why the contrast is fast

`x = sortBy(x, keyFn)` is a self-update, served by
`src/codegen/collection/assign/builder_inplace_sort.rs:try_inplace_sort_by_assign`: keys for every element first
(`keyFn` called once each, in index order), then `emit_index_merge_sort` over an index permutation in the function's
self-update scratch (the same compare as the body: `SortCompare::Operator` — the language's own `<` — for a `Float`
key), then `src/codegen/collection/list/list_permute.rs:lower_list_permute_in_place` moves each payload once. It
accepts every element type and every ordered key type. The copying form never reaches it because it is only
dispatched from the self-update seam (`SELF_UPDATE_ARMS`), and the function's scratch slot is only reserved when
`prescan_self_update_scratch` (`src/codegen/collection/assign/self_update.rs`) sees a self-update that needs it.

### B — three lowerings, two answers for n = 1

- `func_sort_by.rs:BODY` returns `value` before `collections::transform(value, keyFn)` when `n < 2` — no call.
- `try_inplace_sort_by_assign` branches to `inplace_sortby_skip` when `count < 2`, with the comment "One element:
  nothing to reorder, but the generic body still returns without calling keyFn; so does this." — no call.
- `lower_collection_sortby_call` (the native Integer-key path) builds the keys before its merge loop — one call.

The man page sentence was written from a probe of the third path only (plan-125-findings §1).

## Goal

- Copying `sortBy` over any element type and ordered key type the in-place arm accepts: allocations per call
  constant in n (checkable: `alloc_calls` equal for n = 1000 and n = 2000 up to the list-sized blocks, i.e. the
  count does not scale with n·log n) and `rec_sortby` ≤ 1.9 ms (2× C -O0 0.938 ms; the in-place spelling measures
  ≈ 1.0 ms for the row's 20 sorts).
- Copying `sort` of a `Float`/`Byte`/`Scalar` list: the same, ≈ the in-place spelling (`fsort_inplace` 35.9 µs vs
  `fsort_copy` 155.4 µs per 1000 elements).
- Every lowering calls `keyFn` the same number of times for n = 0, 1, 2 (the Open Decision's answer).
- Results byte-identical to today's for every n ≥ 2, including ties (stability) and `NaN`-free `Float` keys.

### Non-goals (must NOT change)

- **Owner constraint: no change to the language specification or the memory-management model.** Value semantics
  stay: `value` is not modified, the result is an independent list (a copy), elements are copies; results,
  ordering (stable, right head taken only on strict `<`), errors and error codes unchanged; no new syntax, builtin or
  signature; `mfb spec memory` copy/mutate/alias rules, arena and ownership/drop contract and the self-update cost
  contract unchanged. The fix is internal lowering only: it performs the one semantic copy `sortBy` already owes
  (`value` → the result) and sorts that copy with the existing arm — the same thing `MUT s = xs` / `s = sortBy(s, f)`
  compiles to today. No copy is elided that the source could observe.
- `keyFn` is still called once per element, in index order, before any comparison; a failing `keyFn` still raises
  its error with no list returned, and the partial copy is freed.
- The `.mfb` `BODY` stays as the fallback for anything the arm declines (e.g. a function where scratch cannot be
  reserved) — it is not deleted.
- **Forbidden wrong fixes:** rewriting `benchmark/mfb/src/elem.mfb` into the self-update spelling; widening the
  8-byte word merge of `lower_collection_sortby_call` to records by reinterpreting payload bytes; sorting in place
  over `value`'s own block (it must not be modified). For sub-issue B: changing the man page to match whichever
  lowering a test happens to hit, without the owner's decision.

## Blast Radius

Found with `grep -n "fn sort_by_fast_path\|fn sort_fast_path" -A12 src/codegen/builtins/collections/func_sort*.rs`,
`grep -rn "collections::set([a-zA-Z]*, [a-z]*, collections::get(" src/codegen/builtins --include='*.rs'`, and the
probes above.

- `func_sort_by.rs:sort_by_fast_path` / `BODY` — every copying `sortBy` outside {8-byte item} × {Integer/Fixed/Money
  key} — **fixed by this bug** (A).
- `func_sort.rs:sort_fast_path` accepts `String | Integer | Fixed | Money` only; `Float`/`Byte`/`Scalar` copying
  `sort` runs `__collections_sort`'s body — measured 4.3× the in-place arm (`fsort_copy` vs `fsort_inplace`) —
  **fixed by this bug** (same routing; `elem flt_sort` is the row).
- `func_sort_by.rs:lower_collection_sortby_call` (native Integer-key path) — unaffected by A; **fixed by B** only if
  the decision is "no call for n = 1".
- The other `.mfb` bodies with a `set(dst, k, get(src, j))` move (`crypto/helper_mod_l.rs`,
  `crypto/helper_ed25519_sign.rs`, `compress/helper_deflate_core.rs`, `compress/helper_dynamic_block.rs`,
  `compress/helper_package_merge.rs`, `compress/helper_huffman_table.rs`, `regex/helper_run.rs`,
  `http/helper_match_path.rs`) — all move `Integer`/`String` elements, which allocate nothing (scalar) or are the
  bug-536 String contract — **unaffected** (no record element; the record-copy round trip needs a record).
- `sortBy` with a `String` key over records — the in-place arm handles `string_keys` (parked `String` keys freed on
  failure); **fixed by this bug** through the same routing.
- A graph-typed (type-cycle) element list — the tight copy must be the graph copy (`lower_value_owned` /
  `needs_graph_copy`), exactly as `MUT s = xs` does — **in scope**, must be covered by a Phase 1 case.

## Fix Design

1. Extract the body of `try_inplace_sort_by_assign` after `open_inplace_dest` (keys pass → `emit_index_merge_sort`
   → `lower_list_permute_in_place`) into a helper that takes a buffer slot (`emit_sort_by_buffer`), and the same for
   `try_inplace_sort_assign` (`emit_sort_buffer`). The self-update arms call it unchanged — their `.ncode` must be
   byte-identical before and after this step (`O-order-1` / allocation order).
2. In `sort_by_fast_path` / `sort_fast_path`, for any shape the current native path declines and the helpers
   accept: lower `args[0]` as an **owned copy** (the same lowering `MUT s = xs` uses — tight copy, graph copy for a
   cycle type), store it in a slot, run the helper on it, return the slot as the result (owned, fresh). Order of
   evaluation: `value`, then `keyFn`, as today (the `.mfb` call evaluates its arguments in order).
3. Make `prescan_self_update_scratch` reserve the scratch for a function whose body holds a copying `sort`/`sortBy`
   that step 2 will serve (extend the `target_needs_self_update_scratch` predicate). Expected byte shift: every
   function with such a call gains the scratch slot.
4. B: apply the Open Decision in all three lowerings at once (the `n < 2` guard in `BODY`, `inplace_sortby_skip`,
   and `lower_collection_sortby_call`).

Rejected: extending the word merge to wide payloads (moves every payload ⌈log₂ n⌉ times — the cost this bug
removes); making the `.mfb` body's `set(itemsDst, k, get(items, j))` borrow the source element (removes the per-move
allocation but keeps n·log n payload copies and the whole-list copies per pass — still several times the arm, per
`sortby_intf`).

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add `tests/runtime/rt_sortby_copying_native.rs`, modeled on `tests/runtime/rt_inplace_sort.rs` (print the
      copying result beside the self-update result for the same input) plus the bounded-allocation shape of
      `tests/runtime/rt_list_element_record_borrow.rs` (`--debug`, n and 2n, allocations must not scale with
      n·log n). Cases: record element × {`Float`, `Integer`, `String`} key; fixed-width record; `Float` `sort`;
      ties (stability); a failing `keyFn` (no list, `alloc_calls = free_calls`); a type-cycle element; n = 0, 1, 2.
      Confirm the allocation assertions fail today and the value comparisons pass.
- [ ] Add the n = 1 `keyFn`-count case for all three lowerings (it fails today on whichever side the decision
      picks).
- [ ] Record each Blast Radius verdict here after re-running its probe.

Acceptance: the new allocation cases fail for the documented reason; the value cases pass; B's case fails.
Commit: —

### Phase 2 — the fix

- [ ] Steps 1–3 of the Fix Design (`builder_inplace_sort.rs`, `func_sort_by.rs`, `func_sort.rs`, `self_update.rs`).
- [ ] Step 4 (B) after the Open Decision.

Acceptance: Phase 1 cases green; the self-update arms' `.ncode` for `tests/runtime/rt_inplace_sort.rs`'s program is
byte-identical to before step 1; `rec_sortby` probe ≤ 100 µs/sort.
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [ ] Regenerate the `.ncode` goldens that hold a copying `sort`/`sortBy`; inspect one per shape — the delta must be
      the removed `__collections_sortBy`/`__collections_sort` instantiation and the added scratch/permutation code.
- [ ] Full suite; re-run `elem rec_sortby`, `elem flt_sort`, `list sortBy` and every `sortBy` row in the logs.
- [ ] Sync `mfb man collections sortBy` only if B's decision changes the documented behaviour; sync
      `mfb spec memory collections` (the copying `sort`/`sortBy` now share the self-update arm's algorithm) and
      `.ai/collections.md`.

Acceptance: full suite green; the rows meet the Goal.
Commit: —

## Validation Plan

- Regression test: `tests/runtime/rt_sortby_copying_native.rs`.
- Runtime proof: `/tmp/dbA-cases/gen.sh sortby 50` ≤ 100 µs/sort (from 1 093.9), `--debug` P = 10 vs 20 allocation
  delta ≤ 10 per sort (from 10 042); `/tmp/dbA-one` prints a consistent `keyFn` count.
- Doc sync: see Phase 3.
- Full suite: the project's `cargo test` gate (`.ai/testing-gates.md`).

## Open Decisions

- **B: does `sortBy` call `keyFn` for a one-element list?** Recommended: **yes, once** — it is what `mfb man
  collections sortBy` publishes and what the one native path does, and the owner constraint keeps `mfb man`
  behaviour fixed; the generic body and the in-place arm change (a `keyFn` with an effect or a failure becomes
  observable for n = 1, as the man page already says it is). Alternative: no call, and correct the man page — this
  changes documented behaviour and needs the owner's explicit approval.
- Should the `.mfb` bodies stay as fallbacks once every accepted shape is native? Recommended: yes for now (a
  function that cannot take the scratch, or a future key type without `<` lowering, still needs them).

## Summary

The engineering risk is small: the algorithm, the comparison and the permutation already ship in the self-update
arm with a full test matrix; this bug gives the copying form the same path after the one copy it already owes. The
risk concentrates in the extraction step (it must leave the self-update arms byte-identical) and in the scratch
prescan (allocation-order shifts in every function that calls `sort`/`sortBy`). Sub-issue B is a real, observable
inconsistency that needs an owner decision before its one-line fix.
