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

- [x] `free.rs:lower_arena_flush_coalesce`: the two-flush decommit and frontier retreat; the small empty-page cap. There is no flush since plan-183-C (Corrections 1): `arena_free` calls a new `_mfb_arena_sweep` every 64 medium/huge frees (`decommit.rs`). A hot run ≥ 4 MiB found free at two sweeps is made cold (whole pages decommitted); a free run at a frontier found there at two sweeps retreats it (D flipped which ends those are). Cold runs commit the part taken (`span.rs:commit_cold_part`), merges keep the decommitted count exact (`emit_cold_fixup`). Empty small pages past 64 are decommitted and recommitted on reuse (`small.rs`). `rt_arena_regions` `an_idle_run_goes_back_to_the_os_after_two_sweeps` and `a_free_run_at_the_frontier_retreats`: pass; with the sweep disabled (`ARENA_SWEEP_EVERY` 1,000,000) both fail.
- [x] `free.rs:lower_arena_free`: large-free page decommit with edge fill. In `span.rs:lower_arena_span_free`, where medium/huge frees land: ≥ 4 MiB fills only the partial edge pages and decommits the rest, joining its neighbours as a cold run.
- [x] `src/codegen/debug/arena/`: the report rows; `09_debug-report.md`. The per-arena rows landed with C (plan-183-C Corrections 3); E adds the process row `arena.reservation_bytes` (450,973,663,232 by default: 21 × 20 GiB + 2 MiB) and says in the spec what the decommit rows now count.
- [x] `rt_arena_large_block_reuse.rs`: replace the `MAX_MAPPED_PER_PEAK_LIVE = 4` bound with `committed ≤ 1.25 × peak_live + 64 MiB`. Done in plan-183-C (its Corrections 4: committed bytes made the ×4 bound wrong there); passes on every row below.
- [x] `rt_arena_zeroing.rs`: free a 64 MiB chunk, force a flush, reallocate and assert all zero (the decommit path). `a_decommitted_chunk_comes_back_zero`: four 64 MiB lists; a large free decommits at once, so no flush is needed — ≥ 3 × 60 MiB decommitted, the run taken back, `zero_violations` 0, under 1 MiB zero-filled. The module doc's stale "first 16 bytes hold the free-list node" (true before C) was corrected.
- [x] `planning/todo.md` "## Memory": close Spike 8, Spike 9, Bucket 9, Bucket 12a and A/B 7 with "plan-183". Each now carries a **Closed (plan-183)** line with what replaced it.

- [x] Added (from Validation): the goldens (full gate, full `test-accept.sh` with no `.run`/`build.log` change, `regen-native-goldens.sh`, gate back to 0) and the landing tier (`test-fast` and every `rt_arena_*`, `rt_thread_*`, `rt_worker_*`, `rt_canvas_*` binary). Gate → 166 diffs, all `.ncode`/`.mir` (`parser_hello_world` MIR: `_mfb_arena_decommit` and `_mfb_arena_sweep` added; `_main`, `_small_page`, `_medium_alloc`, `_huge_alloc`, `_span_free`, `_extend`, `_free`, `_destroy` changed; `_mfb_rt_park_error` vreg numbering only); `test-accept.sh target/release/mfb` → 1756 ran, 9 mismatches, all native dumps; `regen-native-goldens.sh` → 230 rewritten, 0 failures; gate → 0 diffs over 2384 goldens. `bash scripts/test-fast.sh` → ok; the 39 landing binaries → 570 passed, 0 failed.

Acceptance: `cargo test --test rt_arena_large_block_reuse --test rt_arena_zeroing --test rt_arena_regions` passes on macOS, 2223 and the Windows box (est. 4 min each).
Measured 2026-10-25, with `rt_arena_grow_in_place` and `rt_arena_memory_cap` added: macOS, box 2226 (for 2223), the Windows box, box 2229 (riscv64) and box 2227 (x86_64) — 5 binaries, 15 passed, 0 failed on each; rerun after Corrections 6 with its new test: 16 passed, 0 failed on each.
Commit: 850c09337 (goldens in the commit after it)

## Validation (plan-183 final)

- **Runtime proof:** weather built with `mfb build --debug`, run until the forecast is loaded, Esc. All four Goal bounds hold; record the numbers in the commit.
- **Landing tier:** `bash scripts/test-fast.sh`, every `rt_arena_*`, `rt_thread_*`, `rt_worker_*` and `rt_canvas_*` binary.
- **Goldens:** run the full artifact gate and regenerate; the full acceptance run must show no `.run`/`build.log` change, run once when the host is free.
- **Archive** `plan-183-A` … `-E` to `planning/completed/` as each completes.

**Runtime proof, measured 2026-10-25** (weather `mfb build --debug`, the 2026-10-25 06z cycle fully loaded — 41 hours; Corrections 7 for the method): `arena.0.committed_peak_bytes` 1,129,414,656 ≤ 1.25 × `peak_live_bytes` 1,105,396,448 + 64 MiB = 1,448,854,424 (`committed_bytes` at exit 1,077,968,896); `peak_live_bytes` 1,105,396,448 ≤ live after load 1,071,919,744 + 32 MiB = 1,105,474,176 (77,728 bytes to spare); `decommitted_bytes` 1,818,656,768 > 0 (193 decommits); `fill_grow_bytes` 0. `arena.slices` 22, `arena.reservation_bytes` 472,448,499,712; `extend_hits` 607 of 51,647.

## Corrections

1. **There is no flush to hang the two-flush rule on.** plan-183-C replaced the bins and their flush with runs that merge at every free. The rule moved to a sweep `arena_free` runs every 64 medium/huge frees; "free at the previous flush" became "flagged at the previous sweep", with its own flag for the frontier rule so the large-run mark cannot count as the frontier's first sweep.
2. **Frontier retreat runs in the sweep, under the same two-sweep rule,** not on every free: a chunk freed and taken again at a frontier would otherwise decommit and recommit a commit step each time. D's flip makes the ends the *top* of huge and the *bottom* of medium.
3. **A run's flags survive takes and pass through merges.** First built so any change cleared them, the idle-run test showed a churn taking 24 KB chunks from the edge of a 40 MiB free run and freeing them back kept the whole run young forever. Now a take's rest keeps the flags and a merged run takes its runs' seen flags.
4. **A cold run the frontier retreats over keeps the commit edge out of its interior.** Found by the frontier test: the edge was moved to the next commit step, inside the decommitted interior, and the next frontier chunk faulted (bus error). Over a cold run the medium edge stops at `page_down(end)`, the huge edge at `page_up(start)`.
5. **Decommitted memory faults until committed again** on every target (macOS `mmap` fixed with no access, Linux `madvise` + `mprotect(PROT_NONE)`, Windows `MEM_DECOMMIT`), so reuse always commits first and a missed commit is a crash, not stale data.
6. **The weather proof found a decommit past a run's end.** A windowed weather run hit a bus error in `windAt`: the first page of the live `earlierWinds` chunk was decommitted. Reproduced headless by a stress of 2–30 MB strings beside growing lists, then localized with an lldb log of every commit/decommit/extend and a watchpoint on the chunk's tag record: a free merged with a *cold run shorter than a page* to its right — `[end, liveStart)`, whose interior is empty — and the merge's gap decommit ran to `page_up(end)`, past the run into the live chunk's page. Every merge decommit is now capped at the merged run's last whole page, `page_down(re)`. New regression test `rt_arena_regions` `a_tiny_cold_run_beside_a_live_chunk_leaves_it_committed` builds exactly that layout; it faults with the cap removed.
7. **The proof ran headless and quit with Escape on stdin, not in a window.** Windowed runs need a display: with the screen locked the window surface never appears and the program never starts (no canvas stats written in a 10 s windowed run). `MFB_HEADLESS=1` with stdin a FIFO keeps the program running (headless `io::readKey` reads stdin, and an end-of-file quits it) and an Escape byte written into the FIFO quits it the normal way. "Live after load" was read from the `--debug` registry (`_mfb_rt_debug_arena_base` slot 0, `live_bytes` at +88) by attaching lldb at 15, 25 and 40 s, because the report's own `live_bytes` is taken after `main` has returned and freed its locals.
