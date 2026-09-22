# plan-145-E: plan-142's reallocating arms at a last-inlined field

Last updated: 2026-09-21
Effort: large (3h–1d)
Depends on: plan-145-D

Prerequisites: see plan-145-A.

These plan-142 arms can store a new block pointer into their slot, so they cannot
use a sub-block address. Handed one, a lowering that reallocates frees a pointer
into the middle of the live record (`.ai/collections.md:233-238`).

| arm | where it reallocates |
|---|---|
| `union`, `symmetricDifference` | `lower_map_set_in_place(…, None)` per added element (`builder_inplace_setmap.rs:499-506`) |
| `merge` | `lower_map_set_in_place(…, None)` (`:877-884`) |
| `mapValues`, variable-width value | the same, for a value of a new width |
| `replace`, `transform`, variable-width element | `emit_reserve_list_tail` → `emit_repack_list_data` (`builder_inplace_rewrite.rs:150`) |
| `filter take drop mid distinct`, variable-width element | the compaction's out-of-order repack (`list_compact.rs:454`), which is why letter D admitted them at a field only for a fixed-width element |

The route already exists for the growing record arms: `Option<InlineGrow>`
(`map_mutate.rs:32`). Its three rules are:

1. request `fieldOffset + collectionSize`;
2. treat the allocation as the new record and copy `[0, fieldOffset)` verbatim;
3. free the old record.

After E, every reallocation site these arms reach takes an `Option<InlineGrow>`,
so each arm runs at a **last-inlined** field (`G17`) with the record block
repointed. At a `STATE` field it is also published back (`close_inplace_dest`).

## 1. Goal

- `FieldReach::Realloc` for `union symmetricDifference merge`, and for the
  variable-width kinds of `replace transform mapValues` and the five shrink
  arms. Their `FIELD_PENDING` entries at S4, T2, T4 and T8 are removed.
- `field_expect.tsv`: every F1 row is `arm` or `rebuild:new-value` at S4, T2, T4
  and T8. Count the lines left as `copy:D`/`copy:E` with
  `grep -cE 'copy:(D|E)'` → 0.
- The compaction repacks at an inlined field, through `InlineGrow`: a
  500-iteration `filter`/`append` loop on an out-of-order `String` field ends with
  its data region no larger than twice its live bytes. Plan-142-B's probe measures
  this through the debug report's `live_bytes`.
- Failure atomicity: `ErrOutOfMemory` can only occur at the reserve or grow, which
  runs before the first write. Assert it by reading the arm: the grow sits above
  the first element store in the emitted `--ncode`, named by label.

### Non-goals

- Not-last fields keep `G17` for these arms. A grow would shift the sibling
  sub-blocks.
- `String` fields (plan-145-A Open Decision 1).

## 2. Current State

- `lower_map_set_in_place` takes `inline: Option<InlineGrow>` already. The D arms
  pass `None`.
- `emit_repack_list_data` (`list_mutate.rs:3148`) has no `InlineGrow` parameter.
  It allocates the new list block, copies payloads packed, frees the old block,
  and stores the new pointer into `buffer_slot`.
- `lower_inline_list_append_in_place` shows the record-grow shape for lists.

## 3. Design

- `emit_repack_list_data(…, inline: Option<InlineGrow>)`: when `Some`, allocate
  `fieldOffset + newListSize`, copy the record prefix verbatim, then build the
  packed list at the new record's `+fieldOffset`. Free the old **record** and
  repoint `block_slot`. That is the same three rules as the map route.
  `emit_reserve_list_tail` and the compaction pass it through.
- Each arm reads the field offset once, before any grow
  (`open_inplace_inlined_field_offset`). It passes `InlineGrow { block_slot,
  field_off_slot }` wherever it passed `None`.
- **The sub-block address goes stale after a grow.** The arms written for a plain
  local reload the collection pointer from the slot after each
  `lower_map_set_in_place`. At a field they must instead recompute
  `record + fieldOffset`. The slot helper from letter D gains a `reload` for
  `Inlined`, and every post-grow reload in these arms goes through it. Phase 1
  lists each reload site.

Risk: a stale sub-block pointer after a grow inside a loop (`union` adds many
elements). A missed reload writes into the freed old record, and the out-of-order
differential probe is the test.

## Phases

### Phase 1: Measure

- [ ] List every point in these arms where the collection pointer is read after
      a call that can reallocate. Record them with line numbers.

Acceptance: list recorded (est. 15 min).
Commit:

### Phase 2: `InlineGrow` through the repack

- [ ] `emit_repack_list_data` and `emit_reserve_list_tail` take
      `Option<InlineGrow>`, and the compaction's repack passes it.
- [ ] Unit codegen test in `src/codegen/builtins/tests/`: an inlined repack emits
      the prefix copy and a record free, not a list free.

Acceptance: `cargo test --bin mfb inline_repack` → pass (est. 5 min).
Commit:

### Phase 3: The arms

- [ ] `union symmetricDifference merge`, and the variable-width kinds of
      `mapValues replace transform filter take drop mid distinct`, at `Inlined`,
      each with `InlineGrow` and the reload helper.
- [ ] Flip `field_expect.tsv`, and remove the `FIELD_PENDING` entries.
- [ ] Differential probe: each arm at S4 and T2 against the copying call on a
      copy, over `Integer`, in-order and out-of-order `String` lists and maps, and
      a 500-iteration loop. Record the ok count and `alloc_calls = free_calls`.
- [ ] RED proof: drop one reload in `union` and confirm the probe fails. Restore.

Acceptance: `cargo test --bin mfb self_update` passes, and
`MFB_SELF_UPDATE_SITES=S4,T2,T4,T8 cargo test --test rt_inplace_self_update` passes
(est. 15 min).
Commit:

## Validation Plan

- Tests above; `rt_inplace_failure_atomic` field cases for `union` and `merge`.
- Goldens: the fixtures D's `rg` found for these ops, plus any the artifact gate
  flags in `rt-behavior/collections`. Each is objdumped once to confirm the arm.
- Per-letter unit gate: `cargo test --bin mfb`.

## Corrections

## Summary

One route (`InlineGrow`) now covers every reallocation a field arm can reach, and
a reload helper keeps the sub-block pointer fresh across grows. After E, every
collection row with an arm is in place at every last-inlined field.
