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

- [ ] `lower_for_each`: the field-write body walk, the owned iterable, and no
      tracking push.
- [ ] Runtime cases (`tests/runtime/rt_inplace_field_loop.rs` + stanza):
  - entry-time elements for four ops × two containers;
  - `EXIT FOR` and `RETURN` from inside the loop;
  - the no-leak assertion.
- [ ] RED proof: skip the owned lowering, and confirm the no-leak assertion
      fails with today's leak. Restore.
- [ ] Flip S7/T7. Remove their `FIELD_PENDING` entries.

Acceptance: `cargo test --test rt_inplace_field_loop` passes, and
`MFB_SELF_UPDATE_SITES=S7,T7 cargo test --test rt_inplace_self_update` passes
(est. 8 min).
Commit:

### Phase 2: S9

- [ ] `WriteBack::Ref`, and the field site for a by-ref owner.
- [ ] Runtime cases in `rt_inplace_field_loop.rs`: a scalar, `append` (a grow) and
      `filter` on a captured record inside `forEach`.
- [ ] Flip S9. Remove its `FIELD_PENDING` entries.

Acceptance: the same two commands with `S9` (est. 6 min).
Commit:

## Validation Plan

- Tests above. Per-letter unit gate: `cargo test --bin mfb`.
- Goldens: loop-over-field fixtures (`rt-behavior/collections/*for-each*`,
  `rt-behavior/resources/*`) are expected to diff. Objdump one of each and
  regenerate the confirmed ones.

## Corrections

## Summary

The field versions of plan-142-F and -G. The loop walks a copy when its body
writes the field, which also ends the §3.5 leaks. A by-ref owner writes back
through its reference.
