# plan-183-A: Arena — fast zeroing, no fill of fresh memory

Last updated: 2026-10-24
Overall Effort: huge (> 3d) — plan-183 is split A → E, landed in letter order
Effort: medium (1h–2h)
Depends on: nothing

## plan-183 overview (shared by every letter)

The arena does not work the way it was designed to. It maps a block per request
(`max(4096, size + slack)`), so a large list gets one OS mapping per growth step;
blocks never merge, nothing is returned, and fresh blocks are zero-filled by hand.
`examples/weather` maps 3.64 GB for a 1.78 GB peak and spends 629 GB of zeroing in 70 s.

plan-183 replaces the block layer while keeping one invariant: **every byte not in
active use by the program is zero.** The letters, in order:

| Letter | Delivers |
|---|---|
| A | fast per-target zeroing; fresh OS memory is never filled by hand |
| B | `maxThreads`, `ErrThreadLimit`, one startup reservation cut into equal arena slices |
| C | three regions per arena, side bitmaps, free-list bookkeeping out of free memory, the new arena-state layout, block headers removed, committed-byte cap |
| D | `arena_extend`, so collections grow in place |
| E | decommit of long-free ranges, frontier retreat, `--debug` report rows, weather proof |

Owner decisions (2026-10-24), binding on every letter:
- Zero-on-free stays.
- No allocation directly from the OS per request; the runtime manages one reservation.
- Arena size is `min(20 GiB, reservation ÷ slices)`. Slices = `1 + graphics + 2 × maxThreads`, and all arenas are equal.
- `options.maxThreads` defaults to 10. A new error, `ErrThreadLimit`, is raised past it.
- Three regions per arena: small fixed, medium and huge growing toward each other.
- The arena-state layout may change as needed.
- Tuning values (confirmed by the owner):
  - 64 KiB small pages;
  - a huge tier at 32 MiB and up;
  - 4 MiB commit steps;
  - a 4 MiB minimum decommit run;
  - a 256 MiB per-arena reservation floor;
  - `maxThreads` 1–4096.

Remote boxes (`.ai/remote_systems.md`):

| Box | Platform | Use |
|---|---|---|
| 2223 | aarch64 glibc | preferred Linux box |
| 2227 | x86_64 musl | emulated; only for x86_64-specific checks, keep runs short |
| 2229 | riscv64 musl | riscv64 |
| `test@192.168.1.206` | Windows | the only Windows box |

## Goal

Freed memory is zeroed by a fast per-target routine, and freshly mapped memory is
never zeroed by hand. The visible effect: `fill_grow_bytes` is 0, and freeing costs
less CPU.

### Non-goals

- Allocation and free algorithms, block layout and the cap are unchanged (B–E change them).
- Zero-on-free stays.

## Current state (read 2026-10-24)

- `src/codegen/memory/arena/arena/mod.rs:lower_arena_fill_zero` stores 8 bytes at a time, 32 bytes per iteration.
- `src/codegen/memory/arena/arena/alloc.rs:lower_arena_alloc` grow path calls it on every freshly mapped block (`04_arenas.md` step 5, "calls `arena_fill_zero` to zero the freshly mapped block").
- `free.rs:lower_arena_free` calls it over `[ptr+16, ptr+size)` on every free.
- Weather (`mfb build --debug`, 70 s): `fill_grow_bytes` 3,641,109,184; `fill_free_calls` 260,347,971; `fill_free_bytes` 629,284,695,184.
- Fresh anonymous memory is zero on every target: `mmap(MAP_ANONYMOUS)` on Linux and macOS, `VirtualAlloc(MEM_COMMIT)` on Windows.

## Design

- **AArch64** (macOS, Linux aarch64):
  - At startup, read `DCZID_EL0` once into a runtime word, `_mfb_rt_dczva_bytes`. If its DZP bit (bit 4) is set, `dc zva` is prohibited and the word stays 0. Otherwise the word is `4 << BS` (the block size).
  - `fill_zero` with a non-zero word: plain stores up to block alignment, `dc zva` per block, plain stores for the tail. With 0: a 64-byte `stp xzr, xzr` loop.
- **x86-64** (Linux, Windows): `rep stosb`.
- **riscv64:** an unrolled loop of 8 `sd zero` stores, 64 bytes per iteration.
- **Grow path:** drop the `arena_fill_zero` call; the frame stays, because other calls remain.

## Phase A1 — Fast zero routine, no fresh-block fill

- [ ] `mod.rs:lower_arena_fill_zero`: the three per-arch bodies above.
- [ ] Startup: the `DCZID_EL0` read into `_mfb_rt_dczva_bytes` (AArch64 entry and the `--app` program entry), with the DZP check.
- [ ] `alloc.rs`: remove the grow-path fill.
- [ ] `04_arenas.md` *Zero Fill* and step 5: the OS-zero guarantee per target; the new routine.
- [ ] Test: new `tests/runtime/rt_arena_zeroing.rs` — free chunks of 24 B, 3 KiB and 5 MiB, reallocate the same sizes, and assert every byte past the first 16 is zero (C tightens this to all bytes); assert the `--debug` report's `fill_grow_bytes` is 0.

Acceptance: `cargo test --test rt_arena_zeroing --test rt_arena_large_block_reuse` passes on macOS, 2223, 2227 (x86_64 for `rep stosb`; short run) and the Windows box (est. 3 min each, 2227 longer: emulated).
Commit: —

## Validation

- `bash scripts/test-fast.sh`.
- Codegen goldens: every binary links `fill_zero`, so every `.ncodesum` is expected to move. `scripts/artifact-gate.sh target/release/mfb all`, then regenerate. Per AGENTS.md, the full acceptance run must pass with no `.run`/`build.log` change before the re-baseline commit, run when the host is free.
- Doc sync: `04_arenas.md`.

## Corrections
