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

- [x] `error_constants.rs`: the new layout; every offset is a constant. Small heads in the old quick-bin span, medium heads in the old large-bin span, the huge head and node free list in the old carve words, a 21-word scalar block at the end (`ARENA_SC_*`, addressed off a computed register); `ARENA_STATE_SIZE` 3936. See Corrections 2.
- [x] `alloc.rs`, `free.rs`, `mod.rs`: regions, page table, side bitmaps, out-of-line free runs; FreeNode removed; whole-chunk zero-on-free. New `layout.rs` (geometry, `_mfb_arena_commit`), `small.rs` (pages, descriptors, slot bitmaps), `span.rs` (medium/huge runs: tag records and a node pool — Corrections 1). Old quick bins, carve chunk, free list, large bins, flush and insert helpers deleted.
- [x] `arena_cap/accounting.rs`: charge on commit, refund on decommit. `emit_commit_charge`/`emit_commit_refund`/`emit_decommit_refund`, counter `_mfb_rt_arena_committed_bytes`; `rt_arena_memory_cap` 3 passed.
- [x] Block headers removed: `arena_destroy`, `_mfb_rt_thread_finish`, `emit_unmap_refund`. `arena_destroy` decommits the whole slice and refunds the arena's `committed` word; `_mfb_rt_thread_finish` reaches it through `arena_destroy`; `emit_unmap_refund` became `emit_decommit_refund`.
- [x] `04_arenas.md`: state layout, regions, algorithms, *Process Memory Cap*; the block section removed. Also `09_debug-report.md`, `08_program-startup.md`, `01_project-manifest.md`, threading 02/07/08, `mfb man debug report`/`pools`, `.ai/codegen-invariants.md`. `cargo test --bin mfb spec` → 72 passed.
- [x] Tests:
  - `rt_arena_zeroing.rs` tightened: every byte of a freed chunk is zero, including the first 16; (`ARENA_ZERO_CHECK_SKIP` is 0, so the switch checks every byte)
  - new `tests/runtime/rt_arena_regions.rs`:
    - 1,000 small allocations interleaved with huge ones; free the huge ones and assert one merged free run;
    - a 100,000-operation random alloc/free stress across all tiers, with a shadow model asserting no live chunk is ever handed out twice and every freed range reads zero. (The shadow model is in the program: each slot's string is checked for its length and letter before it is replaced, so a chunk handed to a second owner fails the check; `MFB_ARENA_CHECK_ZERO` checks every chunk handed out. 18 huge, ~30,000 medium and ~70,000 small operations: `bad 0`, `zero_violations 0`, `live_bytes 0`.)

Acceptance: `cargo test --test rt_arena_regions --test rt_arena_zeroing --test rt_arena_large_block_reuse --test rt_worker_arena_release` passes on macOS, 2223, 2229 (riscv64, for the offset reach) and the Windows box (est. 6 min each; 2229 longer).
Measured 2026-10-24: macOS 2+2+3+2 passed; box 2229 (`MFB_TEST_REMOTE=linux-riscv64,test@127.0.0.1,2229,musl`) 2+2+3+2 passed; box 2226 standing in for 2223 (`linux-aarch64,…,2226,glibc`) 2+2+3+2 passed; Windows box 2+2+3+2 passed. (`rt_worker_arena_release` reads peak RSS from the `--debug` report on a remote box.)
Commit: f7ae0e904 (goldens in the commit after it)

## Phase C2 — Debugger and specs that read the layout

- [x] `src/debugger/runtime.rs`: arena views show regions, frontiers and free runs instead of blocks and bins; tests in `src/debugger/runtime/tests` and `src/debugger/prompt/tests`. `runtime::areas`/`free_runs`, `render::area_lines`, `show arena <id> <region>` dumps, `list arena`'s `Committed (KB)`, the no-symbol main-arena test by slice base; `ARENA_STATE_FIELDS` tiles the new state.
- [x] `src/docs/spec/tooling/10_debugger.md` (arena output), `07_control-block.md` and `09_debug-report.md` where they cite the layout. Plus `mfb man debug pools`, regenerated from a real session (its "random bytes" text was stale since plan-183-A's zero fill).

Acceptance: `cargo test --bin mfb debugger::` passes (est. 1 min); on macOS, `mfb dbg` of a `--debug -O0` build shows the region view.
Measured: `cargo test --bin mfb debugger::` → 311 passed. `printf 'bp pause\nrun\nlist arena\nshow arena 0\nq\n' | mfb dbg /tmp/p183/dbgprog/build/dbgprog.out` (signed with `scripts/sign-debugger.sh`) printed `Arena Region (0 small)` … `(2 huge)` with their frontiers and committed bytes, and `Free Runs: 1` / `medium[1] 0x00000070b1803310 0x0000000000000bd0`.
Commit: f7ae0e904 (goldens in the commit after it)

## Validation

- `bash scripts/test-fast.sh`.
- Goldens: every binary moves. Run the full artifact gate and regenerate; the full acceptance run must show no `.run`/`build.log` change before re-baselining.
- Doc sync as listed in the phases.

## Corrections

1. **Medium and huge use tag records and a node pool, not a granule bitmap scanned for neighbours.** Finding a free neighbour's start by scanning an allocated-bit bitmap is O(run length) — a 1 GiB free run is a million words. Instead one 16-byte record per 2 KiB block of the span holds `{start, end}`: what starts in the block (an allocated chunk's size and granule offset, or a free run's node) and which free run ends in it. Every medium/huge chunk and run exceeds 2 KiB (a split rest below 2064 bytes, or below one 16 KiB huge page, stays in its chunk), so at most one thing starts and one run ends per block, and a free finds its size and both neighbours in O(1), each checked against the run's node. The start record also gives the chunk's true size and refuses a pointer that is not a chunk start (a repeated free, a pointer into a chunk). Medium and huge share the one tag array and node pool, and runs are listed by size class (medium) or on one list (huge), best fit, as designed. Side cost: 32 bytes per 2 KiB of span, committed as the frontiers advance.
2. **The allocator's scalar words are a block at the end of the state.** The old allocator span holds the 128 small heads, the 64 medium heads, the huge head and the node free list; the 21 scalar words (geometry, three frontiers and their committed edges, tag/node commit edges, committed bytes) do not fit there. They sit past rv64's 12-bit immediate, so every helper computes their base into a register once — the "computed address" route the plan allows. Words +0, +24, +48 are reserved. `ARENA_STATE_SIZE` grew from 3776 to 3936.
3. **The report's rows changed in C, not E.** Removing blocks and bins removed `maps`/`mapped_bytes`/`unmaps`/`unmapped_bytes`, the `hit_quick_bin`…`grow` paths, `flushes` and `insert_free_calls`. Their replacements are the rows plan-183-E lists — `reserved_bytes`, `commits`, `committed_bytes`, `committed_peak_bytes`, `decommits`, `decommitted_bytes`, `extend_calls`, `extend_hits` — plus `hit_small`, `small_pages`, `hit_medium_run`, `hit_medium_frontier`, `hit_huge_run`, `hit_huge_frontier`, and `grows`. **A grow is now a frontier extension** (a never-used small page, a medium or huge chunk carved at its frontier), and the memory series samples on it: commits come in 4 MiB steps, so sampling per commit could not exercise the series' halving without gigabytes. `rt_debug_arena`, `rt_thread_block_arena` and `rt_arena_zeroing` assert the same properties in the new rows; the series' `committed_bytes` only rises, so "strictly increasing" became "never falls, and rises overall".
4. **`rt_arena_large_block_reuse`'s ×4 bound moved to E's committed bound in C.** Committed bytes carry a per-region floor (a 4 MiB data step per region, 64 KiB side granules) that mapped bytes never had: the climbing case committed 5,963,776 bytes for a 1,364,672-byte peak (4.37×) with every allocation reused. The bound plan-183-E specifies for committed bytes, `≤ 1.25 × peak + 64 MiB`, is in force from C.
5. **Alignment above 16 is served by the huge region** (16 KiB-aligned chunks); above 16 KiB it is `ErrInvalidArgument`. Every one of the 280 `arena_alloc` call sites passes 1, 2 or 8 (survey of `src/`), so no code path needs more.
6. **A small page that empties stays on its class list when it is the class's only page.** Moving every empty page to the empty list made a class that frees and allocates one slot at a time re-initialise a page each time: the zeroing smoke program set up 121 pages for 201 small allocations; with the rule, 2.
7. **The Rust reference model of the old free list was deleted** (`engine/tests/tests.rs`: `FreeListSim`, `checked_arena_used_after_alloc` and the nine tests on them). It modelled the address-ordered free list and block payloads C removed and never exercised emitted code; the region allocator is exercised by `rt_arena_regions`' stress test.
8. **plan-183-B and plan-183-C landed in one commit.** C rewrote B's interim grow and destroy code before B's goldens were regenerated, so the tree held both. B's acceptance rows were measured on the B-only binary and re-run on the combined one (macOS, 2226, Windows). The goldens for both were regenerated once, on the combined binary (plan-183-B's golden task carries the gate, acceptance and regen output).
9. **Box 2223 was down**; 2226 stands in for it, and 2229 ran the riscv64 row.
10. **The `tests/acceptance` TESTING app times out under a debug-built `mfb`** in the full acceptance run (300 s watchdog; `scripts/test-accept.sh` documents 339 s with a debug compiler). The golden run uses the release build.
