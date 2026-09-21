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

- [ ] `lower_map_compact_in_place(map_slot, keep_bitmap_slot, map_type)` in
      `map_mutate.rs` (entries shift down, `count` updated, `BUCKETS_READY := 0`,
      dropped key/value graphs freed) + unit test.
- [ ] Decide and record the self-alias handling (recommended above).

Acceptance: `cargo test --bin mfb map_compact_in_place` → pass (est. 3 min).
Commit: —

### Phase 2 — The four set operations

- [ ] Arms for `union`, `intersection`, `difference`, `symmetricDifference`
      (matched through `self_update_builtin`), rows, `cases.tsv`.
- [ ] Runtime cases for the self-alias forms and for insertion order after each op.

Acceptance: `cargo test --bin mfb self_update && cargo test --test rt_inplace_self_update`
→ pass (est. 10 min).
Commit: —

### Phase 3 — `merge` and `mapValues`

- [ ] Arms, rows, `cases.tsv`; `mapValues` failing-`f` case in `rt_inplace_failure_atomic`.

Acceptance: `cargo test --bin mfb self_update && cargo test --test rt_inplace_self_update --test rt_inplace_failure_atomic`
→ pass (est. 10 min).
Commit: —

### Phase 4 — Expected outputs

- [ ] Measured: `rg -lP "(\w+) = collections::(union|intersection|difference|symmetricDifference|merge|mapValues)\(\1\b" tests examples --glob '*.mfb'`
      → 0 files, so no golden is expected to shift. Run
      `scripts/test-accept.sh target/debug/mfb target/accept-actual` on
      `tests/rt-behavior/collections` to confirm.

Acceptance: acceptance run green on that directory (est. 10 min).
Commit: —

## Validation Plan

- Tests: harness rows, atomicity case, self-alias and order runtime cases,
  compaction unit test.

## Corrections

## Summary

The map compaction primitive is the only new low-level code; everything else loops
over primitives that already exist.
