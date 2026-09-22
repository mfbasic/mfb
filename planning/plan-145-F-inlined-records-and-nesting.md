# plan-145-F: Fixed-size inlined record fields, and nested paths

Last updated: 2026-09-21
Effort: large (3h–1d)
Depends on: plan-145-E

Prerequisites: see plan-145-A.

Two findings share one mechanism: a field whose value is itself an inlined record.

- **Finding 4 (143 rows).** A package-record field (`vector::*`, `color::Color`,
  the `datetime` types, `big::Int`, `http::Response`) or a nested user record is
  never in place. It is inlined (findings B.4), so Layer 1 refuses it in `STATE`,
  and no arm handles it in a record. A `vector::Float3` field is three scalars, and
  the rebuild writes the same bytes back through a whole new record.
- **Finding 6, `G17` at S6/T6.** In
  `o = WITH o { inner := WITH o.inner { b := op(o.inner.b, …) } }`, the outer
  update is a record, so every arm declines. The field path has depth 2.

After F:

- an inlined record field of **compile-time fixed size** is overwritten in place
  (the new value's bytes are copied over the sub-block);
- a nested path `o.inner.b` is a field site of depth 2 that every field-capable
  arm serves. For a reallocating arm, each level on the path must be last-inlined.

## 1. Goal

- `field_kinds.tsv`: every `InlinedFixed` kind (plan-145-A Phase 1's list) flips
  to `arm+value` at S3, S4, T1 and T2. The nested-record kind flips too.
- `field_expect.tsv`: the S6 and T6 lines flip to match S4/T2's verdict for the
  same row.
- A value check per kind: every field of the vector, colour or date reads back
  what the copying path would give, and a sibling field is untouched.

### Non-goals

- `InlinedVariable` kinds (`big::Int`, `http::Response`, a nested record holding a
  `String`) remain `Rebuild` (plan-145-A Open Decision 3).
- No change to how a package record is laid out.

## 2. Current State

- `record_field_is_inlined` (`builder_collection_layout.rs:3152`): a composite
  that is `type_is_memcpy_copyable` is inlined. That is every package record type
  among the rows (findings B.4).
- The fixed-size list comes from plan-145-A Phase 1. Its count is UNMEASURED until
  then.
- A register-native vector value materializes to a block only where a block is
  needed (`vector_value_as_block`, `builder_control.rs` in `StateAssign`). An
  overwrite can store lanes straight into the sub-block, with no temp block.
  Whether each vector op's result is register-native is UNMEASURED (Phase 1).

## 3. Design

**Fixed-size overwrite.** Handled in the field-site builder before the arms (type
driven, like letter C's scalars). The conditions:

- the field's type is `InlinedFixed`;
- any number of such updates, alongside the scalar updates of letter C.

Evaluate each new value (spill it), then copy `size` bytes into
`record + fieldOffset`. The size is a compile-time constant, so no size read is
needed.

- If the value is a fresh block, free it after the copy. Its memory is the
  `arm+value` allowance.
- If it is register-native, store the lanes directly.

`STATE` owners also take this path. The block pointer comes through
`RESOURCE_OFFSET_STATE`, and nothing moves, so there is no write-back.

**Nested paths.** `FieldSite` gets a `path: Vec<(field_index, record_type)>`
instead of a single index. The site builder peels nested single-update `WITH`s
whose target is the parent path:

- `WITH o.inner { … }` inside `inner := …`;
- `WITH h.state.inner { … }`.

Each level must pass `G13`/`G14`.

- The sub-block address is the sum of the stored offsets along the path.
- `InlineGrow` for a path needs every level last-inlined. It grows the **outer**
  block by the inner growth. Inner offsets are block-relative to their own record,
  so they are unchanged. Only the outer size changes.

`field_off_slot` becomes the summed offset, read once before the grow.

Risk: the summed offset across a grow at depth 2. It must be read once, before the
grow, and every level's last-inlined proof must hold, or a sibling shifts.

## Phases

### Phase 1: Measure

- [ ] For each `InlinedFixed` kind, check whether its self-update-shaped ops return
      a register-native value or a block. Build one probe per kind with
      `--ncode`, and grep for the vector lanes vs an `arena_alloc`. Record the
      table.

Acceptance: table recorded (est. 15 min).
Commit:

### Phase 2: Fixed-size overwrite

- [ ] Field-site builder: the `InlinedFixed` overwrite for records and `STATE`.
- [ ] Flip the kind lines, and update `FIELD_KIND_TABLE`.
- [ ] RED proof: revert the builder call, and confirm the kind lines fail the
      bound.

Acceptance: `MFB_SELF_UPDATE_SITES=S3,S4,T1,T2 cargo test --test rt_inplace_self_update`
filtered to the kind lines → pass (est. 4 min).
Commit:

### Phase 3: Nested paths

- [ ] `FieldSite.path`, peeling the nested `WITH`, summed offsets, `InlineGrow`
      over a path.
- [ ] Runtime cases (`tests/runtime/rt_inplace_field_nested.rs` + stanza):
  - depth-2 `append` and `filter`;
  - a not-last inner level (asserts a decline);
  - a sibling of `inner` kept intact across the outer grow.
- [ ] Flip S6/T6 in `field_expect.tsv`, and remove the `FIELD_PENDING` entries.

Acceptance: `cargo test --test rt_inplace_field_nested` passes, and
`MFB_SELF_UPDATE_SITES=S6,T6 cargo test --test rt_inplace_self_update` passes
(est. 8 min).
Commit:

## Validation Plan

- Tests above; differential probe over every `InlinedFixed` kind.
- Goldens: `rt-behavior/vector` and the datetime and colour fixtures whose records
  hold such fields are expected to diff. Run
  `scripts/artifact-gate.sh target/release/mfb 'rt-behavior/*'` once, objdump one
  diff per directory, and regenerate only the confirmed ones.
- Per-letter unit gate: `cargo test --bin mfb`.

## Corrections

## Summary

A fixed-size inlined record field is a byte overwrite, and a nested path is a
field site with a longer path. Every arm reaches both through the same seam.
