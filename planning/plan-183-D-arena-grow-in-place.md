# plan-183-D: Arena — grow in place (`arena_extend`)

Last updated: 2026-10-24
Effort: medium (1h–2h)
Depends on: plan-183-C

## Goal

A collection whose buffer has free space after it grows without moving. There's no
copy and no old-plus-new peak, and its capacity becomes exactly what was requested.

### Non-goals

- Collection growth policy is unchanged when an extend fails.
- No change to value semantics.

## Current state (read 2026-10-24)

`src/codegen/collection/buffer/collection_buffer.rs:emit_geometric_step` allocates the
new capacity, copies and frees. Lookup slots double up to 1024, then grow ×1.5; data
doubles up to 64 KiB, then grows ×1.5 (`05_collections.md`). Weather's forecast list
peaked at old plus new, 1.78 GB.

## Design

- **The helper:** `arena_extend(ptr, old_size, new_size) → Boolean` (`mod.rs:lower_arena_extend`). It succeeds when the bytes after the chunk are free in the side bitmap, or are the region's uncommitted frontier (committing more on demand). It marks them allocated; they are already zero. The chunk doesn't move.
- **Collections:** `emit_geometric_step` tries `arena_extend` first for the exact requested capacity. On success it does nothing else. On failure it falls back to today's allocate-copy-free.
- **Tier crossing:** a medium chunk growing to 32 MiB or more is reallocated once into the huge region, then extends in place there.

## Phase D1 — `arena_extend` and collection growth

- [ ] `mod.rs`: `lower_arena_extend`, with an `extend_calls`/`extend_hits` counter bump.
- [ ] `collection_buffer.rs:emit_geometric_step` and its callers: extend first; exact capacity on success; the tier-crossing move.
- [ ] `05_collections.md` growth section; `.ai/collections.md` note.
- [ ] Test: new `tests/runtime/rt_arena_grow_in_place.rs` — append 41 records, each holding a 1.6 MiB `List OF Float`, one at a time. Assert `peak_live ≤ final_live + one record + 1 MiB` and `extend_hits ≥ 30`.

Acceptance: `cargo test --test rt_arena_grow_in_place --test inplace_self_update_census` passes on macOS, 2223 and the Windows box (est. 3 min each).
Commit: —

## Validation

- `bash scripts/test-fast.sh`.
- Goldens: every collection-growing fixture moves. Run the full artifact gate and regenerate; the full acceptance run must show no `.run`/`build.log` change.
- Doc sync: `05_collections.md`.

## Corrections
