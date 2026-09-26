# bug-732: six in-place self-updates cost the size of the collection, not the work they do, so build loops that use them are O(n²)

Last updated: 2026-09-25
Effort: large (3h–1d)
Severity: MEDIUM (performance; output is correct)
Class: Other (performance / scalability)

Status: Open
Regression Test: none yet — see Phase 1. Repro: `bugs/repro/bug-732-self-update-arms-cost-collection-size.mfb`

Found by bug-652's Phase 1 audit of `SELF_UPDATE_TABLE`. bug-652 records the owner's
rule (2026-09-25): a `MUT` self-update `x = f(x, …)` must cost time proportional to
the work the call actually does — the elements it adds, removes or moves, or, for an
operation that must test every element, the elements it tests. It must never cost the
size of the part of the collection it leaves alone. An arm that accepts the statement
and still costs O(size) is a bug (`.ai/collections.md`, "In-place mutation").

Six in-place forms break the rule. Each one does O(1) work per call but walks all of
`x`, so a loop over it is quadratic. The output is always correct.

**The single correct behavior a fix produces:** each shape below scales linearly —
4x the iterations costs about 4x the time, the same as the `append` contrast.

References:

- `bugs/bug-652-take-drop-self-reassignment-copies-every-returned-element.md` — the
  rule, and the audit table these rows come from.
- `./mfb spec memory collections` (Self-updates, "Cost") — lists each of these as a
  current exception to the bound.
- `src/codegen/collection/assign/self_update.rs` — `SELF_UPDATE_TABLE`.

## Failing Reproduction

```
CARGO_TARGET_DIR=/tmp/b652-target cargo build --release --bin mfb
mfb init /tmp/b732 && cp bugs/repro/bug-732-self-update-arms-cost-collection-size.mfb /tmp/b732/src/main.mfb
cd /tmp/b732 && mfb build
/usr/bin/time -p ./build/b732.out <shape> <n>
```

Measured on `worktree-B-652` with the bug-652 fix applied, 2026-09-25. The table
gives `user` CPU seconds. The machine was heavily loaded, so wall time was not
usable.

| shape | loop body | n | user | 4n | user | ratio |
| --- | --- | --- | --- | --- | --- | --- |
| `append` (contrast) | `x = append(x, s)` | 10,000 | 0.00 s | 40,000 | 0.01 s | linear |
| `removeAtLast` | append 2, `x = removeAt(x, len(x) - 1)` on `List OF String` | 10,000 | 0.04 s | 40,000 | 0.61 s | ×15 |
| `removeKey` | `m = set(m, k, i)`, `set(m, "tmp", 0)`, `m = removeKey(m, "tmp")` | 10,000 | 0.65 s | 40,000 | 11.76 s | ×18 |
| `setRemove` | `s = add(s, i)`, `add(s, -1)`, `s = remove(s, -1)` | 10,000 | 0.77 s | 40,000 | 15.46 s | ×20 |
| `difference` | `s = add(s, i)`, `s = difference(s, {-5})` | 10,000 | 1.33 s | 40,000 | 23.67 s | ×18 |
| `symDiff` | `s = add(s, i)`, `s = symmetricDifference(s, {-5})` | 10,000 | 1.27 s | 40,000 | 22.41 s | ×18 |
| `stripSuffix` | `s = s & "x"`, `s = strings::stripSuffix(s, "q")` | 100,000 | 0.26 s | 400,000 | 3.95 s | ×15 |
| `popSorted` | `x = sort(x)` once, then `x = take(x, len(x) - 1)` until empty | 10,000 | 0.04 s | 40,000 | 0.72 s | ×18 |

- Observed: every shape except `append` costs about 16x for 4x the iterations.
- Expected: about 4x, like `append`.

Contrast: `popAppended` pops the same list without sorting it first. It is linear on
the bug-652 fix (0.00 s → 0.01 s). `take`/`drop` that remove nothing are O(1), which
bug-652 pins in `tests/runtime/rt_list_take_drop_in_place.rs`.

## Root Cause

Each finding was first reported by a read-only audit (bug-652 Phase 1). It was then
spot-checked by grepping the named loop labels, and confirmed by the timings above.

1. **`removeAt`, variable-width list.** `lower_list_remove_at_in_place` calls
   `emit_offset_compaction_fixup(…, "remove_inplace_fix")`
   (`src/codegen/collection/buffer/collection_buffer.rs`). That loop visits every
   surviving entry to fix its offset, whatever the index. It also moves every data
   byte after the hole in *data* order, which is not entry order. So removing the
   last element costs O(count). The fixed-width (`kind2`) path moves only the suffix.
2. **`removeKey` (Map) and `remove` (Set).** `lower_map_remove_key_in_place`
   (`src/codegen/collection/map/map_mutate.rs`) makes three passes:
   - `mrk_scan_loop` searches the key linearly, even when the key type is
     probe-eligible.
   - `mrk_shift_loop` shifts every later entry.
   - It stores `COLLECTION_OFFSET_BUCKETS_READY = 0`, so the next probe rebuilds the
     whole bucket index.
3. **`difference`.** `try_inplace_filter_set`
   (`src/codegen/collection/assign/builder_inplace_setmap.rs`) runs
   `emit_membership_marks` over all of `s`, then `lower_map_compact_in_place`
   (`mcompact_loop`), which resets the bucket index. Removing a one-element `t` costs
   O(|s|).
4. **`symmetricDifference`.** `try_inplace_symmetric_difference_assign` marks all of
   `s` (`inplace_symdiff_s`) and compacts it, even though at most |t| elements change.
5. **`String` window family.** `try_inplace_string_window_assign`
   (`src/codegen/collection/assign/string_self_update.rs`) always copies the whole
   result window to the start of the block (`inplace_str_window_move`). It does so
   even when the window already starts there. So `stripSuffix` and a no-match
   `stripPrefix` copy every kept byte onto itself. `trimEnd` also scans the string
   forward before trimming (`strings_trim_end_loop`, `gen_trim.rs`). Lesser cases
   (`mid`/`left` from 0, `trim` with nothing leading) pay the same self-copy.
6. **`take`/`drop`/`mid` truncation of an out-of-order list.** bug-652's fast path
   (`emit_compact_range_truncate`, `src/codegen/collection/list/list_compact.rs`)
   retracts `dataLength` only when the removed payloads are the tail of the data
   region. After `sort`, `prepend`, `insert` or a relocating `set` they are not.
   The call then falls through to the general compaction, which walks every entry
   to count live bytes for its dead-byte repack trigger. A list whose payloads stay
   out of order pays O(count) on every pop. The live-byte total is not stored
   anywhere, so it can only be recomputed by that walk.

## Goal

- Every shape in the table scales linearly (4x iterations ≈ 4x CPU time).
- `SELF_UPDATE_TABLE`'s `Arm` rows all meet the cost rule. The "Cost" exception list
  in `./mfb spec memory collections` becomes empty and is deleted.

### Non-goals (must NOT change)

- Value semantics, failure atomicity, the drop obligations of removed graph-owning
  elements, and the allocation bounds pinned by `tests/runtime/rt_inplace_self_update.rs`.
- Map/Set iteration order (insertion order).
- Do not "fix" a row by making its arm decline. Per the rule, a declining arm is
  correct only where a copy is required.

## Blast Radius

The audit table in bug-652 covers every `Arm` row. Rows not listed here met the rule
when measured or read. Note (not an offender under the rule): `set`/`add`/`union`/
`merge` scan linearly (`mapset_loop`) for a key type that is not probe-eligible
(`map_key_probe_eligible`). That cost belongs to the data structure's lookup, not to
the arm.

## Fix Design

Not designed. Each finding is independent and can be its own phase:

- Findings 1 and 6 need either dead-byte accounting or a cheaper order/tail test.
  Adding a header field would change the collection layout, which bug-652 rejected
  for its own fix. Weigh that first.
- Findings 2–4 need a removal that probes the key, and bucket maintenance that does
  not rebuild the whole index.
- Finding 5 needs a skip when the window already starts at the block start, and a
  backward scan for `trimEnd`.

## Phases

### Phase 1 — failing tests
- [ ] A scaling test per finding (CPU-time ratio, as `tests/runtime/rt_list_take_drop_in_place.rs`).
Commit: —

### Phase 2 — fixes
- [ ] One per finding; update the spec "Cost" exception list as each lands.
Commit: —

### Phase 3 — full validation
- [ ] Full suite.
Commit: —

## Summary

bug-652 fixed the take/drop build loop and made the cost rule explicit. Its audit
found five more in-place forms, plus bug-652's own out-of-order truncation case,
that still walk the whole collection. All six are measured quadratic above. The
spec documents them as current exceptions until this bug lands.
