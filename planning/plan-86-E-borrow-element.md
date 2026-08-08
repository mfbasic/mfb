# plan-86-E — borrow read-only collection element

Sub-plan **E** of [plan-86](plan-86-benchmark-perf.md). Open.

**Covers (1 P2):** dispatch union (160.6).

## Root cause
`benchmark/mfb/src/dispatch.mfb:44` binds `LET e = collections::get(nodes, i)` then `MATCH e` read-only.
`get` lowers through `materialize_owned_element` (`builder_collection_queries.rs:10-23`) → `copy_flat_block`:
a fresh arena copy per element. The element is an `Expr` union — freeable-flat and ≠`"String"`, so it hits
the copy (~4M copies/rep). MATCH's own variant binding already aliases the inline block without copying, so
the copy is pure overhead.

## Fixes
- [ ] **E1** — return an aliasing borrow (pointer into the container's inline element) for a `get` whose
  result is consumed read-only within the statement (MATCH scrutinee, field read, predicate arg); copy only
  when it is stored/returned/mutated. Escape analysis, ride `[[nir-visitor-exhaustive-escape-analysis]]`.
- [ ] **E2** — fuse `MATCH collections::get(list, i)` into a direct read of the inline element's tag+payload
  (no intermediate `e` block).

## Acceptance
dispatch checksum unchanged + `scripts/artifact-gate.sh`.

## Note
`materialize_owned_element` excludes `"String"`, so E applies to `dispatch union` (Expr-union is
freeable-flat) but NOT to the String list HOFs (those pay the interpreted-body cost, addressed in
[plan-86-A](plan-86-A-string-native-lowering.md)) — a plan-64 conflation this round corrected.
