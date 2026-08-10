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
- [x] **E1 — aliasing borrow for a read-only `get` consumed by MATCH. LANDED: dispatch union 160.9 → 43.2 ms
  (~3.7×), checksum `212666511` unchanged.** A new `collect_borrow_get_locals` classifier
  (`function_lowering.rs`, on the exhaustive `NirVisitor` seam per plan-77 M6, populated for `lower_function`)
  marks the bindings that may alias, gating on soundness: `e = get(L, i)` where `L` is a plain Local that is
  IMMUTABLE (bound ≤1, never `Assign`ed, not address-taken — a reassign frees `L`'s block while `e` points into
  it), and every read of `e` is borrow-transparent. **KEY: the IR desugars `MATCH e` into
  `$matchN = e; MATCH $matchN`, and the MATCH reads `$matchN` once as its scrutinee PLUS once per case via
  `UnionExtract` (the variant bindings) — so the classifier follows the chain: `e`'s only read is the match-temp
  copy `$matchN = e`, and `$matchN` is read only by the MATCH (scrutinee + `UnionExtract`s). BOTH `e` and
  `$matchN` borrow, so the container element flows into MATCH with ZERO copies.** Codegen: `borrow_get_result`
  flag makes `materialize_owned_element` return the alias (no `copy_flat_block`); the Bind arm excludes borrows
  from `owns_freeable_value` (no copy, no scope-drop free) — gated on the SAME set + a freeable-flat-non-String
  element type (a String `get` returns an OWNED fresh block, so it keeps its copy+free). **THE BUG THAT COST
  THE MOST: `lower_value` registers the get result as a plan-25 PENDING TEMP (statement-scope free,
  `builder_values.rs:17/30`); the borrow path (plain `lower_value`, no `claim_pending_temp`) left it registered
  → the statement-scope free `arena_free`d the alias INTO the container → garbage + free-list corruption. Fix:
  `register_pending_temp` early-returns when `borrow_get_result` is set (the alias is not a fresh block).**
  Verified: positive byte-identical to the copy path across all variants; NEGATIVE cases (e RETURNED / container
  reassigned) correctly fall back to the copy (classifier excludes them); 100k-iteration stress with
  interleaved allocations shows no UAF/corruption. Fixture: `get-borrow-match-rt`. Commit: `de90b2841`.
- [x] ~~**E2 (fuse `MATCH collections::get(...)`)**~~ — **moot: subsumed by E1.** The real source pattern is
  `LET e = get(...); MATCH e`, which the IR desugars to `$matchN = e; MATCH $matchN` — E1's chain-aware
  classifier already borrows both `e` and `$matchN`, so the container element reaches MATCH with zero copies
  without a separate directly-fused `MATCH get(...)` peephole. Still P2 (43.2 vs c-O0+10 ≈ 25) — the residual
  is the interpreted MATCH/recursive-tree-eval overhead vs C's switch, not the copy (which E1 eliminated).

## Acceptance
dispatch checksum unchanged + `scripts/artifact-gate.sh`.

## Note
`materialize_owned_element` excludes `"String"`, so E applies to `dispatch union` (Expr-union is
freeable-flat) but NOT to the String list HOFs (those pay the interpreted-body cost, addressed in
[plan-86-A](plan-86-A-string-native-lowering.md)) — a plan-64 conflation this round corrected.
