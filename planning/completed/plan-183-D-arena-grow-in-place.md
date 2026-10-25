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

- [x] Added (Corrections 1): flip the span — huge grows up from the span base, medium down from the slice top — so the newest huge chunk ends at a frontier it can grow into. `layout.rs` geometry, `span.rs` frontier paths and region bounds, `ARENA_SC_MEDIUM_BASE` → `ARENA_SC_SPAN_BASE`, the debugger's region view (regions now 0 small, 1 huge, 2 medium; `mfb man debug pools` regenerated from a real `mfb dbg` session), `04_arenas.md`, `10_debugger.md`, `.ai/codegen-invariants.md`. `cargo test --test rt_arena_regions --test rt_arena_zeroing --test rt_arena_large_block_reuse --test rt_debug_arena --test rt_worker_arena_release --test rt_arena_memory_cap` → 20 passed; `cargo test --bin mfb debugger::` → 311 passed.
- [x] `mod.rs`: `lower_arena_extend`, with an `extend_calls`/`extend_hits` counter bump. In its own `extend.rs` (`_mfb_arena_extend(ptr, old, new)` → 1/0): small within its slot; medium into the free run above; huge into the run above and on into the huge frontier; a medium chunk asked for 32 MiB or more refuses (the tier crossing). Emitted in every module beside the other arena helpers; `04_arenas.md` *`arena_extend`*.
- [x] `collection_buffer.rs:emit_geometric_step` and its callers: extend first; exact capacity on success; the tier-crossing move. One helper, `buffer/extend_in_place.rs:emit_list_try_extend`, called before the allocation of every in-place grow — `append`, bulk and self-slice `append`, inline-field `append` and bulk `append`, `insert`/`prepend`, `repeat`, map `set` (value grow and capacity grow), map reserve — and a string self-append (`emit_string_regrow`, joined so `fill` is emitted once). On success the header takes the capacities the grow computed, a table that grew moves the data up (overlapping, highest byte first) and zeroes the uncovered bytes, and control skips copy, free and install. The tier crossing is the medium refusal above: the fallback allocation lands in huge. The `set` repack stays allocate-copy-free — Corrections 4. `declared_errors_all` classifies `_mfb_arena_extend` and `_mfb_arena_fill_zero` for `collections`. A probe exercising every arm (`collections::set` on a 100,000-entry map, string self-append, insert at front and middle, `repeat`, inline record field append) printed the same output as main's compiler, with `MFB_ARENA_CHECK_ZERO`: `zero_violations 0`, 36 of 73 extends hit.
- [x] `05_collections.md` growth section; `.ai/collections.md` note.
- [x] Test: new `tests/runtime/rt_arena_grow_in_place.rs` — append 41 records, each holding a 1.6 MiB `List OF Float`, one at a time. Assert `peak_live ≤ final_live + one record + 1 MiB` and `extend_hits ≥ 30`. Both bounds corrected (Corrections 2): one tier crossing, ≥ 1 extend, `peak ≤ final + two records + 1 MiB` (measured final 82,076,848 B, peak 85,276,960 B; before D the peak was 138,394,832 B). Plus the huge-tier case (Corrections 3): one `List OF Float` appended to 2^25 elements — one crossing, ≥ 5 extend hits, `peak ≤ final + 4 MiB` (measured peak 310,282,496 B = the final capacity; before D 517,137,296 B).

- [x] Added (from Validation): the goldens — full gate, full `test-accept.sh` with no `.run`/`build.log` change, `regen-native-goldens.sh`, gate back to 0. `scripts/artifact-gate.sh target/release/mfb all` → 166 diffs, all `.ncode`/`.mir` (`parser_hello_world` MIR: `_mfb_arena_extend` added; `_geometry`, `_medium_alloc`, `_huge_alloc`, `_span_free` changed; `_mfb_rt_park_error` vreg numbering only); `scripts/test-accept.sh target/release/mfb` → 1756 ran, 9 mismatches, all native dumps; `regen-native-goldens.sh` → 230 rewritten, 0 failures; gate → 0 diffs over 2384 goldens.

Acceptance: `cargo test --test rt_arena_grow_in_place --test inplace_self_update_census` passes on macOS, 2223 and the Windows box (est. 3 min each).
Measured 2026-10-24: macOS 2 binaries, 5 passed; with `rt_arena_regions` and `rt_arena_zeroing` added (the flip moves both regions), box 2226 (for 2223) 4 binaries 9 passed, Windows box 9 passed, box 2229 (riscv64) 9 passed, box 2227 (x86_64) 9 passed.
Commit: 974e566a3 (goldens in the commit after it)

## Validation

- `bash scripts/test-fast.sh`.
- Goldens: every collection-growing fixture moves. Run the full artifact gate and regenerate; the full acceptance run must show no `.run`/`build.log` change.
- Doc sync: `05_collections.md`.

## Corrections

1. **The huge region could not extend in place as C laid it out, so D flips the span.** C carved huge chunks downward from the slice top, so the newest huge chunk — the one a growing collection lives in — had the frontier *below* it and the slice top or an older chunk above: it could only grow into a free run, which the copy-and-free pattern leaves above it about every other step, and each miss copied gigabytes with old plus new live. Huge now grows up from the span base and medium down from the slice top. A medium chunk is under 32 MiB, so the copies it cannot avoid leave less than 32 MiB extra live — what plan-183-E's `peak ≤ live + 32 MiB` bound already allows — and it crosses into huge once, where it then extends at every step. Weighed with the user against per-chunk headroom (uncommitted space reserved above each huge chunk: every huge chunk extends, but needs headroom sizing and a way to take it back) and three fixed upward sections (only the newest chunk extends, and a fixed split starves one tier at small slices); the flip needs no new state. The bit-55 headroom reservation added to C's tag format was dropped with it.
2. **`extend_hits ≥ 30` and "one record" were miscalibrated for this design and this program.** Below 32 MiB the list of records copies by design, so only its growth steps after the crossing extend (2 here: the list ends at 82 MB). And `day()` holds its 1.6 MB list and the record built around it at once, so the peak is the final size plus *two* records (3,200,112 B over final). That excess is the program's, not the allocator's. The test asserts what D guarantees instead: one crossing (`hit_huge_frontier` 1), at least one extend, and `peak ≤ final + 2 records + 1 MiB` — which the pre-D allocator failed by 53 MB (peak 138,394,832 B).
3. **A huge-tier case was added** (suggested by the user): one list from nothing to 256 MiB, where every step after the crossing must extend and the peak must be the final capacity within one commit step.
4. **The variable-width `set` repack (`emit_repack_list_data`) keeps allocate-copy-free.** Its copy is the compaction itself, done in entry order; after a replaced value has gone to the data tail, entry order is not data order, so compacting in place would overwrite payloads not yet moved. `copy_list_with_append_headroom` is not a grow of an existing block (its source lives on) and is unchanged.
5. **"Exact capacity" means the capacity the grow computed.** On success the block takes exactly the `capacity`/`dataCapacity` the allocate path would have — the geometric step — not `count + 1`: extending by one slot per append would shift a variable-width list's data region on every append (O(n) each), since the data base follows the capacity.
6. **`arena_extend` answers 1/0 and never moves the chunk**, as designed, which lets every site jump to its post-install point with the block slot untouched.
