# plan-145-C: Scalar fields, pointer fields and the mixed `WITH`

Last updated: 2026-09-21
Effort: large (3h–1d)
Depends on: plan-145-B

Prerequisites: see plan-145-A.

This letter closes the findings' largest record-vs-`STATE` gap (§3.2 item 1: 68
rows, 408 cells):

- `h.state.hp = h.state.hp - dmg` is a single store (Layer 1,
  `try_inplace_state_scalar_assign`, `builder_control.rs:144`).
- `r = WITH r { hp := r.hp - dmg }` rebuilds the whole record through
  `lower_with_update` (`builder_value_semantics.rs:703`). No record arm handles a
  non-collection field (`G17`).

After C, a record local gets Layer 1's twin. Two more cases become in place:

- a pointer field (`json::Json`) is replaced in its slot (plan-145-A Open
  Decision 4);
- a two-field `WITH` that pairs one arm-served field with scalar fields (S10/T5)
  is in place. `G14` declines it today (findings §3.2 item 6).

## 1. Goal

- `field_kinds.tsv` lines `Integer Float Fixed Money` flip to `arm` at S3, S4 and
  S10. The `json::Json` line flips to `arm` at S3 and S4.
- `field_expect.tsv`: the 10 arm rows flip to `arm` at S10, T5 (a scalar update
  beside the collection update). The 68 scalar rows flip to `arm` at S10.
- Failure atomicity: `r = WITH r { hp := r.hp - big, xs := append(r.xs, k) }`
  inside a `TRAP` whose operand overflows leaves `r` fully unchanged, both fields.
- Order of effects: two updates whose values call a printing `FUNC` print in
  source order.

### Non-goals

- `WITH` semantics stay the same: every update expression reads the **old**
  record, and updates are evaluated in source order.
- S5 (global) and S9 (by-ref) are letters G and H. This letter declines them.
- Inlined non-collection fields are letter F (fixed-size) or `Rebuild`.

## 2. Current State

- Layer 1 admits a `STATE` update iff every updated field is
  `!record_field_is_inlined && !record_field_is_pointer`. That is exactly
  `Integer Float Fixed Money` among the rows (findings B.6). It computes every
  value first (source order, spilled), then loads the `STATE` pointer once and
  stores each value at `8 * index`. `G25` declines an operand that can reach a
  `STATE` assignment.
- The record side has no twin. In a record block, a scalar field is inline in its
  8-byte slot, the same as in a `STATE` block (the same `record_fields`
  layout). A pointer field holds an owned pointer that the record's drop frees
  (`record_field_is_pointer`, `builder_collection_layout.rs:2815`).
- Record locals can carry facts keyed on the local: `local.constant`,
  `len_of_local`, `provable_index_locals`, and promoted float locals. Which ones
  can name a record **field** is UNMEASURED (Phase 1).

## 3. Design

**One scalar-store routine for both containers.** Replace Layer 1 with
`try_inplace_scalar_fields(site_owner, value)`. `site_owner` is a record local
(block in its slot) or a `STATE` resource (block through
`RESOURCE_OFFSET_STATE`). The eligibility, the source-order compute and spill,
and the stores are Layer 1's, unchanged. Only the block pointer's source
differs. For a record local, the other gates are:

- `G1` declines (by-ref is letter H);
- live `FOR EACH` gates do not apply, because a scalar is never an iterable and
  storing one moves no block;
- `G25` is `STATE`-only.

For the `STATE` owner, the `.ncode` must stay byte-identical. That is checked on
`rt_res_state_inplace_mutation`'s fixtures.

**Pointer fields (records only).** Evaluate the new value owned, spill it, load
the old pointer from the slot, drop it with `emit_owned_value_drop`, then store
the new pointer. The new value is built before the old one is freed, so
`j := json::set(r.j, …)` still reads the live old value. The drop is the same drop
the record's scope exit would run on that field.

**The mixed `WITH` (relaxing `G14`).** Admit a `WITH` with any number of updates
when exactly one update is a non-scalar field whose field-capable arm accepts it,
and every other update is scalar- or pointer-eligible. Order:

1. evaluate each scalar value **that precedes** the arm's update in source
   order, and spill it;
2. run the arm (its operands evaluate here; the arm is failure-atomic);
3. evaluate the remaining scalar values;
4. store all scalars.

Step 3 runs after the arm has mutated the arm's field. So admit the statement
only if no update after the arm's update reads the arm's field. The site's
`read_by` is the test. Otherwise decline to the rebuild. A failure in step 1 or 3
must leave the record unchanged. Step 1 is safe: nothing has been stored yet.
Step 3 is not, because the arm has already written. So also require that every
update after the arm's update cannot fail. The predicate for "cannot fail" is
UNMEASURED; Phase 1 finds it (candidates: `inline_builtin_is_infallible`,
`ir::fallible`). If no usable predicate exists, admit only the order
`scalars…, arm` (the arm last). Record that as a Correction.

Risk: the mixed `WITH`'s ordering and atomicity rules. A wrong admit drops or
reorders a write. Each rule gets a runtime case that asserts the **decline**.

## Phases

### Phase 1: Measure

- [ ] Which per-local facts can be keyed on a record field: read `len_of_local`,
      `provable_index_locals`, `promoted_float_locals` and `local.constant` for
      record-field keys. Record, with citations, which facts a field store must
      invalidate.
- [ ] Is there a codegen-visible "cannot fail" predicate for an arbitrary scalar
      expression? Record the answer and its evidence (see §3).

Acceptance: both answers recorded here (est. 20 min).
Commit:

### Phase 2: Scalar and pointer fields

- [ ] `try_inplace_scalar_fields` for both owners. Layer 1 becomes a call to it.
- [ ] Pointer-field replace for records.
- [ ] `NirOp::Assign`: try it before the field seam.
- [ ] Flip the `field_kinds.tsv` lines. Remove the matching `FIELD_KIND_TABLE`
      pending entries.
- [ ] RED proof per `.ai/testing-gates.md:809`: revert the `Assign` call and
      confirm the flipped lines fail the bound. Restore.

Acceptance: `rt_res_state_inplace_mutation` fixtures byte-identical
(`scripts/artifact-gate.sh target/release/mfb 'rt-behavior/resources/*'`, est. 5 min).
Then `MFB_SELF_UPDATE_SITES=S3,S4,T1,T2 cargo test --test rt_inplace_self_update`
filtered to the kind lines → pass (est. 3 min).
Commit:

### Phase 3: The mixed `WITH`

- [ ] Relax `G14` as §3 describes, in the field-site builder, for both owners.
- [ ] Runtime cases in `tests/runtime/rt_inplace_field_mixed.rs` (+ stanza):
  - the atomicity case in §1;
  - the effect-order case;
  - a later update reading the arm's field (asserts a decline and the correct
    value);
  - a pointer and a scalar in one `WITH`.
- [ ] Flip the S10/T5 lines in `field_expect.tsv`.

Acceptance: `cargo test --test rt_inplace_field_mixed` passes, and
`MFB_SELF_UPDATE_SITES=S10,T5 cargo test --test rt_inplace_self_update` passes
(est. 8 min).
Commit:

## Validation Plan

- Tests: the kind lines, S10/T5 lines, `rt_inplace_field_mixed`.
- Goldens: every committed fixture with a scalar record-field `WITH` is expected
  to diff (the rebuild becomes stores). Name them in Phase 2 by running
  `scripts/artifact-gate.sh target/release/mfb all` once. Objdump one per
  directory to confirm the diff is the store replacing `lower_with_update`, then
  regenerate only those. Record the count and the command.
- Per-letter unit gate: `cargo test --bin mfb`.

## Corrections

## Summary

Layer 1 becomes a routine for both containers, a pointer field is replaced in its
slot, and a `WITH` may carry scalars beside one arm. The risk is the mixed
`WITH`'s ordering, and it is fenced by decline tests.
