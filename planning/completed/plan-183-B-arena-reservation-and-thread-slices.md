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

- [x] `src/manifest/`: `maxThreads` read, validate and diagnose; `01_project-manifest.md` section and code-table row. Done: `max_threads` on `ArenaMemoryLimits` (it reaches codegen with the arena settings, `cli/build/mod.rs` → NIR), `PROJECT_JSON_MAX_THREADS_INVALID` `2-200-0029` in the rule table and `01_rule-codes.md`. The `01_project-manifest.md` *Thread limit* section landed with B2, so the spec never described a limit the runtime did not enforce.
- [x] `src/codegen/builtins/errorcode/mod.rs`: `ErrThreadLimit`; `mfb man errorCode` prose (`.ai/man-content.md`); error-model spec entry. Done: `ErrThreadLimit` `7-705-0029` (`cargo test --bin mfb errorcode` → 7 passed, `table_matches_registry` included); `mfb man errorCode` renders it. The language threads topic's limit rule (the error model topic lists no codes; the threads topic owns `thread::start`) and `mfb man thread start`'s prose and declared errors landed with B2.
- [x] Tests: the manifest unit tests for valid, invalid and absent values (existing `src/manifest/` test module). `max_threads_reads_validates_and_defaults`; `cargo test --bin mfb manifest::` → 257 passed.

Acceptance: `cargo test --bin mfb manifest::` passes (est. 1 min); `mfb man errorCode` renders `ErrThreadLimit`.
Measured: `cargo test --bin mfb manifest::` → 257 passed; `mfb man errorCode` shows the `errorCode::ErrThreadLimit │ Integer │ 77050029` row.
Commit: a7779c91d

## Phase B2 — Reservation, slices, thread limit

- [x] `src/codegen/runtime/arena_cap/`: `lower_arena_reserve` and the three runtime words; called after `lower_arena_cap_init`; `08_program-startup.md` *The Arena Reservation*. `arena_cap/reserve.rs` (base, slice size, reserved bytes, slice bitmap), the platform seam `CodegenPlatform::emit_arena_vm` (`Reserve`/`Commit`/`Decommit`; raw syscalls on Linux and macOS, `VirtualAlloc`/`VirtualFree` on Windows); the entry calls it whether or not the cap is on. Smoke: `arena.slices 21`, `arena.slice_size 21474836480`.
- [x] `alloc.rs` step 5: commit from the slice frontier instead of `mmap`; `04_arenas.md` step 5. `emit_slice_commit`, 64 KiB commit granules; `arena_destroy` decommits the slice instead of unmapping blocks, and the now-unused `emit_arena_unmap` seam is deleted from every platform.
- [x] Main entry and canvas graphics spawn take slices 0 and 1. The reserve helper points the entry's arena at slice 0; `thread_lifecycle.rs` gives the graphics arena slice 1. `cargo test --test 'rt_canvas_*'` → see the acceptance line.
- [x] `src/codegen/builtins/thread/lowering.rs` and `src/codegen/runtime/thread/`: take two slices or raise `ErrThreadLimit`; `_mfb_rt_thread_finish` decommits and releases; `06_thread-runtime-helpers.md`, `07_control-block.md`. `thread/slices.rs` (`_mfb_rt_thread_slices_acquire`/`_release` under the thread-table lock, the build-specific message); the start helper acquires first and assigns the block arena the high slice and the worker arena the low one; `_mfb_rt_thread_finish` decommits the worker's slice (through `arena_destroy`) and the control block's release hands both back — see Corrections 1.
- [x] Tests:
  - new `tests/runtime/rt_thread_limit.rs` — `maxThreads: 2`; start 2 long-lived workers; the third `thread::start` traps `ErrThreadLimit`; after joining one, a new start succeeds; (plus a default-limit control where the same program's third start succeeds)
  - new `tests/runtime/rt_arena_reservation.rs` — run a program under `ulimit -v 67108864` (64 GiB) and assert it starts, with a smaller `arena.slice_size` in the debug report (B adds the `arena.slices`/`arena.slice_size` rows here; E adds the rest). (Linux only — macOS and Windows do not enforce `RLIMIT_AS`; a second case checks the default 21 × 20 GiB everywhere. The limited run settles on exactly 2.5 GiB slices.)
- [x] Added (from Validation): the goldens — full gate, `regen-native-goldens.sh`, gate back to 0, full `test-accept.sh` with no `.run`/`build.log` change — and the landing tier. Combined with C (plan-183-C Corrections 8): `scripts/artifact-gate.sh target/release/mfb all` → 166 diffs, all `.ncode`/`.mir` (`parser_hello_world` MIR: `_mfb_arena_flush_coalesce`/`_insert_free` gone, `_mfb_arena_commit`/`_geometry`/`_huge_alloc`/`_medium_alloc`/`_small_page`/`_span_free`/`_mfb_rt_arena_reserve` added, `_main`/`_alloc`/`_free`/`_destroy` changed); `scripts/test-accept.sh target/release/mfb` → 1756 ran, 9 mismatches, every one a native dump; `scripts/regen-native-goldens.sh target/release/mfb` → 230 rewritten, 0 failures; gate → 0 diffs over 2384 goldens. Landing tier: `bash scripts/test-fast.sh` → ok.

Acceptance: `cargo test --test rt_thread_limit --test rt_arena_reservation --test rt_worker_arena_release --test 'rt_canvas_*'` passes on macOS, 2223 and the Windows box (est. 6 min each).
Measured 2026-10-24 on the combined B+C build (plan-183-C Corrections 8): macOS `cargo test --no-fail-fast --test rt_thread_limit --test rt_arena_reservation --test rt_worker_arena_release` plus all 24 `rt_canvas_*` binaries → 27 binaries, 542 passed, 0 failed; box 2226 (for 2223, `MFB_TEST_REMOTE=linux-aarch64,test@127.0.0.1,2226,glibc`) and the Windows box (`windows-x86_64,test@192.168.1.206`) the three non-canvas binaries → 6 passed each. The `rt_canvas_*` programs run on the host only, so the boxes ran the canvas gates instead: `scripts/test-canvas-vulkan.sh target/debug/mfb --box 2226 --target linux-aarch64` → "canvas Vulkan runtime tests passed", `scripts/test-winapp.sh target/debug/mfb --box 22` → 46 ok, "passed".
Commit: f7ae0e904 (goldens in the commit after it)

## Validation

- `bash scripts/test-fast.sh`.
- Goldens: every binary links the startup reservation, so every `.ncodesum` moves. Run the artifact gate in full and regenerate. The full acceptance run must pass with no `.run`/`build.log` change before re-baselining.
- Doc sync: `01_project-manifest.md`, `08_program-startup.md`, `04_arenas.md`, threading 06/07, `mfb man errorCode`/`thread`.

## Corrections

1. **The control block's arena cannot give its slice back when the worker finishes.** It holds the worker's result, which the parent reads (`thread::waitFor`) after the worker has exited, and its queues, which the parent drains. So `_mfb_rt_thread_finish` decommits only the worker's own slice (its `arena_destroy`), and both slices go back to the pool in `_mfb_rt_thread_block_release`, when the thread has finished **and** its `Thread` value has been dropped. That is the meaning of "alive" the limit enforces; the manifest, threads and control-block topics and `mfb man thread start` say so.
2. **The slice bitmap is guarded by the thread-table mutex, not `_mfb_rt_arena_map_lock`.** The map lock exists only when the arena memory cap is enabled (`maxArenaMemoryDisable: true` emits none of it), while the slices must be managed either way; the table lock is taken at exactly the two moments slices change hands (start and final release), and only threaded programs touch the bitmap.
3. **Commits are rounded to 64 KiB granules.** macOS AArch64 pages are 16 KiB and `mprotect` needs a page-aligned address, while blocks are 4 KiB multiples. The arena commits `[round_up(frontier), round_up(frontier + block))`, so blocks keep their sizes and every commit is aligned for every target (Windows' allocation granularity is 64 KiB too).
4. **`arena_destroy` decommits the slice instead of unmapping blocks.** `munmap` of a block inside the reservation would leave a hole for any later `mmap` (libc's included) to land in, and the next commit at that address would `mprotect` someone else's memory. The block walk stays for the debug counters and the cap refund; the `emit_arena_unmap` seam had no caller left and was deleted.
5. **The reservation asks for 2 MiB more than `slices × size`,** so the slices can start 2 MiB-aligned whatever address the OS returns, and the halving tries exactly the floor before giving up (20 GiB per slice halves to 320 MiB, then 160 MiB would be below the floor). The failure line names the floor request, `slices × 256 MiB + 2 MiB` bytes, a build-time constant.
6. **The reservation's words are emitted in every native module,** not only an entry module: every module's `arena_alloc` reads the slice size. An arena with no slice gets `ErrOutOfMemory` from the grow path.
7. **A GPU glyph run read its clip from the stack, and B's acceptance found it.** `rt_canvas_font` `text_on_the_gpu_matches_the_software_oracle` failed on the combined build 2 runs in 6 (the HEAD build: 16 of 16 good). The Metal and Vulkan glyph paths never set the item block's stroke-style and clip words, so the shader clipped each run by whatever the stack held, which the arena helpers had changed; and a `Text`'s own `Paint.clip` was ignored on the GPU. Fixed in its own commit (7e3d37568) with a clipped-text GPU test and a clipped run in `test-canvas-vulkan.sh`, both red without the fix.
8. **Box 2223 was down** (`ssh -p 2223` → `Connection refused`, 2026-10-24). Box 2226 (aarch64 glibc) stands in for it; 2224 (aarch64 musl) and 2227 (x86_64 musl) were run too.
9. **Two runtime tests held more than the default 10 threads** and failed with `ErrThreadLimit` once the limit existed — `rt_thread_table_growth` (200 live) and `rt_thread_drop_running_finish_race` (dropped-but-running workers). Neither binary was in B's acceptance list; plan-183-D's collection sweep found them. Their projects now raise `options.maxThreads` (e8db69ec7); every other test binary using `thread::start` passed unchanged (22 binaries, 314 passed).
