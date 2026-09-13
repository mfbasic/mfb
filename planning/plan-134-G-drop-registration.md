# plan-134-G: Free recursive values where their owners end

Last updated: 2026-09-13
Effort: large (3h–1d)
Depends on: plan-134-F

Register plan-134-F's drop at every place an owner of a recursive value ends: scope exit, the
end of a statement for an unbound temporary, overwrite by assignment (local and global), a
moved source's deactivated cleanup, a closure environment, and the owned copy `collections::get`
returns. Collection **element** frees are letter H. After this letter bug-536 Shape C's two
repros run flat.

Behavioural outcome: **`c_union_rss` and `c_record_rss` (plan-134-A §2.2) peak at the same RSS
at 400 000 and 800 000 iterations** (today 52.7 → 104.3 MB and 105.1 → 209.1 MB), and every
value test from letters B–F still prints the same output.

References: plan-134-A; plan-134-F; `mfb spec memory arenas` "Scope-Drop Frees";
`src/codegen/engine/builder/mod.rs::OwnedValueCleanup`;
`src/codegen/cleanup/owned/builder_owned_cleanup.rs::emit_owned_value_drop`;
`tests/runtime/rt_scope_drop_leaks.rs` (`assert_flat`, `peak_rss`).

Prerequisites: see plan-134-A; plan-134-F complete (`ls planning/completed/plan-134-F-*`).

## 1. Goal

- New shape-C cases in `tests/runtime/rt_scope_drop_leaks.rs` pass via `assert_flat` (growth
  < 8 MiB between the two counts): union bind, record bind, reassignment, global overwrite,
  unbound temp (`len(json::stringify(json::parse(t)))`-style), closure capture, `get` result.
- The `--debug` churn loop of every B–F test shows `arena.double_free_skips` = 0.

### Non-goals

- Element frees inside collections (letter H): a `List OF Node`'s drop still frees only the
  list block until H, so list-heavy decoders are not yet flat.
- No change to flat-value cleanup codegen (byte-identical for modules without a recursive type).

## 2. Current State

- `NirOp::Bind` registers `ActiveCleanup::OwnedValue` when `owns_freeable_value` (requires
  `is_freeable_flat_value`); cleanups drain at scope end (`builder_control.rs`) and at exits
  (`builder_exits.rs`, including `emit_cleanup_branch_to_depth`).
- `OwnedValueCleanup` fields: `type_`, `stack_offset`, `closure_captures`, `capacity_slot`,
  `loop_alias_slot`, `result_wrapper`. `plan_returned_move` and `lower_returned_value` find
  ownership by `ActiveCleanup::OwnedValue` at a `stack_offset` — bug-571's comment warns against
  a separate variant.
- `pending_temp_is_freeable` requires `is_freeable_flat_value && !value_needs_owning_copy &&
  !value_is_runtime_managed`; `drop_pending_temps_to` / `clear_pending_temps_to`.
- `Assign` frees the old value via `emit_owned_value_drop` unless `by_ref`, not freeable-flat, a
  live `FOR EACH` iterable, or a record-field iterable base; `StoreGlobal` likewise.
- The closure drop frees a `Local` capture only when freeable-flat (`builder_owned_cleanup.rs`).
- **Non-owners that must stay unfreed** (read in plan-134-A's census): plan-86 E borrow-`get`
  (only freeable-flat non-`String` elements qualify, so a recursive element is never borrowed —
  VERIFY), `MATCH` scrutinee (`lower_value`, no cleanup), `UnionExtract` (`aliases_union_variant`),
  `by_ref` / `by_ref_capture_slot`, `FOR EACH` element (alias; `for_each_iterable_locals`),
  parameters (no `OwnedValue` at their slot).

## 3. Design

- **One ownership predicate.** `CodeBuilder::owns_graph(type_)` = `needs_graph_copy(type_)`
  (plan-134-D). Everywhere a gate reads `is_freeable_flat_value` to decide *ownership*
  (`owns_freeable_value`, `pending_temp_is_freeable`, the `Assign`/`StoreGlobal` old-value free,
  the closure capture free), OR in `owns_graph` — each of the 32 references (plan-134-A §2.1)
  is classified in Phase 1 as "ownership gate" or "flat-layout question", and only ownership
  gates change. The classification is a table in §2, total (no reference unclassified).
- **Drop dispatch.** `emit_owned_value_drop`: when `owns_graph(type_)`, call
  `_mfb_rt_graph_drop(kind, slot)` and zero the slot (bug-440's null-guard pattern). No new
  `ActiveCleanup` variant (bug-571).
- **Moves deactivate.** A plan-134-C move site whose source local has an `OwnedValue` cleanup
  deactivates it for the rest of the scope, exactly as `plan_returned_move` does for a return
  (remove + restore on the moved path). Without this, a moved graph is freed twice.
- **`MATCH` on a fresh temporary.** A `MATCH` scrutinee that is a fresh call result becomes a
  pending temp freed at statement end, while the arms bind `UnionExtract` aliases into it. The
  temp must stay alive through the whole `MATCH` — the free is attached after the `END MATCH`,
  not at the scrutinee's statement boundary. Phase 1 verifies where today's flat-union MATCH
  frees it and mirrors it.
- **`get` results.** bug-538's owned copy is now registered like any fresh value (it flows
  through the `Bind` arm), so it is freed; no special case.

Risk: this is where a missed non-owner becomes a double free or use-after-free. Every
classification is a table row with the code read, and the churn loops run with the arena's
entropy fill (freed chunks scrubbed) so a use-after-free reads garbage rather than stale data.

## Phases

### Phase 1 — classification and RED

- [ ] Classify all 32 `is_freeable_flat_value` references (plan-134-A §2.1 command) as
      ownership gate / layout question, with file::symbol; add the table to §2.
- [ ] Verify each non-owner in §2 by reading its code path for a recursive type (borrow-`get`
      eligibility, `MATCH` temp lifetime, `UnionExtract`, `by_ref`, `FOR EACH`, params); record
      each verdict.
- [ ] Add the shape-C cases of §1 to `tests/runtime/rt_scope_drop_leaks.rs` using `assert_flat`;
      confirm they fail today.

Acceptance: the table is total; the new leak cases fail on main.
  Check: `cargo test --release --test rt_scope_drop_leaks -- recursive` → the new cases failed
  (est. 4 min).
Commit: —

### Phase 2 — registration

- [ ] `owns_graph`; change each ownership gate from the table.
- [ ] `emit_owned_value_drop` dispatch to `_mfb_rt_graph_drop`.
- [ ] Move-site deactivation in the store lowering (plan-134-C/D sites).
- [ ] `MATCH` fresh-temp lifetime as verified.
- [ ] Tests: Phase 1 leak cases pass; every B–F runtime test passes; add a `--debug` churn case to
      `rt_recursive_value_copies.rs` asserting `double_free_skips = 0` and unchanged output over
      20 000 iterations of every store shape.

Acceptance: the shape-C repros are flat and no value or churn test changes.
  Check: `cargo test --release --test rt_scope_drop_leaks --test rt_recursive_value_copies
  --test rt_recursive_value_construction_copies --test rt_recursive_value_drop_symmetry
  --test rt_recursive_value_copy_depth --test rt_error_value_copies` → passed (est. 12 min:
  `rt_scope_drop_leaks` is 118 RSS cases and is the only suite that sees a leak or a freed-flat
  regression).
Commit: —

### Phase 3 — measurements and goldens

- [ ] `tools/recursive-value-bench/run.sh target/release/mfb c_union_rss c_record_rss` → flat;
      record in plan-134-A §2.1's "after" column.
- [ ] Artifact gate; expected diffs: modules with a recursive type (new drop calls at scope
      exits, temps, assignments). Regenerate.
- [ ] Run the copy and leak tests' programs cross-built on box 2223 (Linux) and, via a `.cmd`
      wrapper, on box 2230 (Windows has no harness: `.ai/remote_systems.md`).

Acceptance: repros flat on macOS; the programs print the same output on Linux and Windows;
diffs confined.
  Check: bench (est. 2 min); gate (est. 15 min); box runs (est. 10 min).
Commit: —

## Validation Plan

- Tests: the new `rt_scope_drop_leaks` cases; churn assertions.
- Runtime proof: bench flat; box 2223 and 2230 runs.
- Doc sync: `mfb spec memory arenas` "Scope-Drop Frees" (no longer "no per-type recursive drop
  glue"); `.ai/codegen-invariants.md` (the "no owning copy at a bind and no drop anywhere"
  passage for recursive types becomes history); bug-536 doc Phase 3 progress.
- Final gate: plan-134-H.

## Open Decisions

- None beyond plan-134-A's.

## Corrections

(Filled in during execution.)

## Summary

The highest-risk letter: it turns ownership into frees. It is safe only because B–E made every
owner distinct and F proved the drop exact; the total classification table and the churn loops
are its own guard.
