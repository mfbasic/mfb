# bug-47: in-place collection mutators leak the abandoned buffer on grow — `prepend`, `set` (list, on grow-rebuild), and `set` (map, on value/capacity grow) omit the old-buffer free that `append`/`bulk_append` perform

Last updated: 2026-07-09
Effort: medium (1h–2h)

The MUT in-place collection mutators reallocate a larger backing buffer when they run
out of capacity, repoint the local's slot to the new buffer, and continue — but three
of them never free the buffer they abandoned. `collections::append` and the bulk
append path were fixed (they emit `bl _mfb_arena_free` of the old buffer before
installing the grown one, citing bug-01), but the sibling grow paths in
`collections::prepend`, `collections::set` on a list (larger-payload rebuild), and
`collections::set` on a map (value-grow and capacity-grow) were not. In a
mutation-heavy loop each grow orphans another block and the arena grows without bound.

This is not a crash or a wrong result — it is an unbounded memory leak, the same class
as bug-01's append-grow leak, at new sites introduced by later plan-02/plan-25 in-place
work. `grep ARENA_FREE_SYMBOL src/target/shared/code/builder_collection_mutate.rs`
returns exactly two hits (lines 981 and 1276), both inside the two append functions;
`prepend`, `list_set`, and `map_set` contain none.

The single correct behavior a fix produces: every in-place mutator that abandons a
buffer on grow frees that buffer, exactly as `append` does — so a
`prepend`/`set`-in-a-loop program's arena footprint stays proportional to live data,
not to the number of iterations.

References:

- `src/target/shared/code/builder_collection_mutate.rs:1348` (`lower_list_prepend_in_place`);
  grow-install at `:1585-1587`, no free.
- `src/target/shared/code/builder_collection_mutate.rs:1716` (`lower_list_set_in_place`);
  larger-payload rebuild branch `:1843-1881` abandons three blocks (original buffer,
  the `singleton`, the `removed` intermediate).
- `src/target/shared/code/builder_collection_mutate.rs:1908` (`lower_map_set_in_place`);
  value-grow install `:2317-2318` and capacity-grow install `:2622-2623`, neither frees.
- Correct siblings (the fix templates): `lower_list_append_in_place` (`:707`, frees at
  `:957-991`, comment at `:950-956` cites bug-01) and `lower_list_bulk_append_in_place`
  (`:1098`, frees at `:1276`).
- The bypass that makes each mutator responsible for its own free:
  `src/target/shared/code/builder_control.rs:304-318` (`try_inplace_*` short-circuit)
  skips the general-reassignment old-block free at `builder_control.rs:351-385`.
- Non-in-place contrast that frees correctly: `lower_collection_set` list branch
  (`:262-319`), via `free_intermediate_collection` (`:314-318`).
- Dispatch wiring: `builder_inplace_assign.rs:366` (prepend), `:240` (list set),
  `:287` (map set).
- Layout for the free size: `builder_collection_layout.rs:196-202`
  (`emit_flat_block_size`; map buckets = `capacity << 4`).
- Same class: bug-01 (four value-semantic collection leaks). KNOWN, not re-filed.
- Found during the goal-01 compiler source review of `src/target/shared/code/`.

## Failing Reproduction

Each of the three needs a uniquely-owned MUT local mutated in a loop. Because a
freshly built/copied collection is tight (capacity == count), the **first** mutation
hits the grow path.

```
IMPORT collections
IMPORT io

SUB main()
  MUT xs AS List OF Integer = [1, 2, 3]
  MUT i AS Integer = 0
  WHILE i < 1000000
    xs = collections::prepend(xs, i)      # grows every time capacity is hit
    i = i + 1
  WEND
  io::print(toString(collections::length(xs)))
END SUB
```

- Observed: resident arena memory grows roughly linearly with iteration count; each
  geometric grow leaks the previous buffer. (Same shape for `collections::set(m, k, v)`
  building a map, and `collections::set(xs, i, longerString)` on a list of strings.)
- Expected: arena memory stays proportional to the live collection size; abandoned
  buffers are freed on grow.

Contrast cases that work correctly today (regression guards):

- `collections::append(xs, v)` and the bulk-append path in the same loop shape do
  **not** leak — they free the old buffer (`:981`, `:1276`). This is the direct
  evidence the fix is a matter of copying their free sequence.
- The no-grow fast paths leak nothing: prepend with spare capacity, the same-size list
  `set` overwrite (`:1818-1840`), the map value-fits overwrite (`:2080-2125`), and the
  map spare-slot insert (`:2627+`) allocate nothing.
- The non-in-place `lower_collection_set` frees its `singleton` and `removed`
  intermediates.

## Root Cause

`builder_control.rs`'s `try_inplace_*` short-circuit (`:304-318`) bypasses the
general-reassignment old-block free (`:351-385`), so each in-place mutator owns the
responsibility to free any buffer it abandons. `append`/`bulk_append` discharge it;
`prepend`/`list_set`/`map_set` do not. Each grow path ends with `load x1 = new_buf;
store slot = x1; branch` and no intervening `bl _mfb_arena_free` of the pre-grow
buffer. `list_set`'s rebuild branch is the worst: it allocates a `singleton`, a
`removed` intermediate (via `lower_list_remove_at`), and a `rebuilt` result (via
`lower_list_insert_collection`), installs only `rebuilt`, and frees none of the three.

## Goal

- `collections::prepend`, `collections::set` (list grow-rebuild), and
  `collections::set` (map value-grow and capacity-grow) each free the buffer(s) they
  abandon.
- A `prepend`/`set`-in-a-loop program's arena footprint is bounded by live data.
- No double-free: a freed buffer is never one still reachable from the local's slot or
  from another live collection.

### Non-goals (must NOT change)

- The no-grow fast paths (they allocate nothing — adding a free there would be a
  double-free / use-after-free).
- `append`/`bulk_append` (already correct).
- The `try_inplace_*` short-circuit itself and the general-reassignment free — the fix
  is per-mutator, matching the established pattern, not a reroute through the general
  path.
- Collection value semantics and layout (bucket region sizing, header offsets).
- **Forbidden wrong fix:** freeing the buffer *before* the copy into the new buffer
  completes, or freeing a buffer that a value-semantic copy still shares. The free must
  come after the copy and only for a uniquely-owned abandoned block, exactly as
  `append` sequences it.

## Blast Radius

Every in-place mutator that reallocs. Found by reading each `try_inplace_*` target and
checking for an `arena_free` on its grow path.

- `lower_list_prepend_in_place` grow (`:1585-1587`) — **fixed by this bug** (free the
  old buffer).
- `lower_list_set_in_place` rebuild (`:1843-1881`) — **fixed by this bug** (free the
  original buffer + `singleton` + `removed`).
- `lower_map_set_in_place` value-grow (`:2317-2318`) and capacity-grow (`:2622-2623`) —
  **fixed by this bug** (free the old buffer incl. the bucket region).
- `lower_list_append_in_place`, `lower_list_bulk_append_in_place` — unaffected (already
  free).
- `lower_list_remove_at` in-place, `transform`/`filter` in-place (if any) — audit in
  Phase 1: confirm each either does not realloc or frees; add to scope if it leaks.
- **`NirOp::StoreGlobal` global reassignment (`builder_control.rs:257-279`) — related
  sibling, in scope.** A module-level freeable-flat global (`DIM g AS List OF Integer`)
  reassigned in a loop (`g = collections::filter(g, cb)`) calls `lower_value_owned` +
  `store_value_at` with **no free of the previous block** — leaking one block per
  iteration. Globals carry no `OwnedValue` scope-drop cleanup, and unlike the local
  `NirOp::Assign` path (`:351-385`, the bug-01 fix) `StoreGlobal` has no old-block free.
  Fix: before overwriting a global whose type `is_freeable_flat_value`, free the old block
  (load current pointer, size-from-type, `arena_free`, spilling the new value across the
  free) — the same shape as the local `Assign` path. Or accept globals as intentionally
  never reclaimed (freed at process exit) and document it. LOW (memory-growth only).

## Fix Design

For each site, before installing the new buffer, compute the abandoned buffer's size
and `bl _mfb_arena_free` it, spilling the new-buffer pointer across the call (which
clobbers all of x0–x17). Copy the exact sequence from `lower_list_append_in_place`
(`:957-991`):

- **prepend / map grow:** free the single old backing buffer. Size =
  `emit_flat_block_size(capacity, dataCapacity)` — for the map include the bucket
  region (`capacity << 4`) per `builder_collection_layout.rs:196-202`.
- **list `set` rebuild:** free the `singleton` and `removed` intermediates via the same
  helper `free_intermediate_collection` that `lower_collection_set` uses (`:314-318`),
  **and** free the original buffer held in the slot before overwriting it.

The correctness risk is entirely in the spill discipline (the new pointer must survive
the free call) and in not freeing a value-shared block — both already solved in
`append`, so the fix is a faithful port, not new design.

Rejected alternative: route in-place mutations back through the general-reassignment
free. Rejected — it defeats the purpose of the in-place short-circuit (the whole point
is to mutate without the copy/free churn) and would regress the plan-25 performance
work.

## Phases

### Phase 1 — failing test + audit

- [x] Add arena-footprint regression tests (or leak-detector runs) for a
      prepend-loop, a map-build-via-set loop, and a list-set-grow loop. Confirm each
      leaks today and that the equivalent append-loop does not. (Out-of-tree RSS
      before/after measured for all five sites — see Resolution.)
- [x] Audit every remaining `try_inplace_*` target for an unfreed realloc; extend
      scope to any additional leaking site found. (append/bulk_append already free;
      list/map set + prepend fixed here. **Found one more:** the string self-append
      regrow in `lower_string_self_append_one`, `builder_inplace_assign.rs:527-529`,
      abandons the old buffer without a free — but that file is outside this bug's
      blast radius and the free is entangled with the static-string-vs-arena
      distinction (bug-06), so it is left for a follow-up bug, not fixed here.)

### Phase 2 — the fix

- [x] Add the old-buffer free to `lower_list_prepend_in_place` grow.
- [x] Add the original + `singleton` + `removed` frees to `lower_list_set_in_place`
      rebuild.
- [x] Add the old-buffer (incl. bucket region) free to both `lower_map_set_in_place`
      grow paths.
- [x] Add the old-block free to `NirOp::StoreGlobal` for a freeable-flat global.

Acceptance: Phase 1 tests show bounded arena footprint; no double-free under a leak
checker; the append/bulk-append paths are byte-identical.

### Phase 3 — validation

- [x] Regenerate codegen goldens (no existing native golden exercises these paths —
      the 21 native-golden dirs are basic control-flow/project-entry tests; zero
      shift). New behavioral test carries no golden dir.
- [ ] `scripts/artifact-gate.sh`, then `scripts/test-accept.sh` (run by the
      orchestrator).

## Validation Plan

- Regression test(s): the three loop programs above, asserting bounded arena growth.
- Runtime proof: run each loop for a large N and confirm RSS/arena usage plateaus.
- Doc sync: none expected.
- Full suite: `scripts/artifact-gate.sh`, `scripts/test-accept.sh`.

## Summary

Three in-place mutators that grow their buffer but never free the old one — the exact
bug-01 pattern, re-introduced at sites the append fix didn't cover. The fix is a
faithful port of `append`'s free sequence to each grow path; the only risk is the
spill discipline and avoiding a value-shared double-free, both already solved in the
template. No behavior changes beyond memory reclamation.

## Resolution

Fixed. Every in-place mutator that abandons a backing buffer on grow now frees it,
matching the `append`/`bulk_append` template, and `StoreGlobal` frees a freeable-flat
global's previous block before overwriting it.

### Files changed

- `src/target/shared/code/builder_collection_mutate.rs`
  - New private helper `emit_free_pre_grow_buffer(slot, type_)`: frees the buffer held
    in `slot` before an in-place grow installs a fresh block over it, delegating sizing
    (incl. a map's `capacity << 4` hash-bucket region) and the block-pointer spill
    across `arena_free` to the existing `free_intermediate_collection`. Correct for both
    lists and maps — a hand-rolled port of `append`'s list-only free sequence would have
    under-sized a map free by the bucket region (the bug-02 arena-corruption class), so
    the helper deliberately routes through `emit_flat_block_size`.
  - `lower_list_prepend_in_place` grow: `emit_free_pre_grow_buffer(buffer_slot, ...)`
    before the install.
  - `lower_map_set_in_place` value-grow (`vgrow`) and capacity-grow (`grow`): same call
    on `map_slot` before each install.
  - `lower_list_set_in_place` rebuild branch: snapshots the original buffer pointer into
    `orig_slot` at the top of the branch, then after `rebuilt` is installed frees the
    `singleton`, the `removeAt` intermediate, and the original buffer — all three via
    `free_intermediate_collection` (the same helper the non-in-place `lower_collection_set`
    uses for the first two). All three are distinct blocks from `rebuilt`, so no
    double-free.
- `src/target/shared/code/builder_control.rs`
  - `NirOp::StoreGlobal`: when the global's type `is_freeable_flat_value`, snapshot the
    current global pointer, spill the freshly computed new value, `emit_owned_value_drop`
    the old block (null-guarded → first store over a zero-initialized global is a no-op),
    then re-derive the global address (its base `x19`/arena-state is callee-saved and
    survives `arena_free`) and store. Mirrors the local `NirOp::Assign` old-block free.
- `tests/rt-behavior/codegen/inplace-grow-free-bug47/` (new): behavioral (`mfb test`)
  regression guard exercising all five grow paths across many geometric grows and
  asserting result correctness — catches any double-free / use-after-free / bucket-region
  under-size the fix could introduce (all 5 cases pass).

### Why the free is safe at each site (no double-free / UAF)

- Every in-place mutator fires only under unique ownership and never on a live `FOR EACH`
  iterable (`try_inplace_*` guards in `builder_inplace_assign.rs`), so the abandoned block
  has no other reference. `lower_list_remove_at` allocates a fresh result and never frees
  its input, so freeing the original in the rebuild branch is once-only. The install
  overwrites the slot with the new/rebuilt buffer, which the binding's scope-drop later
  frees — a different block. For `StoreGlobal`, `lower_value_owned` deep-copies aliasing
  sources so the new global block never aliases the freed one.

### Runtime leak proof (out-of-tree RSS, macOS/aarch64)

Fresh-inode copies of each freshly built binary were measured with `/usr/bin/time -l`
(in-place overwrite of a just-run binary trips macOS's code-signature cache → spurious
"Killed: 9"; copying to a new path avoids it). Fix toggled off/on via a temporary
`BUG47_DISABLE` env gate (since removed):

| Grow path                    | N      | RSS before (leak) | RSS after (fix) |
|------------------------------|--------|-------------------|-----------------|
| `prepend` grow               | 40 000 | 33.8 MB           | 1.06 MB         |
| map `set` capacity-grow      | 40 000 | 33.9 MB           | 1.03 MB         |
| map `set` value-grow         | 40 000 | 19.3 MB           | 1.08 MB         |
| list `set` rebuild-grow      | 10 000 | 18.0 MB           | 1.03 MB         |
| `StoreGlobal` reassignment   | 40 000 | 42.0 MB           | 1.06 MB         |

After the fix RSS is flat across N (prepend measured at 40 k / 300 k / 3 000 000 all
≈ 1 MB); before, RSS grows linearly with iteration count and the loops time out at
larger N. All programs produce identical, correct output in both configurations (the
leak never corrupted results — the in-tree test guards correctness/double-free; this
table is the leak proof).

### Test results

- `mfb test tests/rt-behavior/codegen/inplace-grow-free-bug47` → 5 pass / 0 fail.
- `cargo test --bin mfb` → 2441 passed, 0 failed.
- No existing native golden (`.ncode/.nir/.nplan/.mir`) shifts: the fix is native-lowering
  only (AST/IR unchanged) and none of the 21 native-golden fixtures exercise a collection
  mutator or a freeable-flat global. `scripts/artifact-gate.sh` / `scripts/test-accept.sh`
  left to the orchestrator.
