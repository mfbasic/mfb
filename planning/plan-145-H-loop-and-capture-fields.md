# plan-145-H: `FOR EACH` over the field (S7/T7), and a captured record (S9)

Last updated: 2026-09-21
Effort: large (3h–1d)
Depends on: plan-145-G

Prerequisites: see plan-145-A.

Three gates decline the arm rows at the aliasing sites (findings §3.2 item 6):

- **`G15` (S7).** A `FOR EACH v IN r.b` whose body updates `r.b`. The fallback
  also **skips the old block's free** (`builder_control.rs`, the
  `for_each_iterable_record_fields` check in `NirOp::Assign`). So each iteration
  copies the record **and** leaks the displaced block (findings §3.5: 66 cells).
- **`G16` (T7).** The same over `h.state.b`. The replace skips the bug-644 free
  (`FREE=-` in all 55 T7 probes, §3.5).
- **`G1` (S9).** A record captured by reference in a `collections::forEach`
  lambda. The by-ref slot holds the parent's slot address.

This is the field version of plan-142-F (S7) and plan-142-G (S9), with their
designs reused:

- the loop walks a private copy of the field, made once at entry, when its body
  writes the field;
- a by-ref capture reaches the parent's record block through the reference.

## 1. Goal

- `field_expect.tsv` and `field_kinds.tsv`: every S7, T7 and S9 line matches its
  S4/T2 verdict. The S7/T7 lines for types a `FOR EACH` rejects stay
  `na:<diagnostic>`.
- The loop visits the field's entry-time elements (`append`, `removeAt`, `sort`
  and `filter` inside the loop, on both containers).
- **No leak.** After a 1,000-iteration loop at S7 and T7, the debug report shows
  `alloc_calls = free_calls` and `live_bytes 0`. Today it leaks one record or
  payload per iteration.
- S9: a record captured by reference in `forEach` and updated in the lambda
  prints the same values as the copying path. The parent sees every update.

### Non-goals

- Snapshot semantics are unchanged: the loop never sees its own writes.
- A loop whose body does not write the field keeps borrowing it.

## 2. Current State

- `lower_for_each` (`builder_control.rs:2522`) pushes `(local, field)` onto
  `for_each_iterable_record_fields` (`:2665`), and the `STATE` twin onto
  `for_each_iterable_state_fields`. `G15`/`G16` read those lists, and so do the
  rebuild's free guards. A loop over a plain local that its body writes walks an
  owned copy instead (plan-142-F).
- By-ref capture: `InPlaceDest::Ref { ref_slot, block_slot }` (plan-142-G) for a
  plain binding. A field site has no `Ref` form.

## 3. Design

**S7/T7.** Extend plan-142-F's body walk. When the iterable is `r.b` (or
`h.state.b`) and the body contains a statement that writes it, `lower_for_each`
lowers the iterable **owned** and does not push onto the tracking list. A
statement writes it when it is a `NirOp::Assign` to `r` with a `WITH` updating
`b`, or a `StateAssign` on `h` updating `b`, or a by-ref capture of `r`/`h`
inside the loop. Then `G15`/`G16` do not fire, the arms run, and because the
rebuild's free guards no longer match, the leak path is never reached. The free
on every loop exit edge (normal exit, `EXIT FOR`, `RETURN`, `TRAP` unwinding)
reuses plan-142-F's drop.

**S9.** The field site's owner may be a by-ref local. The builder builds
`Inlined { block_slot: <scratch>, write_back: WriteBack::Ref(ref_slot) }` (the
`WriteBack` enum from letter G):

- open loads the parent's record pointer through the reference;
- arms and `InlineGrow` repoint `block_slot`;
- close stores it back through the reference.

Scalar stores and fixed-size overwrites need no write-back. They write through the
loaded pointer.

Risk: the loop-exit free on every edge (a missed edge leaks, a doubled one
double-frees), and the by-ref write-back after an `InlineGrow`.

## Phases

### Phase 1: S7 and T7

- [x] `lower_for_each`: the field-write body walk, the owned iterable, and no
      tracking push.
      `owns_owner_field_iterable`: an iterable `r.f` whose body writes `r`
      (`ops_write_local`) or `h.state.f` whose body writes `h`'s `STATE`
      (`ops_write_state`, new: a `StateAssign` on `h` or a by-ref capture of it)
      takes the operand snapshot (plan-142-F's drop, freed with the statement on
      every exit edge) and pushes neither `for_each_iterable_record_fields` nor
      `for_each_iterable_state_fields`.
- [x] Runtime cases (`tests/runtime/rt_inplace_field_loop.rs` + stanza):
  - entry-time elements for four ops × two containers;
  - `EXIT FOR` and `RETURN` from inside the loop;
  - the no-leak assertion.
  `append`, `removeAt`, `sort`, `filter` × record and `STATE` visit `3,1,2,`;
  `EXIT FOR`, `RETURN`, a 1,000-element loop that doubles its field, and 100
  loops over a `STATE` field: `alloc_calls = free_calls`, `live_bytes 0`.
  `MFB_TEST_EXE=target/debug/mfb cargo test --test rt_inplace_field_loop` → "3
  passed"; the plan-145-C compiler → FAILED ("a field loop leaked: 8052
  allocated, 5048 freed, 12344544 B live").
- [x] RED proof: skip the owned lowering, and confirm the no-leak assertion
      fails with today's leak. Restore.
      In a copy of the tree under `/tmp`, `owns_owner_field_iterable` forced
      `false`: both loop tests fail with the same leak (8052 allocated, 5048 freed,
      12,344,544 B live). The worktree was never changed.
- [x] Flip S7/T7. Remove their `FIELD_PENDING` entries.
      `LANDED += H` (with S9, Phase 2): `field_expect.tsv` S7, T7, S9 → `arm`;
      the `ALIAS` entries removed — `FIELD_PENDING` is now empty. The S7/T7 kind
      lines stay `na:TYPE_FOR_EACH_REQUIRES_COLLECTION`. `cargo test --bin mfb
      self_update` → "6 passed".

Acceptance: `cargo test --test rt_inplace_field_loop` passes, and
`MFB_SELF_UPDATE_SITES=S7,T7 cargo test --test rt_inplace_self_update` passes
(est. 8 min).
The harness (`MFB_SELF_UPDATE_SITES=S7,T7,S9`) is recorded in the next commit.
Commit: `(recorded in the next commit)`

### Phase 2: S9

- [x] `WriteBack::Ref`, and the field site for a by-ref owner.
      The unopened `InPlaceDest::RefField { ref_slot, … }` loads the parent's
      record pointer through the reference; `close_inplace_dest` stores it back
      through it. The seam's `G1` gives way for a `RefField` site; the store
      routine reaches the block through the reference (`emit_field_owner_block`),
      and the mixed `WITH` builds a `RefField` for a by-ref owner.
- [x] Runtime cases in `rt_inplace_field_loop.rs`: a scalar, `append` (a grow) and
      `filter` on a captured record inside `forEach`.
      `a_record_captured_by_reference_is_updated_in_place`: 50 rounds of a
      scalar and an `append` in `forEach` lambdas, then a `filter`: `1050 151`,
      `alloc_calls = free_calls`.
- [x] Flip S9. Remove its `FIELD_PENDING` entries.
      With Phase 1's flip; `field_kinds.tsv` S9 → `arm` for the scalar, pointer
      and fixed kinds (41 lines). No `copy:` expectation is left in either file
      (`grep -c copy: field_expect.tsv field_kinds.tsv` → 0, 0).

Acceptance: the same two commands with `S9` (est. 6 min).
Recorded with Phase 1's harness run.
Commit: `(recorded in the next commit)`

## Validation Plan

- Tests above. Per-letter unit gate: `cargo test --bin mfb` — run at plan-145-I's
  full gate (plan-145-D Correction D6).
- Goldens: loop-over-field fixtures (`rt-behavior/collections/*for-each*`,
  `rt-behavior/resources/*`) are expected to diff. Objdump one of each and
  regenerate the confirmed ones.

## Corrections

- **H1 — `STATE` writes have their own walk.** `ops_write_local` sees an `Assign`
  to a local; a `STATE` payload is written by `StateAssign`, so the loop asks the
  new `ops_write_state` for `IN h.state.f`.
- **H2 — one harness run for both phases.** S7, T7 and S9 flip together
  (`LANDED += H`), so one `MFB_SELF_UPDATE_SITES=S7,T7,S9` run is both phases'
  acceptance.

## Summary

The field versions of plan-142-F and -G. The loop walks a copy when its body
writes the field, which also ends the §3.5 leaks. A by-ref owner writes back
through its reference.
