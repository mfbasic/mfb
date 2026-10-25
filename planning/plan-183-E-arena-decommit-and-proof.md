# plan-183-E: Arena — decommit long-free ranges, report rows, weather proof

Last updated: 2026-10-24
Effort: medium (1h–2h)
Depends on: plan-183-D

## Goal

Memory a program stops using goes back to the OS and returns zero when reused.
Weather meets plan-183's bounds:
- `arena.0.committed_bytes ≤ 1.25 × arena.0.peak_live_bytes + 64 MiB`
- `arena.0.peak_live_bytes ≤ arena.0.live_bytes(after load) + 32 MiB`
- `arena.0.decommitted_bytes > 0`
- `arena.0.fill_grow_bytes == 0`

### Non-goals

- The reservation is never unmapped before exit; only pages are decommitted.

## Current state (read 2026-10-24)

After C, free runs merge across each region, but committed pages stay committed until
the thread ends. `tests/runtime/rt_arena_large_block_reuse.rs` bounds mapped bytes by
`MAX_MAPPED_PER_PEAK_LIVE = 4`. The `--debug` report has `mapped_bytes` and `unmaps`
but no commit or decommit rows (`09_debug-report.md`).

## Design

- **Decommit at the end of each flush:** a page-aligned free run of 4 MiB or more in the medium or huge region that was also free at the previous flush is decommitted. Platform calls are B's: `madvise(MADV_DONTNEED)` on Linux, `mmap(MAP_FIXED, PROT_READ|PROT_WRITE)` over the range on macOS (guaranteed zero; `MADV_FREE_REUSABLE` is not), `VirtualFree(MEM_DECOMMIT)` on Windows. The cap is refunded.
- **Large frees:** a free of 4 MiB or more decommits the whole pages inside the chunk instead of zero-filling them, and fills only the partial edge pages.
- **Frontier retreat:** a free top of medium, or a free bottom of huge, moves the commit frontier back.
- **Small region:** fully free pages go to a per-arena empty-page list, which is decommitted down to 64 pages.
- **Report rows** per arena: `reserved_bytes`, `committed_bytes`, `committed_peak_bytes`, `decommitted_bytes`, `decommits`, `extend_calls`, `extend_hits`. Process rows: `arena.slices`, `arena.slice_size`, `arena.reservation_bytes`. `mapped_bytes` is retired.

## Phase E1 — Decommit, retreat, report

- [ ] `free.rs:lower_arena_flush_coalesce`: the two-flush decommit and frontier retreat; the small empty-page cap.
- [ ] `free.rs:lower_arena_free`: large-free page decommit with edge fill.
- [ ] `src/codegen/debug/arena/`: the report rows; `09_debug-report.md`.
- [ ] `rt_arena_large_block_reuse.rs`: replace the `MAX_MAPPED_PER_PEAK_LIVE = 4` bound with `committed ≤ 1.25 × peak_live + 64 MiB`.
- [ ] `rt_arena_zeroing.rs`: free a 64 MiB chunk, force a flush, reallocate and assert all zero (the decommit path).
- [ ] `planning/todo.md` "## Memory": close Spike 8, Spike 9, Bucket 9, Bucket 12a and A/B 7 with "plan-183".

Acceptance: `cargo test --test rt_arena_large_block_reuse --test rt_arena_zeroing --test rt_arena_regions` passes on macOS, 2223 and the Windows box (est. 4 min each).
Commit: —

## Validation (plan-183 final)

- **Runtime proof:** weather built with `mfb build --debug`, run until the forecast is loaded, Esc. All four Goal bounds hold; record the numbers in the commit.
- **Landing tier:** `bash scripts/test-fast.sh`, every `rt_arena_*`, `rt_thread_*`, `rt_worker_*` and `rt_canvas_*` binary.
- **Goldens:** run the full artifact gate and regenerate; the full acceptance run must show no `.run`/`build.log` change, run once when the host is free.
- **Archive** `plan-183-A` … `-E` to `planning/completed/` as each completes.

## Corrections
