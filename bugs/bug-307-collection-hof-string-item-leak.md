# bug-307: higher-order collection members leak a freshly-materialized String/composite per iteration

Last updated: 2026-07-17
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Memory-safety (leak)

Status: Open
Regression Test: tests/rt-behavior (new) — iterating a large `List OF String` with forEach/transform/filter/reduce does not grow arena RSS per element

`collections::forEach`/`transform`/`filter`/`reduce` over a `List OF String` (or a
list of inline-`String` records) allocate one owned String block per element that is
never reclaimed. `load_collection_loop_item` → `emit_load_collection_payload`'s
`String` arm routes to `emit_materialize_string_from_bytes`, which `arena_alloc`s a
*fresh* owned block and returns it; that register is moved into `argument_register(0)`
for the callback and then never referenced again and never freed. Unlike
`collections::get` (whose fresh String flows into a `ValueResult` the assignment site
scope-drops), these loop items are raw register strings with no pending-temp
registration and no `arena_free`. The callback receives a by-value borrow and does
not own/free it. So arena RSS grows ~O(total bytes iterated) with no reclamation.

The single correct behavior a fix produces: iterating a collection of Strings (or
composites) reclaims (or never separately allocates) each element block, so
per-iteration memory does not grow unboundedly.

References:

- `bugs/completed-bugs/bug-01-collection-value-leaks.md`,
  `bug-142` (foreach in-place append UAF), the `collection-memory-mgmt` memory note.
- Found during goal-06 review of `src/target/shared/code/builder_collection_queries.rs`.

## Failing Reproduction

```
' forEach / transform / filter / reduce over a List OF String with N elements
' allocates N String blocks that stay live for the process.
```

- Observed: arena RSS grows ~O(total bytes iterated); `filter` leaks even on the
  "keep" path (the append copies bytes rather than adopting the block).
- Expected: bounded per-iteration memory.

(Established by code reading; measuring the leak needs arena instrumentation.
Primitive-element lists are unaffected — the scalar arms allocate nothing.)

## Root Cause

`src/target/shared/code/builder_collection_queries.rs:2038`
(`load_collection_loop_item`) feeding `lower_collection_for_each_call:1537`,
`lower_collection_transform_call:1637`, `lower_collection_filter_call:1736`,
`lower_collection_reduce_call:1826`, via
`src/target/shared/code/builder_collection_layout.rs:1867`
(`emit_load_collection_payload`, `"String"` arm): a fresh owned String block is
materialized per element with no owner and no free site.

## Goal

- The higher-order members either pass a *borrow* into the collection's data region
  for String/composite elements (the callback only reads), or free the item block at
  the bottom of each loop iteration.

### Non-goals (must NOT change)

- Primitive-element iteration (correctly allocates nothing).
- The callback's by-value borrow contract, unless ownership semantics require a copy.

## Blast Radius

- `load_collection_loop_item` and the four higher-order lowerings — fixed here.
- `emit_load_collection_payload` `String`/composite arms — the materialization site.
- Confirm `collections::get` (which correctly scope-drops) is unaffected.

## Fix Design

Preferred: a load path that returns the data-region pointer (a borrow) for
String/composite loop items, like the inline-composite arm already does — no
allocation, no free. If ownership rules require a fresh copy, free the item block at
the loop-iteration tail (spilling/reloading the callback result around the
`arena_free`, as `emit_callback_failure_exit` already does). Confirm the intended
ownership semantics first.

## Phases

### Phase 1 — failing test + audit
- [ ] rt-behavior test iterating a large `List OF String` and asserting bounded
      arena growth (or an instrumented leak check). Audit all four members + the
      filter keep-path.
### Phase 2 — the fix
- [ ] Borrow-or-free the per-element block across all four members.
### Phase 3 — validation
- [ ] Full suite green; no per-iteration growth; results unchanged.

## Validation Plan

- Regression: the iteration-growth test.
- Runtime proof: arena RSS stable across a large iteration.
- Doc sync: none.

## Summary

The higher-order collection members materialize an unowned String per element,
leaking one block per iteration. Passing a borrow (or freeing per iteration) fixes
it; the real work is confirming ownership semantics across all four members and the
filter keep-path.
