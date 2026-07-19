# bug-307: higher-order collection members leak a freshly-materialized String/composite per iteration

Last updated: 2026-07-17
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Memory-safety (leak)

Status: Fixed
Regression Test: tests/rt-behavior/collections/hof-string-item-lifetime-rt (correctness) + out-of-tree RSS measurement (the leak itself)

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

## Resolution

The leak was **measured**, not inferred. Iterating an unchanged 500-element
`List OF String` with `forEach`:

| passes | RSS before | RSS after |
| --- | --- | --- |
| 1 | 69.5 MB | 69.5 MB |
| 10 | 77.9 MB | 69.5 MB |
| 40 | 111.3 MB | 69.5 MB |
| 100 | — | 69.5 MB |

The data is identical in every run; only the number of passes over it varies, so
growth of ~1.07 MB per pass is unambiguously per-iteration allocation that is never
reclaimed. After the fix RSS is flat out to 100 passes.

`free_collection_loop_item` releases the block, applied to `forEach`, `transform` and
`filter`. It is a no-op for every other element type: only the `String` arm of
`emit_load_collection_payload` allocates — the rest hand back a scalar or a pointer
into the packed data region.

### `reduce` is deliberately excluded

Its reducer may return the loop item itself as the new accumulator
(`reduce(xs, "", FUNC(acc, x) RETURN x)`), so the block can still be live after the
callback returns. Freeing it would trade a leak for a use-after-free, which is
strictly worse. That case is exercised by the fixture so the exclusion stays
deliberate rather than becoming an oversight. Closing it needs the accumulator to
take an owning copy — the same aliasing question the existing comment there already
records.

### Two placement traps, both hit and both real

1. **Freeing by register segfaulted.** The first attempt passed the item register.
   `arena_free` is a *call*, and a call destroys every caller-saved register — in
   `transform` it wiped `RESULT_VALUE_REGISTER` before the callback's result was
   stored. The helper now takes a **stack slot**, and the pointer is stashed before
   the callback. (Same hazard as [[arena-alloc-clobbers-x14-x15]].)
2. **`filter` must free after the append, not before.** Its item is appended to the
   output when the predicate keeps it. That is safe only because
   `emit_copy_payload_to_collection` *copies* the String's bytes into the output's
   packed data region rather than storing the pointer — checked in the source before
   relying on it. The free therefore sits below `skip_label`, covering both the keep
   and skip paths.

The fixture pins correctness rather than the RSS number: every member must still
return the right values, which is exactly what the first attempt broke. It also
iterates 50 further times, since a freed-then-reused block would surface as changed
output.

Full `cargo test` green; artifact gate 0 diffs; acceptance 1010/1010.
