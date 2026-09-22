# plan-145-D: plan-142's cannot-reallocate arms at a field

Last updated: 2026-09-21
Effort: large (3h–1d)
Depends on: plan-145-C

Prerequisites: see plan-145-A.

plan-142 gave a plain binding 26 arms. A record field has only its original 9,
and a `STATE` field its 8. So 53 of the 63 collection rows are `n (no arm)` at S4
and 42 at T2 (findings §3.2 item 3). This letter brings over the arms whose
lowering **never stores a new block pointer into its slot** for the element kinds
it admits. They mutate an inlined field through its sub-block address
(`open_inplace_inlined_subblock`, `inplace_dest.rs:610`), the route
`.ai/collections.md` (`:215-231`) sets out for the "cannot grow" row.

| arm | admitted here for | why it cannot reallocate (Phase 1 confirms each) |
|---|---|---|
| `filter take drop mid distinct` | fixed-width element | a fixed-width list has no payload order to repair: the range form is one block move, the marks form a walk (plan-142-B Phase 2). The out-of-order repack (`list_compact.rs:454`) is reachable only for a variable-width element, so that kind is letter E |
| `math` (16 functions) | 8-byte lanes | `try_inplace_math_assign` copies scratch lanes over `x`'s data (`builder_inplace_rewrite.rs:45`) |
| `sort sortBy` | every element kind | permutation of entries (`list_permute.rs`) |
| `intersection difference` | every element kind | `try_inplace_filter_set` compacts (`builder_inplace_setmap.rs:646`) |
| `replace transform mapValues` | fixed-width element or value | same-width rewrite. `emit_reserve_list_tail` is a no-op when the stride is 0 (`builder_inplace_rewrite.rs:124`) |

## 1. Goal

- Each arm's `FieldReach` becomes `NoRealloc { kinds }`. Its `FIELD_PENDING`
  entries at S4, T2, T4 and T8 are removed, and the matrix fires them there.
- `field_expect.tsv`: those rows flip to `arm` at S4, T2, T4 and T8. If Open
  Decision 2 (plan-145-A) is decided yes, they also flip at S3, T1 and T3, and so
  do the existing sub-block arms `removeKey`, `removeAt`, Set `remove` and
  fixed-width `set`.
- Failure atomicity at a field: `filter`'s failing predicate, `mid`'s bad range and
  a `math` domain error each leave `r.b` and `h.state.b` unchanged
  (`rt_inplace_failure_atomic` gains field cases).
- The debug report shows `live_bytes 0` after the field cases. A shrunk sub-block
  must not leak the record's tail.

### Non-goals

- Variable-width `replace`, `transform`, `mapValues` and the variable-width kinds
  of the five shrink arms are letter E. They decline here.
- The copying lowerings are unchanged.

## 2. Current State

- The arms read their slot as `target.dest.block_slot()` (`grep -n 'block_slot()'`
  over `builder_inplace_{shrink,rewrite,sort,setmap}.rs` → 15 sites). For an
  `Inlined` destination that is the **record** block, not the collection. So each
  of the 15 sites must read the collection slot through one helper instead.
- The su_scratch frame slot (plan-142-B Correction B1) is per function, not per
  binding. It serves field sites unchanged.
- How the record's byte size is computed for copy and free after an inlined
  collection shrinks is UNMEASURED (Phase 1). The existing `removeAt` and
  `removeKey` record arms already shrink a last-inlined sub-block, and plan-144
  found no leak in them (findings §3.5 names none). That is precedent, but not a
  measurement.

## 3. Design

**One helper for the collection slot.**
`inplace_collection_slot(&dest) -> usize`:

- `Direct`/`Ref`/`Global` return `block_slot()`, as now;
- `Inlined` calls `open_inplace_inlined_subblock` (emits, so it runs after the
  gates, `O-order-1`);
- `StateField` opens the STATE destination first.

The 15 sites switch to it. At a plain site the bytes do not change, and the
`rt-behavior/collections` goldens confirm it.

**Shrink arms at a field only for a fixed-width element.** A variable-width
list's compaction may take the out-of-order path, and that path repacks (a
reallocation) when the dead bytes exceed the live ones (plan-142-B Correction B3).
The order is a run-time fact, so an arm cannot decline it at compile time. Leaving
the dead bytes in place without the repack would let a `filter`/`append` loop at a
field grow without bound, because the inlined append grow copies dead bytes along
with live ones. So variable-width shrink at a field waits for letter E, where the
repack reaches the record through `InlineGrow`. The fixed-width gate is the
existing `list_entry_stride(element) == 0` test (`builder_inplace_rewrite.rs:124`).

**Not-last fields (Open Decision 2).** Phase 1 reads how record size, copy and
free find each field. If they use stored offsets and the last field's end (not a
sum of sizes), a middle sub-block that shrinks leaves slack nothing reads, and
`G17` becomes "last-inlined **or** the arm is `NoRealloc`". Record the evidence,
and apply it only if the user decided yes.

Risk: the size answer. If the record's free size is computed from a shrunk
sub-block, the arena gets back fewer bytes than it gave out. The `live_bytes 0`
check is the test for exactly that.

## Phases

### Phase 1: Measure

- [ ] For each arm above, list every store into its slot argument and every call
      that can reallocate (`emit_repack_list_data`, `lower_map_set_in_place`, any
      `arena_alloc` result stored to the slot), per element kind. Record the
      table with its `grep` commands. Any arm that can reallocate for a kind is
      moved to letter E for that kind.
- [ ] How are a record's size (`emit_record_block_size_to_slot`,
      `emit_inlined_block_size_from_ptr_slot`,
      `builder_collection_layout.rs:729`), its copy and its free computed? Offsets
      or sums? Record the answer with citations. It settles Open Decision 2's
      premise.

Acceptance: both recorded (est. 30 min).
Commit:

### Phase 2: The slot helper and the arms

- [ ] `inplace_collection_slot`, and the 15 call sites switched to it.
- [ ] The fixed-width element gate on the five shrink arms at `Inlined`.
- [ ] `FieldReach::NoRealloc` for each arm, with its element-kind gate.
- [ ] The `G17` relaxation, if Open Decision 2 is yes.
- [ ] Field cases in `rt_inplace_failure_atomic` and a `live_bytes 0` assertion.
- [ ] Flip `field_expect.tsv`, and remove the `FIELD_PENDING` entries.
- [ ] RED proof: remove the fixed-width gate, and confirm an out-of-order
      `String` list field under `filter` fails the differential probe (heap
      corruption or a wrong value). Restore.

Acceptance: `rt-behavior/collections` goldens byte-identical
(`scripts/artifact-gate.sh target/release/mfb 'rt-behavior/collections/*'`, est.
5 min). Then `cargo test --bin mfb self_update && cargo test --test rt_inplace_failure_atomic`
→ pass, and `MFB_SELF_UPDATE_SITES=S3,S4,T1,T2,T3,T4,T8 cargo test --test rt_inplace_self_update`
→ pass (est. 25 min: 79 lines × 7 sites. These are exactly the sites this letter
flips).
Commit:

## Validation Plan

- Tests above, plus a differential probe like plan-142-B's: each arm at S4 against
  the copying call on a copy, over `Integer`, an in-order `String` list, an
  out-of-order `String` list, and a record element.
- Goldens: a committed fixture whose record field self-updates with one of these
  ops is expected to diff. Find them with
  `rg -lP "(\w+) = WITH \1 \{ *\w+ := collections::(filter|take|drop|mid|distinct|sort|sortBy|intersection|difference|replace|transform|mapValues)\(\1\." tests examples --glob '*.mfb'`,
  and record the count here.
- Per-letter unit gate: `cargo test --bin mfb`.

## Corrections

## Summary

Every plan-142 arm that only shrinks or rewrites at the same width, for a
fixed-width element, reaches a field through its sub-block address, with no
repack. The risk is how a shrunk
sub-block is sized for free, and `live_bytes 0` checks it.
