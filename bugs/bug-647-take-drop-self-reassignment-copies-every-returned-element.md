# bug-647: self-reassigning a list through `collections::take`/`drop` copies every returned element, so a build loop is O(n²)

Last updated: 2026-09-15
Effort: large (3h–1d)
Severity: MEDIUM (performance; output is correct)
Class: Performance / scalability

Status: Open
Regression Test: none yet — see Phase 1

`pending = collections::take(pending, len(pending))` and
`pending = collections::drop(pending, 0)` allocate a fresh list and copy every returned
element, even though the store makes the source dead. `collections::append` on the
identical local hits the documented in-place arm and is O(1) amortized, so a loop that
rewrites a shared pending list one element at a time is O(n²) while the same loop built
with `append` alone is linear. At n=8000 with a cyclic element type that is 0.02 s vs
~22 s; at 100,000 nodes a take/drop builder exceeds a 30 s alarm.

The output is correct. This is a scalability defect, not a miscompile.

**The single correct behavior a fix produces:** `x = collections::take(x, len(x))` and
`x = collections::drop(x, 0)` on an owned, uniquely-referenced `MUT` local cost O(1) in
the number of elements retained, so the `wideTake` / `wideDropAll` shapes below land in
the same order of magnitude as `wideAppend` (0.02 s at n=8000) instead of ~22 s, and the
"wide" builder at 100,000 nodes completes rather than hitting the alarm.

**Not blocking plan-138.** Found while executing plan-138-A Phase 1 (`packages/xml`).
That plan uses a recursive-descent builder whose per-element children list only ever
receives `append`, which is linear (0.11–0.13 s at 100,000 nodes), so plan-138 has a
working path and does not wait on this fix.

References:

- `.ai/collections.md` — "In-place MUT append" (`try_inplace_append_assign`), and the
  in-place inventory of plan-121-A ("Declining is always correct").
- `planning/completed/plan-134-C-last-use-move-analysis.md` — the last-use move analysis
  whose answer is what `take`/`drop` never consult.
- `planning/completed/plan-134-D-copy-insertion-at-owning-stores.md` — `needs_graph_copy`,
  the class that multiplies the per-element cost.
- `src/docs/spec/language/12_collections.md` — `take`/`drop` are source generics; the
  destructive-update contract is stated only for assignment-back-to-the-same-`MUT`.

## Failing Reproduction

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

## Root Cause

Two independent contributors. Both are cited below; the second is what makes the cyclic
element type 35x worse than the flat one.

**1. `take`/`drop` are whole-range copies with no in-place or move arm.**

`collections::take` and `collections::drop` are source generics
(`src/docs/spec/language/12_collections.md`), whose bodies are one line each:
`__collections_take` (`src/codegen/builtins/collections/func_take.rs:90`) is
`RETURN __collections_slice(value, 0, count)` and `__collections_drop`
(`src/codegen/builtins/collections/func_drop.rs:81`) is
`RETURN __collections_slice(value, count, len(value))`.

`__collections_slice` is intercepted before any FUNC call is emitted — the `NirValue::Call`
arm at `src/codegen/engine/value/builder_values.rs:1939` calls `try_inline_slice_op` on the
monomorphized `#collections_slice$T` target — and lowered natively by `try_inline_slice_op`
→ `lower_list_slice_range` (`src/codegen/builtins/collections/gen_slice.rs:27`, `:56`),
which allocates a **fresh** collection and copies the entries `[start, stop)` into it —
a block copy for a contiguous fixed-width ("kind 2") payload, otherwise the per-element
loop at `gen_slice.rs:311-340`. The work is proportional to the number of elements in the
returned range, which is exactly what the `drop-none` vs `drop-all` contrast measures:
`drop(pending, len(pending))` copies nothing, `drop(pending, 0)` copies everything.

Nothing recognises the self-assignment. The in-place dispatch chain in the `NirOp::Assign`
arm at `src/codegen/engine/control/builder_control.rs:1100` (chain spanning `:1131-1228`)
runs
`try_inplace_append_assign`, `try_inplace_bulk_append_assign`, `try_inplace_set_add_assign`,
`try_inplace_set_assign`, `try_inplace_remove_key_assign`, `try_inplace_prepend_assign`,
`try_inplace_remove_at_assign`, `try_inplace_insert_assign`,
`try_inplace_set_remove_assign`, `try_inplace_concat_assign` and the
`try_inplace_record_field_*` family (definitions in
`src/codegen/collection/assign/builder_inplace_assign.rs`) — **there is no slice, take or
drop arm**, so `x = collections::take(x, …)` falls through to the general copying
reassignment at `builder_control.rs:1234` (`lower_value_owned` plus a free of the old
block). `append` on the same local is fast for exactly the reason `take` is slow:
`try_inplace_append_assign` (`builder_inplace_assign.rs:23`) routes to
`lower_list_append_in_place` (`src/codegen/collection/list/list_mutate.rs:426`).

The move analysis that *should* apply never sees this site. plan-134-C's
`collect_last_use_moves` (`src/codegen/engine/analysis/last_use.rs:912`, queried by
`is_last_use` at `:97`) is consulted only through `store_is_last_use`
(`src/codegen/engine/value/builder_values.rs:1319`), and only at **stores** —
`builder_exits.rs:291` and `:327`, `builder_values.rs:816`, `:833`, `:1415`. The `x` in
`collections::take(x, len(x))` is a **call argument**, not a store, so the analysis is
never asked whether this is `x`'s last read. It is: the result is immediately assigned
back over `x`, making the source dead at the store.

Command behind the inventory and consult-site claims:
`grep -rn "try_inplace_[a-z_]*" src/codegen/engine/control/builder_control.rs` and
`grep -rn "store_is_last_use" src/`.

**2. The cyclic element type additionally deep-copies each element's graph.**

After the range copy, `try_inline_slice_op` calls `own_collection_payload_edges`
(`gen_slice.rs:47-48` → `src/codegen/memory/arena/builder_arena_transfer.rs:1649`), commented
"plan-134-E: the slice owns its elements' graphs". That helper is gated on
`needs_graph_copy` (`builder_arena_transfer.rs:1654`) and then runs
`fix_collection_transfer_payloads` → `fix_collection_transfer_payload`, a per-entry loop
that classifies each element with `graph_copy_edge_kind` and pushes it through
`emit_graph_copy_push` into the non-recursive work-stack walker `_mfb_rt_graph_copy`
(`src/codegen/memory/arena/graph_copy.rs:57`). That is a full graph deep copy **per
element**, on top of the byte copy.

`needs_graph_copy` (`src/codegen/engine/value/builder_values.rs:1301`) is
`!is_freeable_flat_value(type_) && type_reaches_cycle(…) && !type_contains_resource(…)`,
where `type_reaches_cycle`
(`src/codegen/collection/layout/builder_collection_layout.rs:3252`) is a BFS over record
fields, union variants and list/map components.
`Node` satisfies it (`Element.children AS List OF Node` reaches a cycle); `Integer` does
not, so the `List OF Integer` slice is a plain block copy and skips this pass entirely.
That is precisely the measured split: 21.9 s vs 0.62 s for the same loop shape and n.

So the flat case is quadratic from contributor 1 alone, and the cyclic case pays
contributor 1 x contributor 2.

## Goal

- `x = collections::take(x, len(x))` and `x = collections::drop(x, 0)` on an owned,
  uniquely-referenced `MUT` local do not copy the retained elements: `wideTake` and
  `wideDropAll` at n=8000 land within the same order of magnitude as `wideAppend`.
- The "wide" take/drop builder is linear: 4x the nodes costs ~4x the time, and 100,000
  nodes completes inside the 30 s alarm.
- `intTake` / `intDropAll` improve too (the flat case is quadratic for contributor 1
  independently of the graph copy).

### Non-goals (must NOT change)

- **Value semantics.** `collections::take(a, k)` where the result is bound to a *different*
  name, returned, or passed on must keep copying — `a` must be unaffected. Only the
  self-assignment `x = take(x, …)` may mutate in place, exactly as `append` does today.
- **The `drop-none` fast path stays free.** A fix must not make the currently-free
  shapes (`wideAppend`, `wideDrop`) slower.
- **Do not weaken the repro to make it pass.** Re-timing a smaller n, or swapping the
  cyclic `Node` union for a flat element type, hides the effect rather than fixing it —
  the cyclic element type is the whole point of contributor 2.
- **Clamping semantics.** `take(count)` = `slice(0, count)` and `drop(count)` =
  `slice(count, len)`, with `start`/`stop` clamped into range, must be preserved
  bit-for-bit; `func_take.rs` warns explicitly not to read the dead MFBASIC
  `__collections_slice` body as the spec of that clamping.
- Correctness of `chunks` / `window`, the other `__collections_slice` callers, which
  legitimately need a fresh copy per piece.

## Blast Radius

Found by `grep -rn "__collections_slice" src/` and the in-place inventory grep above.

- `collections::take` (`func_take.rs:90`) — fixed by this bug.
- `collections::drop` (`func_drop.rs:81`) — fixed by this bug; same helper, same hazard.
- `__collections_slice` / `lower_list_slice_range` (`gen_slice.rs:56`) — the shared
  mechanism; where a fix most likely lands.
- `collections::chunks` (`func_chunks.rs:422`) and `collections::window`
  (`func_window.rs:442`) — both call `__collections_slice`, but each piece is a *new*
  list handed to the caller, never a self-assignment, so no last-read arm applies.
  Unaffected by design; they must keep copying.
- The `&` concat operator — already has `try_inplace_concat_assign`
  (`builder_inplace_assign.rs:1714`, dispatched at `builder_control.rs:1180`), so the
  self-assigning concat shape is covered today. Note this is the **operator**, not a
  `collections::` member: there is no public `collections::concat`, `collections::slice`
  or `collections::reverse`. It is the closest precedent a take/drop arm should follow.
- Other whole-collection-returning members with **no** in-place arm — latent, same
  "returns a fresh list, no in-place arm" shape, **out of scope**: none was measured here
  and each has different aliasing conditions. From
  `src/codegen/builtins/collections/mod.rs:register` plus each `func_*.rs`: `mid`,
  `replace`, `filter`, `transform`, `sort`, `sortBy`, `distinct`, `flatten`, `chunks`,
  `window`, `zip`, `groupBy`, `mapValues`, `merge`, `partition`, the set-algebra family
  (`toSet`, `union`, `intersection`, `difference`, `symmetricDifference`) and the
  projections `keys`, `values`, `toList`. `filter` and `transform` are worth calling out:
  each already uses `lower_list_append_in_place` into its own *private* accumulator, yet
  still returns a brand-new list, so `x = collections::filter(x, f)` copies everything
  exactly as `take` does. Worth a follow-up audit once the take/drop arm sets the pattern.

## Fix Design

The shape that matches existing precedent is a new in-place/move arm in the
`builder_control.rs:1131-1223` chain — a `try_inplace_slice_assign` recognising
`x = collections::take(x, k)` / `x = collections::drop(x, k)` on a non-`by_ref`, owned
`MUT` local, lowering to a header rewrite (`take`: bump `count`/`dataLength` down, dropping
the tail; `drop`: memmove the retained range down to the data base and adjust the header)
plus element drops for the discarded range. That reuses the `InPlaceDest` /
`inplace_dest.rs` gates the other arms already share, and inherits plan-121-A's
"declining is always correct" rule — an arm that cannot prove unique ownership falls
through to today's copy.

The correctness risk concentrates in the **discarded** elements, not the retained ones:
`take` must drop what it truncates and `drop` must drop what it skips, or the fix trades
a performance bug for a leak. plan-134-H's `emit_drop_list_element` is the existing
instrument for that.

Rejected: making `own_collection_payload_edges` cheaper. It is load-bearing for
plan-134-E correctness (the slice really does own its elements' graphs when it is a
genuine copy), and it would not touch the flat `List OF Integer` case, which is quadratic
without it.

Also rejected: extending `store_is_last_use` to call arguments in general. That is a much
larger semantic change reaching every builtin, and plan-134-C deliberately scoped the
analysis to stores.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Land a perf regression case for the `wideTake` / `wideDropAll` shapes at a node
      count whose linear-vs-quadratic gap is unambiguous, per the project's perf-golden
      conventions (`.ai/testing-gates.md`). Confirm it fails at HEAD for the documented
      reason.
- [ ] Add a value-semantics test pinning the non-goal: `LET b = collections::take(a, k)`
      must leave `a` intact, for both a flat and a cyclic element type.
- [ ] Settle the Open Decision below before designing the arm.

Acceptance: the perf case fails at HEAD; the value-semantics cases pass at HEAD and are
green guards for Phase 2.
Commit: —

### Phase 2 — the fix

- [ ] Add the take/drop in-place arm and wire it into the `builder_control.rs` chain.
- [ ] Drop the discarded elements (`emit_drop_list_element`); prove no leak with a
      `live_bytes` soak in the style of bug-644/645.

Acceptance: Phase 1's perf case passes; value semantics and clamping unchanged.
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [ ] Re-run the n=8000 table and the scaling table; both linear.
- [ ] Full suite; confirm `.ncode` deltas are confined to the take/drop sites.

Commit: —

## Validation Plan

- Regression test(s): the Phase 1 perf case plus the value-semantics guards.
- Runtime proof: the n=8000 table above re-measured with the same `time.sh` command, and
  the "wide" builder completing at 100,000 nodes.
- Memory proof: a `live_bytes` soak proving the discarded elements are dropped exactly
  once, `double_free_skips 0`.
- Doc sync: `.ai/collections.md`'s in-place inventory gains the new arm;
  `src/docs/spec/language/12_collections.md` states the destructive-update contract for
  `append` only and would need the same sentence for take/drop.
- Full suite: the project's acceptance gate per `.ai/testing-gates.md`.

## Open Decisions

- **Is the whole-list copy intended value semantics for `take`/`drop`, or an outright
  defect?** I could not settle this from evidence, and the two readings imply different
  fixes. The spec is genuinely silent on the point: `src/docs/spec/language/12_collections.md`
  promises destructive update only for "the result of an update function assigned back to
  the same `MUT` binding, such as `pts = collections::append(pts, v)`", and names `append`
  as its only example, while calling all update helpers "semantically pure functions" for
  which "destructive update is an optimization only". Under the first reading the copy is
  *correct* and this bug is the **missing last-read move arm plus the graph-copy
  multiplier** — a missing optimization, fixed as designed above. Under the second, a
  self-assignment through `take`/`drop` was always meant to be in-place like `append`, and
  the omission is a defect in the in-place inventory. Either way the user-visible fix is
  the same arm; what differs is whether the spec sentence needs widening (reading 1) or
  merely clarifying (reading 2). **Owner decision** — it determines whether Phase 2 also
  edits §12.

## Summary

The engineering risk is entirely in the discarded elements: retaining a range in place is
mechanical, but `take` truncating and `drop` shifting must drop exactly what they remove
or the fix becomes a leak. The copy path itself, the clamping contract, and the
`chunks`/`window` callers stay untouched. plan-138 is not waiting on this.
