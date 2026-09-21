# plan-142-D: In-place arms for `union`, `intersection`, `difference`, `symmetricDifference`, `merge`, `mapValues`

Last updated: 2026-09-20
Effort: large (3h–1d)
Depends on: plan-142-C

Prerequisites: see plan-142-A.

The four `Set` algebra operations and the two `Map` operations with a self-update
form get in-place arms. After D, `s = collections::union(s, t)`,
`s = collections::intersection(s, t)`, `s = collections::difference(s, t)`,
`s = collections::symmetricDifference(s, t)`, `m = collections::merge(m, n, p)` and
`m = collections::mapValues(m, f)` (when `U = V`) allocate nothing at S1 beyond
the map's own geometric growth.

References: plan-142-A §3; `lower_map_set_in_place` (`map/map_mutate.rs:188`),
`lower_map_remove_key_in_place` (`:1154`, compacts entries and clears
`BUCKETS_READY`); plan-121-E (set-algebra work asymmetry).

## 1. Goal

- 6 rows flip to `Arm`; 6 `cases.tsv` lines flip to `arm`; matrix and harness pass.
- `s = union(s, s)` (self-alias) and `m = merge(m, m, p)` are correct (the second
  operand is read in full before any write, or the arm declines — decided in
  Phase 1 and recorded).
- Atomicity: `mapValues` with a failing `f` leaves `m` unchanged.

### Non-goals

- Iteration order of the result stays what it is today (insertion order, per
  `mfb man collections`' "keep a set's insertion order").
- `mapValues` with `U ≠ V` untouched (not a self-update).

## 2. Current State

| fn | lowering today |
|---|---|
| `union`, `intersection`, `difference`, `symmetricDifference` | `Body::Mfb` source generics (`func_union.rs:96` etc.): `FOR EACH … result = add(result, x)` loops over a copy |
| `merge` | `Body::mfb_with_fast_path`; `lower_collection_merge_call` (`func_merge.rs:172`) for String key, fixed-width `V`, constant `preferB = TRUE` |
| `mapValues` | `Body::mfb_with_fast_path`; `lower_collection_map_values_call` (`func_map_values.rs:43`) for `V = U`, 8-byte fixed-width only |

## 3. Design

- **`union` / `merge`**: for each entry of the second operand, `lower_map_set_in_place`
  on `x` (`merge` honours `preferB`: with `FALSE`, skip keys already present).
- **`intersection` / `difference`**: one pass over `x` marks entries to remove
  (membership test in the other operand), then one compaction of `x`'s entry table
  (a map form of B's `lower_list_compact_in_place`), clearing `BUCKETS_READY`
  once. Calling `lower_map_remove_key_in_place` per key would be O(n²).
- **`symmetricDifference`**: mark-and-compact the shared keys out of `x`, then add
  the other operand's keys that were not shared.
- **`mapValues` (`U = V`)**: overwrite each value in place (fixed-width); for a
  variable-width `V`, the map's value slot takes the same resize path as
  `lower_map_set_in_place`. Fallible `f`: plan-142-A §3 / Open Decision 1.
- **Self-alias** (`union(s, s)`, `merge(m, m, p)`): the result equals `x`; the arm
  recognises the alias (G12-style: the second operand is the same binding) and
  emits nothing but the evaluation of `p`.

## Phases

### Phase 1 — Map compaction primitive + alias rule

- [x] `lower_map_compact_in_place(map_slot, keep_bitmap_slot, map_type)` in
      `map_mutate.rs` (entries shift down, `count` updated, `BUCKETS_READY := 0`,
      dropped key/value graphs freed) + unit test.
      In its own file, `src/codegen/collection/map/map_compact.rs`, beside two
      primitives the arms also needed: `lower_map_clear_in_place` (the self-alias
      `difference`/`symmetricDifference` result) and `emit_map_reserve` (one
      geometric grow before a batch of inserts — Correction D2). As for
      `lower_map_remove_key_in_place`, dropped payload bytes stay behind in the data
      region; keys own no graphs. Unit test:
      `map_compact_in_place_marks_then_compacts_and_union_reserves_once`
      (`src/codegen/builtins/tests/inplace_compact.rs`).
- [x] Decide and record the self-alias handling (recommended above).
      Decided: recognised syntactically (the second operand is the same local) and
      lowered as its value — `union(s, s)`, `intersection(s, s)` and
      `merge(m, m, p)` emit nothing but `p`'s evaluation; `difference(s, s)` and
      `symmetricDifference(s, s)` empty `s` in place (`lower_map_clear_in_place`).
      Runtime cases in the differential probe below.

Acceptance: `cargo test --bin mfb map_compact_in_place` → pass (est. 3 min).
Verified 2026-09-21: `test result: ok. 1 passed; 0 failed`.
Commit: —

### Phase 2 — The four set operations

- [x] Arms for `union`, `intersection`, `difference`, `symmetricDifference`
      (matched through `self_update_builtin`), rows, `cases.tsv`.
      `src/codegen/collection/assign/builder_inplace_setmap.rs`, each reproducing its
      copying body's operations and order (module doc). Membership uses
      `emit_key_membership`, the probe `contains` uses on a `Set`. A `String` key read
      from a set is rebuilt in the self-update scratch instead of allocated
      (`emit_map_entry_borrowed`, Correction D3).
- [x] Runtime cases for the self-alias forms and for insertion order after each op.
      Differential probe (each op in place vs the copying call, printed in
      iteration order): the four ops on `Set OF Integer` and `Set OF String`, all
      five self-alias forms, `symmetricDifference` then `union`, `difference(s, s)`
      then `union`, and a 300-iteration union/difference/intersection loop — 23/23
      `ok` with the `merge`/`mapValues` cases below, `arena.0.alloc_calls 5553` =
      `free_calls 5553`, `live_bytes 0`.

Acceptance: `cargo test --bin mfb self_update && cargo test --test rt_inplace_self_update`
→ pass (est. 10 min).
Verified 2026-09-21: `self_update` → `4 passed`; `rt_inplace_self_update` filtered to the
six D lines → `1 passed` (bound, `before` and result checks).
Commit: —

### Phase 3 — `merge` and `mapValues`

- [x] Arms, rows, `cases.tsv`; `mapValues` failing-`f` case in `rt_inplace_failure_atomic`.
      `merge` honours `preferB` per entry exactly as the body's
      `IF preferB OR NOT hasKey(result, e.key)`; `mapValues` calls `f` for every value
      first and writes each result into its own entry by index
      (`emit_map_entry_value_write`), so no key is read. Probe cases: `merge` with
      `preferB` TRUE/FALSE and self-alias, `mapValues` growing/shrinking/returning
      its argument over `String` values and over `Integer` values, and a 300-iteration
      merge/mapValues loop — all `ok` (counts above). The failing-`f` case uses
      `String` values that grow.

Acceptance: `cargo test --bin mfb self_update && cargo test --test rt_inplace_self_update --test rt_inplace_failure_atomic`
→ pass (est. 10 min).
Verified 2026-09-21: `self_update` → `4 passed`; `rt_inplace_self_update` (D lines) → `1
passed`; `rt_inplace_failure_atomic` → `1 passed`.
Commit: —

### Phase 4 — Expected outputs

- [x] Measured: `rg -lP "(\w+) = collections::(union|intersection|difference|symmetricDifference|merge|mapValues)\(\1\b" tests examples --glob '*.mfb'`
      → 0 files, so no golden is expected to shift. Run
      `scripts/test-accept.sh target/debug/mfb target/accept-actual` on
      `tests/rt-behavior/collections` to confirm.

Acceptance: acceptance run green on that directory (est. 10 min).
Verified 2026-09-21: the `rg` count re-run → `0`; `scripts/test-accept.sh <debug mfb> <dir>
'rt-behavior/collections/*'` → `acceptance tests passed (65 test(s) ran)`; full
artifact-gate with every D arm → `2078 golden(s) checked, 0 diff(s)`.
Commit: —

## Validation Plan

- Tests: harness rows, atomicity case, self-alias and order runtime cases,
  compaction unit test.

## Corrections

- **D1 (Phase 3): a pre-existing leak in the copying `merge`, fixed first.** The
  differential probe's copying `merge(ma(), mb(), TRUE)` calls leaked: the native
  fast path (`String` keys, constant `preferB = TRUE`) rebuilds each of `b`'s keys
  and `String` values as a block for `lower_map_set_in_place` and never freed them
  (4 blocks / 80 B for two entries). Fixed in its own commit (`310aec4bc`, with
  `tests/runtime/rt_merge_fast_path_frees.rs`, RED on both value kinds before).
- **D2 (Phase 2): batches of inserts reserve first.** §3 had `union`/`merge` loop
  `lower_map_set_in_place`, which grows the map whenever it is full — so an
  `ErrOutOfMemory` could strike after some inserts and leave `x` half-updated,
  breaking plan-142-A's failure-atomicity rule. `emit_map_reserve` sizes the map
  once for the whole batch (one entry and `keyLength + valueLength + 16` bytes per
  inserted entry), so the only allocation precedes the first write.
- **D3 (Phase 3): no per-element allocation.** The first harness run failed `merge`
  and `mapValues` as `arm`: "2000 more runs allocated 4000 more blocks" — each
  statement materialized each `String` key (to probe and insert with it, and in
  `mapValues` to find the entry it had just read). Keys and values read from a map
  are now rebuilt as `[length][bytes][NUL]` in the self-update scratch
  (`emit_map_entry_borrowed`, one reused area per payload kind, sized by a pre-pass
  over the source's longest payload), and `mapValues` writes by entry index. The
  set arms use the same borrowed keys, so no D arm allocates per element.

## Summary

The map compaction primitive is the only new low-level code; everything else loops
over primitives that already exist.
