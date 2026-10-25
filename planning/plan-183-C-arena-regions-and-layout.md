# plan-183-C: Arena — three regions, side bitmaps, new arena-state layout

Last updated: 2026-10-24
Effort: large (3h–1d)
Depends on: plan-183-B

## Goal

Inside each slice the allocator is replaced with three regions. Free chunks merge with
their neighbours across the whole region, and no allocator metadata lives in free
memory, so every free byte is zero. Block headers are gone. The cap counts committed
bytes.

### Non-goals

- `arena_alloc(size, align)` and `arena_free(ptr, size)` keep their contracts.
- No in-place growth (D) and no decommit of free runs (E).

## Current state (read 2026-10-24)

**Allocator:**
- `free.rs:lower_arena_free` writes a 16-byte `{next, size}` FreeNode into each freed chunk, which breaks the zero rule.
- Merging happens only within one block (`lower_arena_flush_coalesce`).
- Bins: 128 quick bins (16 B–2 KiB) and 64 large-bin classes (`04_arenas.md`).

**Block headers** (32 bytes: next link, `blockSize` +8, `usableCapacity` +16) are read by:
- `arena_destroy`;
- `arena_cap/accounting.rs:emit_unmap_refund`;
- `src/debugger/runtime.rs`.

**Arena state:**
- `ARENA_STATE_SIZE` = 3,776 bytes (`src/codegen/error/constants/error_constants.rs`). Allocator fields: `blockHead` +0, reserved +16/+24, `freeListHead` +48, `quickBin[128]` +104, `carvePtr`/`carveSize` +1128, `largeBin[64]` +1168, `largeFlushCredit` +3768.
- Non-allocator fields with codegen constraints: `stackLimit` +1680 (one load on every target, bug-789); `v128Slots` +1696, `flagRhs` +3712 and `currentError` +3728 (riscv64 12-bit `addi` reach, or a computed address as `currentError` already uses).
- `git grep -l ARENA_STATE_SIZE -- src | wc -l` → 13 files: the entry shim and thread-spawn zeroing, the debugger (imports `ARENA_*_OFFSET` constants by name in `src/debugger/runtime.rs`), and specs.

**Cap:** `_mfb_rt_arena_mapped_bytes` is charged per mapped block (`08_program-startup.md`, `04_arenas.md` *Process Memory Cap*).

## Design

### Regions

```
| small: sliceSize/8 | medium → grows up        uncommitted        grows down ← huge |
```

- **Small (≤ 2 KiB after normalization):** 64 KiB pages, each serving one of the 128 size classes. A page descriptor table at the start of the small region holds each page's class and a free-slot bitmap. Per-class lists link pages that have free slots. Allocate and free are O(1).
- **Medium (2 KiB < size < 32 MiB):**
  - A side granule bitmap at the start of the medium region, committed with it: 1 bit per 16-byte granule = allocated, and a parallel chunk-start bit.
  - Free runs are indexed by the 64 existing size classes, in out-of-line node arrays in the side space. Best fit.
  - Freeing merges with free neighbours immediately, found by bitmap scan.
- **Huge (≥ 32 MiB):** page-aligned, carved downward from the slice top, with the same bitmap scheme at 16 KiB page granularity.
- **Collision:** medium and huge meeting, or the small region filling, raises `ErrOutOfMemory`.
- **Commit:** in 4 MiB steps per region. Fresh pages are zero (A).
- **No metadata in free memory:** FreeNode overlays are removed, and every byte of a free chunk is zero. Zero-on-free (A's routine) covers the whole chunk.

### Arena-state layout (owner: may change as needed)

- **Allocator fields come first,** in the old allocator span:
  - slice index;
  - three commit frontiers;
  - the small page-table base;
  - the medium and huge bitmap bases;
  - 128 small-class page-list heads;
  - 64 medium-class free-run heads;
  - huge free-run head;
  - committed, decommitted and extend counters.
- `ARENA_STATE_SIZE` is recomputed from the last field and grows at the end if they don't fit.
- **Non-allocator fields may move,** but only through their constants in `error_constants.rs`, never literal offsets.
- **The constraints still hold:**
  - `stackLimit` is one load on every target;
  - `v128Slots`, `flagRhs` and `currentError` stay within riscv64 12-bit reach, or riscv64 switches to a computed address for any that leaves it.

### Block headers removed

There are no blocks:
- `arena_destroy` and `_mfb_rt_thread_finish` decommit the slice, using B's decommit.
- The cap refund comes from the region commit counters.

### Cap counts committed bytes

`_mfb_rt_arena_mapped_bytes` becomes `_mfb_rt_arena_committed_bytes`, with the same lock and rules. It's charged before each commit step and refunded on every decommit.

## Phase C1 — Layout and regions

- [ ] `error_constants.rs`: the new layout; every offset is a constant.
- [ ] `alloc.rs`, `free.rs`, `mod.rs`: regions, page table, side bitmaps, out-of-line free runs; FreeNode removed; whole-chunk zero-on-free.
- [ ] `arena_cap/accounting.rs`: charge on commit, refund on decommit.
- [ ] Block headers removed: `arena_destroy`, `_mfb_rt_thread_finish`, `emit_unmap_refund`.
- [ ] `04_arenas.md`: state layout, regions, algorithms, *Process Memory Cap*; the block section removed.
- [ ] Tests:
  - `rt_arena_zeroing.rs` tightened: every byte of a freed chunk is zero, including the first 16;
  - new `tests/runtime/rt_arena_regions.rs`:
    - 1,000 small allocations interleaved with huge ones; free the huge ones and assert one merged free run;
    - a 100,000-operation random alloc/free stress across all tiers, with a shadow model asserting no live chunk is ever handed out twice and every freed range reads zero.

Acceptance: `cargo test --test rt_arena_regions --test rt_arena_zeroing --test rt_arena_large_block_reuse --test rt_worker_arena_release` passes on macOS, 2223, 2229 (riscv64, for the offset reach) and the Windows box (est. 6 min each; 2229 longer).
Commit: —

## Phase C2 — Debugger and specs that read the layout

- [ ] `src/debugger/runtime.rs`: arena views show regions, frontiers and free runs instead of blocks and bins; tests in `src/debugger/runtime/tests` and `src/debugger/prompt/tests`.
- [ ] `src/docs/spec/tooling/10_debugger.md` (arena output), `07_control-block.md` and `09_debug-report.md` where they cite the layout.

Acceptance: `cargo test --bin mfb debugger::` passes (est. 1 min); on macOS, `mfb dbg` of a `--debug -O0` build shows the region view.
Commit: —

## Validation

- `bash scripts/test-fast.sh`.
- Goldens: every binary moves. Run the full artifact gate and regenerate; the full acceptance run must show no `.run`/`build.log` change before re-baselining.
- Doc sync as listed in the phases.

## Corrections
