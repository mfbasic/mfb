# bug-652: an in-place `take`/`drop` over a variable-width list walks the whole list, so a self-reassigning build loop is still O(n²)

Last updated: 2026-09-25
Effort: medium (1h–2h)
Severity: MEDIUM (performance; output is correct — but see "In-place is required")
Class: Performance / scalability

Status: Open — **rescoped 2026-09-25.** The original defect (no in-place arm, a fresh
copy plus a per-element graph deep copy) was fixed by plan-142-B (`a70924e35`,
2026-09-21) and the flat `List OF Integer` case is now linear. What remains is the
variable-width compaction's cost: it is O(count) per call even when it removes nothing.
Regression Test: none yet — see Phase 1. Repro: `bugs/repro/bug-652-take-drop-variable-width-compaction.mfb`

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

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Land a scaling test for `wideTake` / `wideDropAll` (variable-width, type reaching a
      cycle) that times n and 4n and fails when the ratio is super-linear, per the
      perf-golden conventions in `.ai/testing-gates.md`. Confirm it fails at HEAD.
- [ ] Add a guard that `intTake` / `intDropAll` / `wideAppend` / `wideDrop` stay linear.
- [ ] Add a value-semantics test: `LET b = collections::take(a, k)` leaves `a` intact,
      flat and cyclic element types.
- [ ] Add a leak test: a `take` / `drop` / `mid` that removes graph-owning elements
      ends `--debug` with `alloc_calls = free_calls`, `live_bytes 0`.
- [ ] Audit every `SelfUpdate::Arm` row in `SELF_UPDATE_TABLE` for cost proportional to
      the call's work, not to the collection's size. Record the table here; fix or file
      each offender.

Acceptance: the scaling test fails at HEAD; the guards pass at HEAD.
Commit: —

### Phase 2 — the fix

- [ ] `KeepSource::Range` fast path in `lower_list_compact_in_place` /
      `emit_compact_entries` (Fix Design).
- [ ] Spec: `12_collections.md` — replace "Destructive update is an optimization only
      for the assignment-back-to-the-same-`MUT` pattern" with the guarantee: a `MUT`
      self-update is performed in place, and its cost is proportional to the elements it
      adds, removes or moves, at every optimization level. State where the guarantee does
      not apply (the old value is still read by another live binding, so a copy is
      required). `14_memory-semantics.md` (opening paragraph) "may choose … destructive update" —
      align with the guarantee. Cite the arms (`[[path:Symbol]]`, `.ai/specifications.md`).
- [ ] `.ai/collections.md` — replace "**Declining is always correct**" with: declining
      is correct only where the spec says a copy is required; an arm that declines
      anywhere else, or accepts but costs O(size), is a bug.
- [ ] `mfb man collections` — check `take`/`drop`/`mid`/`append` pages describe the
      in-place self-update in developer terms (`.ai/man-content.md`: "mutate", "copy").

Acceptance: Phase 1's scaling test passes; value, leak and linear guards unchanged.
Commit: —

### Phase 3 — full validation

- [ ] Re-run the table above at n = 8,000 / 32,000 / 100,000; every shape linear.
- [ ] Full suite; `.ncode` deltas confined to the compaction sites.
- [ ] `cargo build`, `cargo test --bin mfb spec`.

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
