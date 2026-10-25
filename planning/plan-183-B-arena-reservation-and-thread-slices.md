# plan-183-B: Arena — one reservation, equal arena slices, `maxThreads`, `ErrThreadLimit`

Last updated: 2026-10-24
Effort: large (3h–1d)
Depends on: plan-183-A (overview, owner decisions and box list are in plan-183-A)

## Goal

- At startup the process reserves one address range and cuts it into equal arena slices.
- Every arena's memory comes from its slice.
- `thread::start` with `maxThreads` workers alive fails with `ErrThreadLimit`.
- A finished thread's slices return to the pool, decommitted and therefore zero.

### Non-goals

- The allocator inside an arena is unchanged. It still carves 32-byte-header blocks, but commits them from its slice instead of calling `mmap` (C replaces it).
- The cap still counts the bytes of those blocks (C switches it to committed bytes).

## Current state (read 2026-10-24)

**Arena owners** (registration sites of `ARENA_KIND_*`, `src/codegen/debug/arena/mod.rs`):
- `main`: the entry thread; in `--app` builds, the language worker that runs the entry.
- `graphics`: canvas only (`src/codegen/runtime/canvas/thread_lifecycle.rs`).
- `worker` (`src/codegen/runtime/thread/runtime_helpers.rs`) and `thread_block` (`src/codegen/runtime/thread/control_block.rs`), one pair per `thread::start`.

The UI thread has no arena state (`src/docs/spec/app/07_window.md`), and nor does the Vulkan pipeline builder (`06_canvas.md`). Audio callbacks touch no arena (`src/codegen/builtins/audio/gen_macos_callbacks.rs`).

**Other facts:**
- There is no thread limit today: the thread table starts at 64 and doubles (`09_debug-report.md`).
- Workers unmap their arena in `_mfb_rt_thread_finish` (plan-156-B).
- The cap check runs at startup before the first allocation (`08_program-startup.md`, `src/codegen/runtime/arena_cap/mod.rs:lower_arena_cap_init`). Its lock `_mfb_rt_arena_map_lock` is a process-wide mutex.
- Manifest arena keys live in `src/manifest/arena_memory.rs`; error codes in `src/codegen/builtins/errorcode/mod.rs` (precedent: `ErrCanvasGroupLimit`).

## Design

- **Slices:** `1 (main) + 1 (graphics, canvas programs only) + 2 × maxThreads`. That's 21 slices with the default, 22 for canvas.
- **Reservation:** right after the cap check, a new `_mfb_rt_arena_reserve` reserves `slices × 20 GiB` with no access: `mmap(PROT_NONE, MAP_PRIVATE|MAP_ANONYMOUS|MAP_NORESERVE)` on Linux/macOS, `VirtualAlloc(MEM_RESERVE, PAGE_NOACCESS)` on Windows.
  - On failure it halves and retries.
  - The floor is `slices × 256 MiB`. Below it the program writes one stderr line (`error: could not reserve <N> bytes of address space for <S> arenas; the program did not start.`) and exits 1, through the cap check's halt path.
  - The result goes in `_mfb_rt_arena_base` and `_mfb_rt_arena_slice_size` = `min(20 GiB, reserved ÷ slices)`, rounded down to 2 MiB.
- **Slice bitmap:** `_mfb_rt_arena_slice_bits`, `ceil(slices / 64)` words, guarded by `_mfb_rt_arena_map_lock`. Slice 0 is main and slice 1 is graphics (canvas). Workers take two free slices at `thread::start`, under the lock.
- **Thread limit:** when fewer than two slices are free, `thread::start` fails with `ErrThreadLimit` before anything is created. The message is `"<N> threads are already running; raise options.maxThreads in project.json to allow more"`.
- **Commit within a slice:** the existing block allocator's map call becomes "commit the next `map_size` bytes at the slice's commit frontier" (`mprotect` read/write; `VirtualAlloc(MEM_COMMIT)`). Fresh pages are zero (A). If the slice is exhausted, the request fails with `ErrOutOfMemory`.
- **Thread finish:** `_mfb_rt_thread_finish` decommits both of the thread's slices and clears their bits. Decommit by platform:
  - Linux: `madvise(MADV_DONTNEED)`, plus `mprotect(PROT_NONE)`.
  - macOS: `mmap(MAP_FIXED|MAP_PRIVATE|MAP_ANONYMOUS, PROT_NONE)` over the range. `MADV_FREE`-style advice does not guarantee zero on macOS.
  - Windows: `VirtualFree(MEM_DECOMMIT)`.
  The next thread finds the slice zero.
- **`options.maxThreads`:** an integer 1–4096, default 10, validated in `src/manifest/` beside `arena_memory.rs`. An invalid value raises a new `2-200-####` diagnostic, `PROJECT_JSON_MAX_THREADS_INVALID`. It is fixed into the binary at build time.

## Phase B1 — Config key and error code

- [ ] `src/manifest/`: `maxThreads` read, validate and diagnose; `01_project-manifest.md` section and code-table row.
- [ ] `src/codegen/builtins/errorcode/mod.rs`: `ErrThreadLimit`; `mfb man errorCode` prose (`.ai/man-content.md`); error-model spec entry.
- [ ] Tests: the manifest unit tests for valid, invalid and absent values (existing `src/manifest/` test module).

Acceptance: `cargo test --bin mfb manifest::` passes (est. 1 min); `mfb man errorCode` renders `ErrThreadLimit`.
Commit: —

## Phase B2 — Reservation, slices, thread limit

- [ ] `src/codegen/runtime/arena_cap/`: `lower_arena_reserve` and the three runtime words; called after `lower_arena_cap_init`; `08_program-startup.md` *The Arena Reservation*.
- [ ] `alloc.rs` step 5: commit from the slice frontier instead of `mmap`; `04_arenas.md` step 5.
- [ ] Main entry and canvas graphics spawn take slices 0 and 1.
- [ ] `src/codegen/builtins/thread/lowering.rs` and `src/codegen/runtime/thread/`: take two slices or raise `ErrThreadLimit`; `_mfb_rt_thread_finish` decommits and releases; `06_thread-runtime-helpers.md`, `07_control-block.md`.
- [ ] Tests:
  - new `tests/runtime/rt_thread_limit.rs` — `maxThreads: 2`; start 2 long-lived workers; the third `thread::start` traps `ErrThreadLimit`; after joining one, a new start succeeds;
  - new `tests/runtime/rt_arena_reservation.rs` — run a program under `ulimit -v 67108864` (64 GiB) and assert it starts, with a smaller `arena.slice_size` in the debug report (B adds the `arena.slices`/`arena.slice_size` rows here; E adds the rest).

Acceptance: `cargo test --test rt_thread_limit --test rt_arena_reservation --test rt_worker_arena_release --test 'rt_canvas_*'` passes on macOS, 2223 and the Windows box (est. 6 min each).
Commit: —

## Validation

- `bash scripts/test-fast.sh`.
- Goldens: every binary links the startup reservation, so every `.ncodesum` moves. Run the artifact gate in full and regenerate. The full acceptance run must pass with no `.run`/`build.log` change before re-baselining.
- Doc sync: `01_project-manifest.md`, `08_program-startup.md`, `04_arenas.md`, threading 06/07, `mfb man errorCode`/`thread`.

## Corrections
