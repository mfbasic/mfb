# bug-652: an in-place `take`/`drop` over a variable-width list walks the whole list, so a self-reassigning build loop is still O(n²)

Last updated: 2026-09-25
Effort: medium (1h–2h)
Severity: MEDIUM (performance; output is correct — but see "In-place is required")
Class: Performance / scalability

Status: Open — **rescoped 2026-09-25.** The original defect (no in-place arm, a fresh
copy plus a per-element graph deep copy) was fixed by plan-142-B (`a70924e35`,
2026-09-21) and the flat `List OF Integer` case is now linear. What remains is the
variable-width compaction's cost: it is O(count) per call even when it removes nothing.
**2026-09-25:** Phases 1–2 landed on `worktree-B-652`, along with the Phase 3 re-measure
(linear). The full suite is still owed. The other audit offenders are filed as bug-703.
Regression Test: `tests/runtime/rt_list_take_drop_in_place.rs`. Repro: `bugs/repro/bug-652-take-drop-variable-width-compaction.mfb`

`pending = collections::take(pending, len(pending))` and
`pending = collections::drop(pending, 0)` now run in place
(`try_inplace_take_assign` / `try_inplace_drop_assign` →
`lower_list_compact_in_place`), with no allocation and no graph copy. But for a
variable-width element type the compaction walks every entry of the list, up to three
times, on every call, even when the kept range is the whole list. So a loop that
rewrites a shared pending list one element at a time is still O(n²): at 100,000 nodes
`wideTake` takes 42 s where `wideAppend` takes 0.07 s.

The output is correct. This is a scalability defect, not a miscompile.

**In-place is required, not an optimization (owner decision, 2026-09-25).** A `MUT`
self-update `x = collections::f(x, …)` must cost time proportional to the work the call
actually does (elements added, removed or moved), never to the size of the untouched
collection, at every optimization level. Without that the language is not functional
for ordinary build loops. The spec and `.ai/collections.md` still describe in-place
update as an optimization the compiler may skip; this bug corrects them (Phase 2).

**The single correct behavior a fix produces:** `x = collections::take(x, k)`,
`x = collections::drop(x, k)` and `x = collections::mid(x, …)` on an owned,
uniquely-referenced `MUT` binding cost O(elements removed) — O(1) when nothing is
removed — for fixed-width and variable-width elements alike, so `wideTake` /
`wideDropAll` scale like `wideAppend` (4x the elements, ~4x the time).

**Not blocking plan-138.** Found while executing plan-138-A Phase 1 (`packages/xml`).
That plan uses a recursive-descent builder whose per-element children list only ever
receives `append`, which is linear (0.11–0.13 s at 100,000 nodes), so plan-138 has a
working path and does not wait on this fix.

References:

- `src/codegen/collection/list/list_compact.rs` — `lower_list_compact_in_place`, the
  plan-142-B compaction primitive shared by `filter`, `distinct`, `take`, `drop`, `mid`.
- `src/codegen/collection/assign/builder_inplace_shrink.rs` —
  `try_inplace_take_assign`, `try_inplace_drop_assign`.
- `.ai/collections.md` — "In-place mutation: one table, four sites" and its
  "Declining is always correct" rule, which the owner decision above overturns.
- `src/docs/spec/language/12_collections.md` ("Destructive update is an optimization
  only …") and `src/docs/spec/language/14_memory-semantics.md` ("The compiler may
  choose … destructive update") — the sentences Phase 2 turns into a guarantee.
- `planning/completed/plan-142-B-shrink-arms.md` (`a70924e35`) — the in-place filter/take/drop/mid/
  distinct arms that fixed the original report.

## Failing Reproduction

(As filed 2026-09-15. The measurement tables in this section are the **original**
numbers, before plan-142-B; see "Re-measured" below for HEAD.)

A package declaring a `Node` union that reaches a type cycle (`Element.children` is a
`List OF Node`), plus four loop shapes that differ only in which `collections` call is
self-assigned. `mfb build` with the release compiler built from `worktree-P-138` HEAD
(`target/release/mfb`, `MFBasic Compiler 0.1.0`, `2026-09-15 22:29:23 UTC`).

```
' package: element type reaches a type cycle
EXPORT TYPE Element
  name AS String
  attributes AS List OF Attribute
  children AS List OF Node
END TYPE

EXPORT UNION Node
  Element
  Text
  Comment
  ProcessingInstruction
END UNION

' append only — hits try_inplace_append_assign
FUNC wideAppend(n AS Integer) AS List OF Node
  MUT pending AS List OF Node = []
  FOR i = 1 TO n
    pending = collections::append(pending, leaf("i"))
  NEXT
  RETURN pending
END FUNC

' take-all: returns EVERY element
FUNC wideTake(n AS Integer) AS List OF Node
  MUT pending AS List OF Node = []
  FOR i = 1 TO n
    pending = collections::take(pending, len(pending))
    pending = collections::append(pending, leaf("i"))
  NEXT
  RETURN pending
END FUNC

' drop-none: returns NO elements (result discarded)
FUNC wideDrop(n AS Integer) AS List OF Node
  MUT pending AS List OF Node = []
  FOR i = 1 TO n
    LET kids AS List OF Node = collections::drop(pending, len(pending))
    pending = collections::append(pending, leaf("i"))
  NEXT
  RETURN pending
END FUNC

' drop-all: returns EVERY element
FUNC wideDropAll(n AS Integer) AS List OF Node
  MUT pending AS List OF Node = []
  FOR i = 1 TO n
    pending = collections::drop(pending, 0)
    pending = collections::append(pending, leaf("i"))
  NEXT
  RETURN pending
END FUNC
```

`intTake` / `intDropAll` are the same two self-assigning loops over a `List OF Integer`,
whose element type does not reach a cycle.

The probe used to measure this lives in `/tmp/xml-builder-spike` (scratch, per AGENTS.md
a one-off probe belongs in `/tmp` or the bug doc, never `scripts/`); the sources above are
the load-bearing part and are reproduced here so the doc outlives the directory.

### Measured — n=8000, 3 runs each

Command (run on this branch, 2026-09-15; these six rows are my own runs):

```
bash /tmp/xml-builder-spike/time.sh /tmp/xml-builder-spike/app2/build/spike.out 8000 \
     wideAppend wideTake wideDrop wideDropAll intTake intDropAll
```

| shape | element type | elements returned per step | run1 | run2 | run3 |
| --- | --- | --- | --- | --- | --- |
| `wideAppend` | `Node` (cyclic) | — (in-place) | 0.03 s | 0.02 s | 0.02 s |
| `wideTake` (take-all) | `Node` (cyclic) | all | 21.68 s | 21.71 s | 21.85 s |
| `wideDrop` (drop-none) | `Node` (cyclic) | none | 0.02 s | 0.02 s | 0.02 s |
| `wideDropAll` (drop-all) | `Node` (cyclic) | all | 23.10 s | 25.94 s | 22.11 s |
| `intTake` (take-all) | `Integer` (flat) | all | 0.62 s | 0.63 s | 0.62 s |
| `intDropAll` (drop-all) | `Integer` (flat) | all | 0.62 s | 0.62 s | 0.62 s |

Independently measured with the same harness and command by the plan-138-A coordinator:
take-all 21.96–21.98 s, drop-all 21.65–22.16 s, append 0.02 s, drop-none 0.02 s,
`List OF Integer` take-all and drop-all 0.61–0.62 s. My `wideDropAll` row runs high
(23.10–25.94 s vs 21.65–22.16 s) because it executed alongside a source-tree grep sweep;
the contention is in the noise of the effect being measured and changes no conclusion.

Three facts follow, and each is pinned by a contrast row rather than by one measurement:

1. **Cost tracks elements RETURNED, not the call.** `drop-none` returns zero elements and
   is free (0.02 s); `drop-all` returns every element and costs ~22 s. Both are one
   `collections::drop` per iteration.
2. **It is quadratic for both element types**, not only the cyclic one — the flat
   `List OF Integer` loops still cost 0.62 s where `append` costs 0.02 s.
3. **The cyclic element type pays ~35x more per element** (21.9 s vs 0.62 s for the same
   loop and the same n), which is a second, multiplicative cost on top of the copy.

### Measured — quadratic scaling

A builder that rewrites the shared `pending` list with `take`/`drop` per close tag
(the first spike, `/tmp/xml-builder-spike/app/build/spike.out`, shape `wide`).
**These three rows were measured by the plan-138-A coordinator, not by me**, with:

```
bash /tmp/xml-builder-spike/time.sh /tmp/xml-builder-spike/app/build/spike.out <n> wide
```

| nodes | time |
| --- | --- |
| 2,000 | 1.37–1.38 s |
| 8,000 | 22.21–22.31 s |
| 100,000 | exceeded the harness's 30 s alarm (exit 142) |

4x the nodes costs 16x the time — quadratic, not linear.

- Observed: the take/drop builder is O(n²) and unusable past a few thousand nodes.
- Expected: linear, as the `append`-only builder already is (0.11–0.13 s at 100,000
  nodes, plan-138-A Phase 1).


### Re-measured at `79bd6e39b` (2026-09-25) — the original report is stale, the scaling is not

The `/tmp/xml-builder-spike` probe is gone; its load-bearing source (the types and the
six shapes above) is now `bugs/repro/bug-652-take-drop-variable-width-compaction.mfb`
(types declared in the program rather than a package). Commands:

```
CARGO_TARGET_DIR=/tmp/b652-target cargo build --release --bin mfb
mfb init /tmp/b652/app && cp bugs/repro/bug-652-take-drop-variable-width-compaction.mfb /tmp/b652/app/src/main.mfb
cd /tmp/b652/app && /tmp/b652-target/release/mfb build
/usr/bin/time -p ./build/app.out <shape> <n>
```

| shape | n=8,000 | n=32,000 | n=100,000 | 2026-09-15 (n=8,000) |
| --- | --- | --- | --- | --- |
| `wideAppend` | 0.00 s | 0.02 s | 0.07 s | 0.02 s |
| `wideTake` (take-all) | 0.09–0.10 s | **2.89 s** | **42.14 s** | 21.7 s |
| `wideDropAll` (drop-none-kept-all) | 0.09–0.19 s | **4.96 s** | **24.47 s** | 22.1 s |
| `wideDrop` (result to another name) | 0.01 s | — | — | 0.02 s |
| `intTake` / `intDropAll` | 0.00 s | 0.00 s | 0.00 s | 0.62 s |

- The flat case is **fixed**: `List OF Integer` take-all/drop-all is 0.00 s even at
  100,000 (the fixed-width `KeepSource::Range` fast path in `emit_compact_fixed_width`).
- The variable-width case is ~200x faster at n=8,000 but still **super-linear**: 4x the
  nodes costs 29x (`wideTake` 0.10 → 2.89 s), and 100,000 nodes takes 42 s. Why it is
  worse than 16x was not measured; cache effects are a guess.
- `sample <pid> 5` on `wideTake 100000` (binary unsymbolicated, `nm` lists 9 symbols)
  puts almost every sample in three inline loops whose shape matches
  `lower_list_compact_in_place`: the dropped-element pass, the order probe, and the
  in-order slide — on their compare/branch instructions, not the word-copy loop. The
  clamp (`start = 0`, `len = clamp(n, 0, count)`) sits just before them. No allocation
  or `_mfb_rt_graph_copy` call is hot. Mapping samples to passes is a reading of the
  disassembly (`otool -tv`), not a symbolicated profile.

## Root Cause

**`lower_list_compact_in_place` is O(count) for a variable-width element type, even
for a `KeepSource::Range` that keeps everything.** Read at `79bd6e39b`
(`src/codegen/collection/list/list_compact.rs`):

1. **Dropped-element pass.** When `kind2_payload_size(element_type).is_none() &&
   self.owns_graph(element_type)` — true for `Node` — a loop runs `index` over
   `[0, count)` and calls `emit_keep_branch` on every entry to find the ones to
   `emit_drop_list_element`. For a `Range` the dropped entries are exactly
   `[0, start)` and `[start + len, count)`; the loop visits all `count`.
2. **Order probe** (`emit_compact_entries`, `compact_probe_loop`). Walks every entry to
   decide whether each payload sits at the aligned end of the previous one. O(count),
   regardless of how many entries survive.
3. **In-order slide** (`compact_ord_loop`). Walks every entry again and, for each kept
   one, compares the payload and entry against their destination
   (`compact_ord_payload_ok` / `compact_ord_entry_ok`) — nothing moves when
   `start = 0`, but every entry is still visited.

The fixed-width path does not have this problem: `emit_compact_fixed_width` special-cases
`KeepSource::Range` as one block slide (skipped when `start == 0`) plus a header write.
That is why `intTake`/`intDropAll` are 0.00 s at 100,000 and `wideTake` is 42 s.

For `filter` and `distinct` (`KeepSource::Marks`) the full walk is inherent — every
element must be tested — so their O(count) is the call's own work, not this bug.

### Historical root cause (fixed by plan-142-B, `a70924e35`)

As filed on 2026-09-15: `take`/`drop` had no self-update arm, so
`x = collections::take(x, …)` fell through to the copying reassignment, which built a
fresh list via `__collections_slice` → `lower_list_slice_range` and, for a type
reaching a cycle, deep-copied each element's graph through
`own_collection_payload_edges` → `_mfb_rt_graph_copy` (the 35x multiplier). plan-142-B
added `ArmId::Take` / `ArmId::Drop` (`SELF_UPDATE_ARMS`, `SELF_UPDATE_TABLE` rows
`collections::take` / `collections::drop` are `SelfUpdate::Arm`), which compact in
place and never reach the slice or the graph copy.

## Goal

- `x = collections::take(x, k)`, `x = collections::drop(x, k)` and
  `x = collections::mid(x, …)` over a variable-width list cost O(elements removed):
  O(1) when the kept range is the whole list.
- `wideTake` / `wideDropAll` scale linearly: 4x the nodes costs ~4x the time, and
  100,000 nodes lands in the same order of magnitude as `wideAppend` (0.07 s).
- The spec and `.ai/collections.md` state in-place self-update as a guaranteed cost
  bound, not an optimization.

### Non-goals (must NOT change)

- **Value semantics.** `collections::take(a, k)` where the result is bound to a *different*
  name, returned, or passed on must keep copying — `a` must be unaffected. Only the
  self-assignment `x = take(x, …)` may mutate in place, exactly as `append` does today.
- **The currently-free shapes stay free.** `wideAppend`, `wideDrop`, `intTake`,
  `intDropAll` must not get slower.
- **Do not weaken the repro to make it pass.** Re-timing a smaller n, or swapping the
  cyclic `Node` union for a flat element type, hides the effect rather than fixing it.
- **Clamping semantics.** `take(count)` = `slice(0, count)` and `drop(count)` =
  `slice(count, len)`, with `start`/`stop` clamped into range, must be preserved
  bit-for-bit.
- **Drop obligations.** Every removed element that owns a graph is still freed exactly
  once (plan-134-H); only the *kept* elements stop being visited.
- **`filter` / `distinct`** keep their full walk (see Root Cause).

## Blast Radius

Found by `grep -rn "lower_list_compact_in_place\|KeepSource::Range" src/codegen`.

- `collections::take`, `collections::drop`, `collections::mid` — every
  `KeepSource::Range` caller of `lower_list_compact_in_place`. Fixed by this bug.
- `collections::filter`, `collections::distinct` — `KeepSource::Marks`; share the
  primitive, must be unaffected.
- The field-site forms (plan-145: `r = WITH r { f := collections::take(r.f, …) }`,
  `h.state.f = …`) go through the same arms and the same primitive, including the
  out-of-order repack's `InlineGrow`; they must keep passing the plan-145 matrix.
- **The guarantee's wider audit.** `SELF_UPDATE_TABLE` marks 23 `collections::` members
  `Arm` and `reduce`/`reduceRight` `Exempt`
  (`grep -A1 'function: "collections::' src/codegen/collection/assign/self_update.rs`).
  "Has an arm" does not prove "costs the call's own work": this bug is an arm that
  exists and is still O(count). Phase 1 audits every `Arm` row's cost shape against
  the guarantee; any other arm found O(count) where the call's work is smaller is fixed
  here or filed.

## Fix Design

Give `emit_compact_entries` (and the dropped-element pass) the `KeepSource::Range`
fast path the fixed-width path already has:

- **Dropped-element pass:** for a `Range`, loop only over `[0, start)` and
  `[start + len, count)` instead of `[0, count)` with a keep test.
- **`start == 0` (take, and drop/mid of nothing at the front):** a truncation. Set
  `count = len`; the kept entries and payloads do not move. `dataLength` becomes the end
  of entry `len - 1`'s payload when the data is in order; when it is not, leaving the
  tail as dead bytes is already tolerated (the `emit_repack_list_data` contract the
  out-of-order path relies on), and the existing dead-bytes-exceed-live-bytes repack
  trigger still bounds the block. Which of the two to take must not need the full order
  probe — decide it from the kept range only, or keep `dataLength` conservative.
- **`start > 0`:** slide only the kept range down. The probe and slide touch
  `len` entries, and the drop pass touches `start + (count - start - len)` entries, so the
  call is O(len + removed); with the self-update loop shapes in the repro this is O(1)
  per call only when `start == 0` or `len == 0`. A drop-from-the-front loop that keeps a
  long tail is inherently O(len) per call on a contiguous array; say so in the spec
  guarantee (cost proportional to elements moved) rather than promise O(1).
- Keep `KeepSource::Marks` on the current path.

Rejected: an offset/start field in the list header so `drop` is O(1) without moving
data — changes the collection layout every consumer reads, for a shape (drop from the
front, keep a long tail) the repro does not exercise.

## Corrections

- **C1 (2026-09-25, found while implementing Phase 2).** Fix Design says an
  out-of-order truncation can leave the removed tail as dead bytes, because "the
  existing dead-bytes-exceed-live-bytes repack trigger still bounds the block". That
  trigger needs the list's live-byte total. The header does not store it, and
  computing it walks every kept entry, which is O(count). Skipping the walk is not
  safe: `append`'s grow copies `dataLength` verbatim, dead bytes included, so a
  pop/append loop over an out-of-order list would grow the block without bound. So the
  shipped fast path (`emit_compact_range_truncate`) takes only the cases that need no
  live count:
  - nothing is removed;
  - everything is removed;
  - every removed payload is the data tail, which always holds for a list built by
    `append`.

  Otherwise it falls through, having written nothing, to the unchanged general path.
  Measured residual, with the bug-703 repro on the fixed compiler: `popSorted` (sort
  once, then `x = take(x, len(x) - 1)` until empty) goes from 0.04 s to 0.72 s user
  for 4n (×18). `popAppended`, the same pops without the sort, is linear. Filed as
  **bug-703** finding 6. The spec lists it as a current exception.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [x] Land a scaling test for `wideTake` / `wideDropAll` (variable-width, type reaching a
      cycle) that times n and 4n and fails when the ratio is super-linear, per the
      perf-golden conventions in `.ai/testing-gates.md`. Confirm it fails at HEAD.
      → `tests/runtime/rt_list_take_drop_in_place.rs`
      (`a_take_all_…_is_linear`, `a_drop_none_…_is_linear`). **Deviation from "n and
      4n":** each shape runs the same total number of elements two ways, as 8 builds of
      `n` and as 1 build of `8n`. Linear work gives ×1 and a per-call O(size) walk gives
      ×8, with the bound at ×3 + 0.05 s. The time is the child's CPU time, taken from
      `wait4`'s `ru_utime + ru_stime`, not wall time. This box ran at load average ≈ 207
      (`uptime`), and wall time was unusable: `wideAppend` 1,000,000 measured 4.61 s wall
      for 0.71 s user. A plain n-vs-4n wall ratio also needs n ≥ 25,000 before the
      fixed shapes rise above timer noise, and at that size HEAD takes minutes.
      **RED at HEAD `4ef538bb9`**, with release `mfb` built from that tree:
      `CARGO_TARGET_DIR=/tmp/b652-target MFB_TEST_EXE=/tmp/b652-target/release/mfb cargo test --test rt_list_take_drop_in_place variable_width -- --test-threads=1 --nocapture`
      → `wideTake: 8×5000 = 0.399 s CPU; 1×40000 = 3.020 s CPU; ×7.58` and
      `wideDropAll: 8×5000 = 0.412 s CPU; 1×40000 = 3.131 s CPU; ×7.60`, both
      panicking. The ratio is ×7.6 against the ×8 that O(count) per call predicts.
      Mechanism confirmed before writing the test: `-ncode` for `wideTake`
      (`/tmp/b652/app`) holds `compact_drop_loop`, `compact_probe_loop`,
      `compact_ord_loop` and `compact_dis_loop`, with no slice and no graph copy.
      `sample` of `wideTake 100000` puts 2,592 of 2,616 samples in one inline range
      (`0x1b90`–`0x1d60`) with no call on the stack. That matches the doc's Root Cause.
- [x] Add a guard that `intTake` / `intDropAll` / `wideAppend` / `wideDrop` stay linear.
      → `the_already_linear_shapes_stay_linear`. It is green at HEAD: `wideAppend`
      ×1.60, `wideDrop` ×1.28, `intTake` ×1.31, `intDropAll` ×1.41.
- [x] Add a value-semantics test: `LET b = collections::take(a, k)` leaves `a` intact,
      flat and cyclic element types.
      → `take_drop_mid_keep_value_semantics_and_the_right_elements`. It covers
      `take`/`drop`/`mid` as copies and through an alias (`LET keep = x` then
      `x = take(x, 1)`). The element types are a cyclic union `Node`, `String` and
      `Integer`. It also runs self-updates over lists whose payloads are out of entry
      order: after `prepend`, `insert`, a longer `set`, and `sort`. Green at HEAD. The
      expected output was checked by hand.
- [x] Add a leak test: a `take` / `drop` / `mid` that removes graph-owning elements
      ends `--debug` with `alloc_calls = free_calls`, `live_bytes 0`.
      → `take_drop_mid_free_every_removed_element_once`, which runs the same program.
      Green at HEAD: `alloc_calls 532`, `free_calls 532`, `live_bytes 0`,
      `double_free_skips 0`.
- [x] Audit every `SelfUpdate::Arm` row in `SELF_UPDATE_TABLE` for cost proportional to
      the call's work, not to the collection's size. Record the table here; fix or file
      each offender. → The table is below. Five offenders were found, plus this bug's
      own out-of-order residual (see Corrections). They are too large for this bug, so
      they are filed as **bug-703** with a measured repro
      (`bugs/repro/bug-703-self-update-arms-cost-collection-size.mfb`).

#### Audit — `SELF_UPDATE_TABLE` `Arm` rows (2026-09-25)

A read-only audit of each arm's lowering produced this table. Each offender was then
spot-checked by grepping its named loop label, and measured with the bug-703 repro
(user CPU seconds, n → 4n).

| ArmId (rows) | Inherent work | Emitted cost | Verdict |
| --- | --- | --- | --- |
| Append | 1 element | writes into spare room; geometric realloc | OK |
| BulkAppend | size of the RHS | copies the RHS; geometric realloc | OK |
| Set | 1 element | same-size overwrite O(1); a length change relocates to the tail, with a geometric repack on overflow | OK (amortized) |
| SetAdd | 1 element | hash probe, then bucket put | OK |
| SetRemove, RemoveKey | 1 element | `mrk_scan_loop` linear, `mrk_shift_loop` suffix, `BUCKETS_READY = 0` (next probe rebuilds the index) | **OFFENDER** → bug-703 (×18 / ×20) |
| Prepend | moves n | shifts all entries | OK (moved) |
| RemoveAt | moves the suffix | kind2 suffix only; variable-width `remove_inplace_fix_loop` over **all** entries | **OFFENDER** (var-width) → bug-703 (×15) |
| Insert | moves count − i | shifts the tail | OK |
| Concat | size of the RHS | copies the RHS into spare capacity; geometric regrow | OK |
| Filter, Distinct | tests all | mark pass plus Marks compaction | OK |
| Take, Drop, Mid | removed + moved | variable-width: probe + slide over all entries | **this bug** (fixed in Phase 2; out-of-order truncation → bug-703) |
| Replace, Transform, Math, MapValues, Intersection | tests all | one walk | OK |
| Sort, SortBy | n log n | merge sort plus permute | OK |
| Union, Merge | size of t / n | walks only the argument; geometric reserve | OK |
| Difference | ≤ size of t | marks **all of s**, `mcompact_loop` over s | **OFFENDER** → bug-703 (×18) |
| SymmetricDifference | size of t | marks t **and all of s**, compacts s | **OFFENDER** → bug-703 (×18) |
| StrIdentity | 0 | nothing | OK |
| StrWindow (13 rows) | removed bytes | always `inplace_str_window_move` of the whole window, even when it does not move | **OFFENDER** for `stripSuffix`, no-match `stripPrefix`, `trimEnd` → bug-703 (`stripSuffix` ×15) |
| StrGrow | added (+ moved) | geometric reserve | OK |
| StrRewrite | tests all | build in scratch, copy back | OK |

Not an offender under the rule: a non-probe-eligible key type makes Map/Set lookups
linear (`mapset_loop`). That cost belongs to the lookup, not to an arm.

Acceptance: the scaling test fails at HEAD; the guards pass at HEAD. ✓ (above)
Commit: —

### Phase 2 — the fix

- [x] `KeepSource::Range` fast path in `lower_list_compact_in_place` /
      `emit_compact_entries` (Fix Design).
      → `src/codegen/collection/list/list_compact.rs`:
      - The dropped-element pass jumps its index over a range's kept run, so it visits
        only `[0, start)` and `[start + len, count)`. The keep test is unchanged for
        Marks.
      - The new `emit_compact_range_truncate` runs before the probe for a Range:
        - `len == count` does nothing (O(1));
        - `start == 0 && len == 0` resets `count` and `dataLength`;
        - `start == 0` walks the removed entries from last to first, retracting
          `dataLength` while each payload ends (aligned) at it, then writes `count` and
          `dataLength`;
        - anything else, and a removed payload that is not the tail, falls through to
          the unchanged probe/slide path (C1).
      - `start > 0` keeps the general path. That path's walk is O(len + removed), which
        is the call's own work, because every kept element moves.
      - `KeepSource::Marks` is unchanged.
      - New label tests in `src/codegen/builtins/tests/inplace_compact.rs`:
        `compact_in_place_range_form_on_an_entry_list_tries_the_truncation_first` and
        `compact_in_place_range_drop_pass_skips_the_kept_run`.
      - The existing `compact_in_place_on_an_entry_list_probes_the_payload_order` still
        passes unedited, because the general path is still emitted as the fallback.
- [x] Spec: `12_collections.md` — replace "Destructive update is an optimization only
      for the assignment-back-to-the-same-`MUT` pattern" with the guarantee: a `MUT`
      self-update is performed in place, and its cost is proportional to the elements it
      adds, removes or moves, at every optimization level. State where the guarantee does
      not apply (the old value is still read by another live binding, so a copy is
      required). `14_memory-semantics.md` (opening paragraph) "may choose … destructive update" —
      align with the guarantee. Cite the arms (`[[path:Symbol]]`, `.ai/specifications.md`).
      → §12 now states the guarantee and the value-semantics limit, and links to
      `mfb spec memory collections`. §14's opening paragraph is aligned with it.
      `memory/05_collections.md` Self-updates gains a **Cost** bullet with the
      take/drop/mid cost model. It also lists the forms not yet within the bound
      (bug-703's six findings), as the as-is rule requires. Every claim is cited
      (`try_inplace_self_update`, `emit_compact_range_truncate`,
      `lower_list_compact_in_place`, `emit_offset_compaction_fixup`,
      `lower_map_remove_key_in_place`, `try_inplace_filter_set`,
      `try_inplace_symmetric_difference_assign`,
      `try_inplace_string_window_assign`).
      "At every optimization level" was measured: `wideTake`/`wideDropAll 100000`
      built `-O0` gave 0.07 / 0.07 s user, and `-O3` gave 0.06 / 0.08 s.
- [x] `.ai/collections.md` — replace "**Declining is always correct**" with: declining
      is correct only where the spec says a copy is required; an arm that declines
      anywhere else, or accepts but costs O(size), is a bug.
      → Done. Also added a "Shrink compaction cost" bullet explaining why an
      out-of-order truncation cannot skip the live-byte walk.
- [x] `mfb man collections` — check `take`/`drop`/`mid`/`append` pages describe the
      in-place self-update in developer terms (`.ai/man-content.md`: "mutate", "copy").
      → `append` already did. `take`/`drop`/`mid` (`func_take.rs`, `func_drop.rs`,
      `func_mid.rs` DESC) each gain an "Assigning straight back to the same `MUT`
      variable" paragraph modeled on `append`'s. Verified by rendering
      `mfb man collections take|drop|mid`. `MFB=… scripts/man-census.sh --memory-scope`
      → `unclassified memory-vocabulary hits: 0`.
      `MFB=… scripts/man-run-examples.sh collections --run` →
      `examples: 140 built: 140 ran: 140 not run: 0 failed: 0`.

Acceptance: Phase 1's scaling test passes; value, leak and linear guards unchanged.
**GREEN:** `CARGO_TARGET_DIR=/tmp/b652-target MFB_TEST_EXE=/tmp/b652-target/release/mfb cargo test --test rt_list_take_drop_in_place -- --test-threads=1 --nocapture`
→ `5 passed`:
- `wideTake: 8×5000 = 0.020 s CPU; 1×40000 = 0.047 s CPU`
- `wideDropAll: 8×5000 = 0.019 s CPU; 1×40000 = 0.038 s CPU`
- guards: `wideAppend` ×1.70, `wideDrop` ×1.29, `intTake` ×1.24, `intDropAll` ×1.28
- value and leak tests pass unchanged

Other checks:
- `cargo test --bin mfb compact_in_place` → 6 passed.
- `cargo test --bin mfb self_update` → 11 passed (the arm matrix).
- The `rt_inplace_failure_atomic`, `rt_recursive_value_collection_drops`,
  `rt_inplace_field_{loop,mixed,nested}`, `rt_inplace_global_record`,
  `rt_for_each_self_update`, `rt_global_self_update` and `inplace_self_update_census`
  suites all pass.
- **Not verified here:** `rt_inplace_self_update`, `rt_res_state_inplace_mutation` and
  `rt_lambda_capture_self_update`. At load average 130–207 the
  `rt_inplace_self_update` matrix was still building cases (about 9,940 of them) after
  2h50m. I killed it so the box was not held any longer, and the two suites queued
  after it never ran. All three are left to the coordinator's full suite.

Commit: —

### Phase 3 — full validation

- [x] Re-run the table above at n = 8,000 / 32,000 / 100,000; every shape linear.
      Same commands as "Re-measured" (release `mfb` from this tree, `/tmp/b652/app`,
      default `-O1`). Each cell is `real / user` seconds. The box was at load average
      ≈ 207, so `user` is the reliable column.

      | shape | n=8,000 | n=32,000 | n=100,000 |
      | --- | --- | --- | --- |
      | `wideAppend` | 0.16 / 0.00 | 0.02 / 0.02 | 0.09 / 0.07 |
      | `wideTake` | 0.01 / 0.01 | 0.03 / 0.02 | 0.11 / 0.07 |
      | `wideDropAll` | 0.00 / 0.00 | 0.03 / 0.02 | 0.16 / 0.07 |
      | `wideDrop` | 0.02 / 0.01 | 0.05 / 0.04 | 0.27 / 0.12 |
      | `intTake` | 0.00 / 0.00 | 0.00 / 0.00 | 0.01 / 0.00 |
      | `intDropAll` | 0.00 / 0.00 | 0.00 / 0.00 | 0.01 / 0.00 |

      `wideTake`/`wideDropAll` now track `wideAppend`: 0.07 s user at 100,000, down
      from 42.14 / 24.47 s. The HEAD tree on this box measured `wideTake` at 0.21 s real
      for 8,000 and 7.23 s real for 32,000. (The 8,000 `wideAppend` 0.16 s real is a
      first-launch cost; user is 0.00.)
- [ ] Full suite; `.ncode` deltas confined to the compaction sites.
      Left for the coordinator. **Expected golden shift: none (prediction, from a
      grep).** The change emits new code only for a *variable-width* `take`/`drop`/`mid`
      self-update: the `compact_range_*` block before the probe, plus
      `compact_drop_outside` in the drop pass for a graph-owning element.
      - Fixed-width lists and `filter`/`distinct` are byte-identical by construction.
      - `git ls-files 'tests/**/*.mfb' | xargs perl -ne 'print if /(\w+) = collections::(take|drop|mid)\(\1\s*,/'`
        finds no self-update fixture.
      - The `RETURN collections::mid(…)` last-use forms in library and fixture sources
        (`grep -rnE "RETURN (collections::|__collections_)(take|drop|mid)\(" src/codegen/builtins tests packages`)
        are all `List OF Byte`/`Integer`, which is fixed-width.

      Any `.ncode` diff outside a variable-width take/drop/mid site is a bug-hunt
      trigger.
- [x] `cargo build`, `cargo test --bin mfb spec`.
      → `CARGO_TARGET_DIR=/tmp/b652-target cargo build --release --bin mfb` is clean.
      `cargo test --bin mfb spec` → 44 passed, and `cargo test --bin mfb citations_resolve`
      → `spec_citations_resolve ... ok`. `mfb spec memory collections`,
      `mfb spec language collections` and `mfb spec language memory-semantics` render
      with 0 leaked `[[`.

Commit: —

## Validation Plan

- Regression test(s): the Phase 1 scaling test plus the linear, value-semantics and
  leak guards.
- Runtime proof: the re-measure table above with the same commands.
- Memory proof: `--debug` report `alloc_calls = free_calls`, `live_bytes 0`,
  `double_free_skips 0` for take/drop/mid removing graph-owning elements.
- Doc sync: spec §12 / §14 guarantee, `.ai/collections.md` rule, man pages.
- Full suite: the project's acceptance gate per `.ai/testing-gates.md`.

## Open Decisions

- ~~Is the whole-list copy intended value semantics, or a defect?~~ **Decided
  2026-09-25 (owner): in-place `MUT` self-update is required, not an optimization.**
  Phase 2 writes it into the spec as a guarantee.

## Summary

plan-142-B fixed what was filed: `take`/`drop` self-updates no longer allocate or deep
copy, and the flat case is linear. The variable-width compaction still visits every
entry on every call, so a take/drop build loop over a `Node` list is still quadratic
(42 s at 100,000). The fix is a `KeepSource::Range` fast path that touches only removed
and moved entries, plus writing the owner's rule — in-place self-update is a guaranteed
cost, not an optimization — into the spec and the collections invariants.
