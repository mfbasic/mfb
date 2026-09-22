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

- [x] For each arm above, list every store into its slot argument and every call
      that can reallocate (`emit_repack_list_data`, `lower_map_set_in_place`, any
      `arena_alloc` result stored to the slot), per element kind. Record the
      table with its `grep` commands. Any arm that can reallocate for a kind is
      moved to letter E for that kind.
      Read with `grep -n "store_u64\|emit_repack_list_data\|emit_map_reserve\|emit_reserve_list_tail\|arena_alloc"`
      over each arm's body and the helpers it calls:

      | arm | stores to its slot / reallocates | fixed width | variable width |
      |---|---|---|---|
      | `take drop mid` (`lower_list_compact_in_place` range form) | fixed width: loads the slot only (a block move) | never | the out-of-order path calls `emit_repack_list_data`, which stores a new block → E |
      | `filter distinct` (marks form, scratch marks) | as above | never | → E |
      | `math` | copies 8-byte scratch lanes over the data; no store | never | n/a (Float/Integer only) |
      | `replace transform` | `emit_reserve_list_tail` is a no-op at stride 0; `lower_list_set_in_place` writes the entry in place | never | a longer value reserves tail room (a repack) → E |
      | `sort sortBy` | `lower_list_permute_in_place` loads only | never | never (every kind) |
      | `intersection difference` | `lower_map_compact_in_place` / `lower_map_clear_in_place` never store the slot or allocate | never | never (every kind) |
      | `mapValues` | a fixed value needs no `emit_map_reserve`; the value is written in place | never | a longer value reserves (a new block) → E |

      So the table's split stands: the five shrink arms and `replace transform
      mapValues` at a field only for a fixed-width element (value), the rest for
      every kind. `grep -n 'block_slot()'` over the four files at `062c50c4c` → 15
      sites, 12 in these arms and 3 in `union`, `symmetricDifference` and `merge`
      (letter E's, which grow).
- [x] How are a record's size (`emit_record_block_size_to_slot`,
      `emit_inlined_block_size_from_ptr_slot`,
      `builder_collection_layout.rs:729`), its copy and its free computed? Offsets
      or sums? Record the answer with citations. It settles Open Decision 2's
      premise.
      **Offsets.** `emit_record_block_size_to_slot` reads the LAST present inlined
      field's stored offset and adds that sub-block's size
      (`emit_inlined_block_size_from_ptr_slot`); it never sums the fields. The copy
      (`copy_flat_block`) and the free (`emit_owned_value_drop` →
      `emit_inlined_block_size_from_ptr_slot`) both take that size. A collection's
      block size (`emit_flat_block_size`) is computed from its **capacity**, and a
      shrink keeps the capacity, so a shrunk middle sub-block keeps its byte
      extent: the slack sits inside it, and nothing reads it. Open Decision 2's
      premise holds; it was adopted (plan-145-A Correction A11), so the `G17`
      relaxation applies.

Acceptance: both recorded (est. 30 min).
Both recorded above.
Commit: recorded with Phase 2 (below)

### Phase 2: The slot helper and the arms

- [x] `inplace_collection_slot`, and the 15 call sites switched to it.
      The 12 sites in the D arms switched (`inplace_dest.rs`
      `inplace_collection_slot`); the other 3 are `union`, `symmetricDifference` and
      `merge`, letter E's (Correction D1). Each arm opens its destination after its
      gates (`open_inplace_dest`), takes the slot after its operands, and closes a
      field (`close_field_dest`).
- [x] The fixed-width element gate on the five shrink arms at `Inlined`.
      `resolve_shrink` (`kind2_payload_size`), and the same gate on `replace`,
      `transform` (`list_entry_stride`) and `mapValues` (`value_type_is_fixed`).
- [x] `FieldReach::NoRealloc` for each arm, with its element-kind gate.
      A unit variant: the kind gate is in each arm (Correction D2). The field
      self-update scratch prescan (`with_holds_field_self_update`) gives `filter`,
      `distinct`, `sort`… their scratch at a field (Correction D3).
- [x] The `G17` relaxation, if Open Decision 2 is yes.
      Yes (Phase 1; plan-145-A A11). `resolve_self_update`'s field branch asks only
      for an inlined collection field; the growing routes (`append`, `add`, Map
      `set`, `splice`) ask `field_is_last_inlined` themselves.
- [x] Field cases in `rt_inplace_failure_atomic` and a `live_bytes 0` assertion.
      `a_failing_field_self_update_leaves_the_owner_unchanged`: `filter`, `mid`,
      `math::sqrt` over a NOT-last record field and `STATE` field, then
      `alloc_calls = free_calls` and `live_bytes 0`.
      `MFB_TEST_EXE=target/release/mfb cargo test --test rt_inplace_failure_atomic`
      → "2 passed".
- [x] Flip `field_expect.tsv`, and remove the `FIELD_PENDING` entries.
      `LANDED = {"B", "C", "D"}` in `field_expect_gen.py`, regenerated: 342 lines
      `copy:D` → `arm` (39 each at S4, S10, T2, T4, T5, T8; 36 each at S3, T1, T3).
      30 `FIELD_PENDING` entries removed (the 13 arms at the last and not-last sites
      and at the mixed sites, and `set`/`removeKey`/`removeAt`/Set `remove` at
      the not-last sites).
- [x] RED proof: remove the fixed-width gate, and confirm an out-of-order
      `String` list field under `filter` fails the differential probe (heap
      corruption or a wrong value). Restore.
      With `resolve_shrink`'s gate made `false` (a copy of the tree under `/tmp`),
      the probe's `filterShort/SO/*` (an out-of-order `String` field filtered down
      to its short elements, so dead bytes exceed live ones and the compaction
      repacks) → exit 139 (SIGSEGV); `(take|drop|mid|distinct)/SO/*` → exit 139
      too. With the gate: `filterShort/*` 8 of 8 `ok`, `live=0`.

Acceptance: `rt-behavior/collections` goldens byte-identical
(`scripts/artifact-gate.sh target/release/mfb 'rt-behavior/collections/*'`, est.
5 min). Then `cargo test --bin mfb self_update && cargo test --test rt_inplace_failure_atomic`
→ pass, and `MFB_SELF_UPDATE_SITES=S3,S4,T1,T2,T3,T4,T8 cargo test --test rt_inplace_self_update`
→ pass (est. 25 min: 79 lines × 7 sites. These are exactly the sites this letter
flips).
- Byte identity (the gate has no glob selector, plan-145-C Correction C1): the
  `-ncode -target linux-x86_64` dump of all 145 fixtures under
  `rt-behavior/collections`, `rt-behavior/resources` and `byte-identity`, built by
  the plan-145-C compiler and by D → "same=143 diff=0" (the 2 others are package
  directories, not projects). `scripts/artifact-gate.sh target/release/mfb all` →
  5 diffs, all `byte-identity/http` — bug-677's fix, regenerated there.
- `cargo test --bin mfb self_update` → "6 passed".
- `cargo test --test rt_inplace_failure_atomic` → "2 passed".
- The seven-site harness run (`MFB_SELF_UPDATE_SITES=S3,S4,T1,T2,T3,T4,T8`) is recorded in the next commit.
- D4 changed the mixed path: `MFB_SELF_UPDATE_SITES=S10,T5` is re-run and recorded in the next commit.
Commit: `(recorded in the next commit)`

## Validation Plan

- Tests above, plus a differential probe like plan-142-B's: each arm at S4 against
  the copying call on a copy, over `Integer`, an in-order `String` list, an
  out-of-order `String` list, and a record element.
  **Result:** `/tmp/p145/dprobe/gen.py` — every D arm (and `math::abs`,
  `math::sqrt`) at S4, S3, T2 and T1 (last and not-last, record and `STATE`), over
  `Integer`, in-order and out-of-order `String`, a record element, `Float`, `Set`s
  and `Map`s, three rounds each against the copying call on an independent
  shadow list: 164 of 164 `ok`, `alloc_calls = free_calls`, `live_bytes 0`. The
  first run showed 2304 B live — the same with the plan-145-C compiler: bug-677
  (`transform`'s block results leaked), fixed here.
- Goldens: a committed fixture whose record field self-updates with one of these
  ops is expected to diff. Find them with
  `rg -lP "(\w+) = WITH \1 \{ *\w+ := collections::(filter|take|drop|mid|distinct|sort|sortBy|intersection|difference|replace|transform|mapValues)\(\1\." tests examples --glob '*.mfb'`,
  and record the count here.
  **Result:** that `rg` → 0 files.
- Per-letter unit gate: `cargo test --bin mfb`. Run at the end of the plan
  (plan-145-I's full gate covers every letter; Correction D6).

## Corrections

- **D1 — 15 `block_slot()` sites, 12 of them D's.** `grep -n 'block_slot()'` over
  the four files at `062c50c4c` → 15: the 12 in the D arms, and 3 in `union`,
  `symmetricDifference` and `merge` — E's arms, which grow. They switch in E.
- **D2 — `FieldReach::NoRealloc` is a unit variant.** The kinds an arm admits at a
  field are gated in the arm (the element-width test), not carried in the table.
- **D3 — a field self-update needs the scratch prescan.** `filter`, `distinct`,
  `sort`, `sortBy`, `mapValues`… decline without the function's self-update
  scratch, which `ops_hold_self_update` claimed only for `x = f(x, …)`. It now
  also recognizes `r = WITH r { f := g(r.f, …) }` and `h.state = WITH h.state
  { f := g(h.state.f, …) }`, one field or mixed (`with_holds_field_self_update`).
- **D4 — the mixed `WITH`'s `G25` asked about the arm's call, not its operands.**
  `sortBy` and `mapValues` never fired at T5: `try_inplace_mixed_with` passed the
  whole update value to `inplace_state_operands_reach_a_state_assign`, and the
  `.mfb`-bodied builtin call reads as user code. It now asks about the arm's
  operands (as `resolve_self_update` does) and each scalar's value. Found by the
  matrix (`every_arm_row_fires_at_every_enabled_site`: "collections::sortBy at T5:
  no probe fired SortBy").
- **D5 — bug-677, found by the differential probe.** `transform` (copying and in
  place) and the in-place `mapValues` never freed a callback's record, union or
  collection result. Fixed with `free_callback_result`; record in
  `bugs/completed/bug-677-*`, regression test `rt_transform_block_result_frees`.
- **D6 — the per-letter unit gate runs once, at the end.** `cargo test --bin mfb`
  takes an hour (3578 s at B). The scoped `self_update` filter covers this
  letter's code; the full run is plan-145-I's gate.
- **D7 — the harness's `FieldReach` edit.** A regex edit first gave `NoRealloc` to
  `union` and `symmetricDifference` and left `filter` and `intersection` at
  `None`; the matrix caught it ("the arm ran without evaluating its scalars"), and
  every result above is from the corrected table.

## Summary

Every plan-142 arm that only shrinks or rewrites at the same width, for a
fixed-width element, reaches a field through its sub-block address, with no
repack. The risk is how a shrunk
sub-block is sized for free, and `live_bytes 0` checks it.
