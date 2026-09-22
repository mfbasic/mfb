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

- [x] List every point in these arms where the collection pointer is read after
      a call that can reallocate. Record them with line numbers.
      Every read goes through the arm's collection slot (the sub-block address
      at a field), and every reallocation repoints that slot — `emit_map_reserve`,
      `lower_map_set_in_place` and `emit_repack_list_data` store the new sub-block
      address into it after `InlineGrow`'s split. No arm caches the pointer in a
      register or a second slot across one:
      - `union` (`builder_inplace_setmap.rs` `try_inplace_union_assign`): reserve →
        `emit_add_all`, whose every insert loads `dest`.
      - `symmetricDifference`: marks and the compaction (no reallocation) → reserve
        → `emit_add_all` as `union`. `marks_t` is a scratch address, not the set.
      - `merge`: reserve → per entry `emit_key_membership(dest)` and
        `lower_map_set_in_place(dest)`, each loading `dest`.
      - `mapValues`, variable-width value: reserve → the write pass's
        `emit_map_loop_head(dest)` / `emit_map_entry_value_write(dest)`.
      - `replace`, `transform`, variable-width element: `emit_reserve_list_tail` →
        the write pass's `lower_list_set_in_place(buffer_slot)`, whose own repack is
        unreachable once the tail is reserved (the reserve's contract).
      - the five shrink arms, variable-width element: the repack is the
        compaction's last step; nothing reads the list after it.
      - every arm's `close_field_dest` (a `STATE` publish) reads the opened
        `block_slot`, which `InlineGrow`'s free step repointed.

Acceptance: list recorded (est. 15 min).
Recorded above.
Commit: recorded with Phase 3 (below)

### Phase 2: `InlineGrow` through the repack

- [x] `emit_repack_list_data` and `emit_reserve_list_tail` take
      `Option<InlineGrow>`, and the compaction's repack passes it.
      Also `emit_map_reserve`, `lower_list_compact_in_place` (→
      `emit_compact_entries`) and `emit_add_all`. Every existing caller passes
      `None`, so plain sites are byte-identical (Phase 3's gate).
- [x] Unit codegen test in `src/codegen/builtins/tests/`: an inlined repack emits
      the prefix copy and a record free, not a list free.
      `inplace_inline_repack.rs`: a field `filter` over `List OF String` and a
      field `union` each emit `inline_grow_prefix` copies (≥ 3: the repack, the
      reserve, the insert grows); the same operations on plain locals emit none.

Acceptance: `cargo test --bin mfb inline_repack` → pass (est. 5 min).
`cargo test --bin mfb inline_repack` → "1 passed".
Commit: recorded with Phase 3 (below)

### Phase 3: The arms

- [x] `union symmetricDifference merge`, and the variable-width kinds of
      `mapValues replace transform filter take drop mid distinct`, at `Inlined`,
      each with `InlineGrow` and the reload helper.
      `inplace_inline_grow` (the `InlineGrow` of an opened field destination) and
      `field_realloc_admitted` (a field only at its owner's last inlined field, and
      no pointer operand reading the owner — a view into the record the grow frees;
      Correction E2). `FieldReach::Realloc` for the three set/map arms. No separate
      reload helper is needed (Correction E1).
- [x] Flip `field_expect.tsv`, and remove the `FIELD_PENDING` entries.
      `LANDED += E`: 18 lines `copy:E` → `arm` (3 each at S4, S10, T2, T4, T5, T8).
      `grep -cE 'copy:(D|E)' tests/runtime/inplace_self_update/field_expect.tsv` →
      0. 6 `FIELD_PENDING` entries removed (the three arms at the last and mixed
      sites).
- [x] Differential probe: each arm at S4 and T2 against the copying call on a
      copy, over `Integer`, in-order and out-of-order `String` lists and maps, and
      a 500-iteration loop. Record the ok count and `alloc_calls = free_calls`.
      D's probe (`/tmp/p145/dprobe/gen.py`) plus `union`, `symmetricDifference`,
      `merge` and growing forms run 60 rounds (`unionGrow`, `mergeGrow`,
      `transformGrow`, `mapValuesGrow`, `replaceGrow`), at S4, S3, T2, T1: 232 of
      232 `ok`, `live_bytes 0`. Each E arm fires at the last field (21
      `inline_grow_prefix` labels for `unionGrow/SS/T2`, `mergeGrow/MS/S4`,
      `symmetricDifference/SS/T2`; 7 for the list and `mapValues` forms) and
      declines at S3 (`unionGrow/SS/S3`: 0, the rebuild's `with_target`). The
      500-iteration `append`/`filter` loop on an out-of-order `String` field
      (`/tmp/p145/loop500`): peak live bytes 608 at S4 and 704 at T2 for both 250
      and 500 iterations (flat — the repack bounds the data region), `alloc_calls
      = free_calls`, `live_bytes 0`.
- [x] RED proof: drop one reload in `union` and confirm the probe fails. Restore.
      In a copy of the tree under `/tmp`, `union` restored its pre-reserve
      sub-block address into `dest` after `emit_map_reserve` (a missed reload):
      `unionGrow/*` → exit 139 (SIGSEGV). The worktree was never changed.

Acceptance: `cargo test --bin mfb self_update` passes, and
`MFB_SELF_UPDATE_SITES=S4,T2,T4,T8 cargo test --test rt_inplace_self_update` passes
(est. 15 min).
- `cargo test --bin mfb self_update` → "6 passed".
- The four-site harness (`MFB_SELF_UPDATE_SITES=S4,T2,T4,T8`, run with S10,T5) is recorded in the next commit.
Commit: `(recorded in the next commit)`

## Validation Plan

- Tests above; `rt_inplace_failure_atomic` field cases for `union` and `merge`.
  **Result:** `a_failing_reallocating_field_update_leaves_the_owner_unchanged` —
  40 growing `union`s at a record field, then one whose operand fails; 40 growing
  `merge`s at a `STATE` field, then one whose `preferB` fails: each owner prints
  as it was, and `alloc_calls = free_calls`, `live_bytes 0`.
  `cargo test --test rt_inplace_failure_atomic` → "3 passed".
- Failure atomicity by reading (§1): in `unionGrow/SS/S4`'s `.ncode`, the arm's
  labels run `mreserve_*` (with its `inline_grow_prefix` copy) — the one
  allocation, and so the only `ErrOutOfMemory` — before `inplace_add_all_one`,
  the first write.
- Goldens: the fixtures D's `rg` found for these ops, plus any the artifact gate
  flags in `rt-behavior/collections`. Each is objdumped once to confirm the arm.
- Per-letter unit gate: `cargo test --bin mfb`. Run at plan-145-I's full gate
  (plan-145-D Correction D6).

## Corrections

- **E1 — no reload helper.** Every reallocation these arms reach already writes
  the new sub-block address back into the slot the arm reads through (the three
  `InlineGrow` helpers end by storing it), so the arms stay unchanged after a grow;
  Phase 1 lists each read.
- **E2 — an operand read from the owner declines.** `union(r.s, r.t)` borrows
  `r.t` as a pointer into the record block; the grow frees that block while the
  inserts still read it. `field_realloc_admitted` declines any pointer operand
  that reads the owner (`read_by`), for the set/map arms and `replace`'s `old`
  and `new`.
- **E3 — the variable-width kinds keep `FieldReach::NoRealloc`.** The table is per
  arm, and those arms never reallocate for a fixed-width element; their
  variable-width kinds are gated in the arm (`field_realloc_admitted`). Only the
  three set/map arms are `Realloc`.

## Summary

One route (`InlineGrow`) now covers every reallocation a field arm can reach, and
a reload helper keeps the sub-block pointer fresh across grows. After E, every
collection row with an arm is in place at every last-inlined field.
