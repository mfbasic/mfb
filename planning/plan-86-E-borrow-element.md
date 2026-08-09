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
  result is consumed read-only. **TRACTABLE — scout (plan-86-A session) verdict: no general escape analysis
  needed; the borrow discipline ALREADY EXISTS.** `materialize_owned_element`
  (`builder_collection_queries.rs:10-23`) is the ONLY thing turning `get`'s aliasing borrow
  (`container_data_base + value_offset`, a live pointer into the container) into an owned `copy_flat_block`
  (the ~4M-alloc/rep cost). And `aliases_union_variant` (`builder_control.rs:199-200,320,387`) is EXACTLY the
  needed "no copy at bind, no cleanup at scope-drop" discipline for a MATCH-variant borrow. **THE LOAD-BEARING
  INVARIANT: gate BOTH the copy-skip AND the scope-drop cleanup-skip on the SAME set — a borrow that gets an
  `OwnedValue` cleanup will `arena_free` a pointer INTO the container's data region → double-free/UAF that
  surfaces as a trap on a LATER alloc** (`builder_collection_layout.rs:2249-2252`). **Edit points (conservative
  path — only when `get`'s result is bound to a LET used SOLELY as a MATCH scrutinee):**
  (1) new classifier in `function_lowering.rs` (beside `collect_value_used_locals:54`, ride the exhaustive
  `NirVisitor` seam per plan-77 M6): collect `Bind` names whose `value` is `NirValue::Call{collections get/getOr}`
  and whose only `NirValue::Local(name)` value-occurrence is the `value` field of a `NirOp::Match`, AND absent
  from `address_taken_locals` + `for_each_iterable_locals`; store as a `HashSet<String>` on `CodeBuilder`
  (`mod.rs:251`, init in the 3 ctors `function_lowering.rs:573/834/975`, populate ~`:678`);
  (2) suppress the copy: set a scoped `self.borrow_get_result` flag in the `NirOp::Bind` arm before lowering
  the value (analogous to `raw_result_discard_error`, `builder_control.rs:311-325`); `materialize_owned_element`
  early-returns `Ok(result)` when set;
  (3) suppress the cleanup + bind-copy: add the borrow classification to the `owns_freeable_value` exclusion
  (`builder_control.rs:213-217`) exactly as `aliases_union_variant` already does → `:320` no-copy branch,
  `:441` no cleanup. **Same-set gating for (2) and (3) is mandatory.**
- [ ] **E2 (cleanest form)** — for a directly-fused `MATCH collections::get(list,i)` (scrutinee is
  `NirValue::Call{get}` at `builder_control.rs:844`), ONLY edit (2) is needed — there is no owned local bound,
  so no cleanup question. MATCH already spills a pointer + reads tag/payload in place + aliases the case
  bindings (`:889-910`). Either write the benchmark as `MATCH get(...)`, or add an IR peephole inlining a
  use-once `get`-binding into the MATCH scrutinee.
- Baseline: dispatch union 160.6 ms (P2, ΔO0 +145.6). Expect a big drop (the copy is pure overhead). NOT yet
  implemented — correctness-critical (UAF if the two skips diverge); implement with a MATCH-scrutinee-borrow
  UAF stress fixture (many gets + repeated MATCH + subsequent allocs, checksum-pinned) + the full artifact-gate.

## Acceptance
dispatch checksum unchanged + `scripts/artifact-gate.sh`.

## Note
`materialize_owned_element` excludes `"String"`, so E applies to `dispatch union` (Expr-union is
freeable-flat) but NOT to the String list HOFs (those pay the interpreted-body cost, addressed in
[plan-86-A](plan-86-A-string-native-lowering.md)) — a plan-64 conflation this round corrected.
