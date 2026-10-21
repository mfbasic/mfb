# plan-179-C: Arena memory cap — allocator enforcement

Last updated: 2026-10-19
Effort: medium (1h–2h)
Depends on: plan-179-B

This sub-plan enforces the limit computed by plan-179-B. Every arena block mapped
by the process counts against one process-wide total. A mapping that would take
the total past the limit fails with `ErrOutOfMemory`, which the program can catch.
Unmapping a block when a worker's arena is destroyed gives its bytes back.

References:

- `planning/completed/plan-179-A-arena-cap-settings.md`, `planning/completed/plan-179-B-arena-cap-startup-check.md` — the settings and the stored limit.
- `src/docs/spec/memory/04_arenas.md` — step 5 (Grow) of `arena_alloc`, the block layout (`blockSize` at +8), and the Arena-State Layout.
- `src/docs/spec/memory/08_program-startup.md` — the probe step added in plan-179-B.
- `.ai/testing-gates.md` — `[[test]]` registration, the golden gates, the arena benchmark gate.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-179-B landed | `grep -n "^Commit:" planning/completed/plan-179-B-arena-cap-startup-check.md` → a hash, not `—` | MET (2026-10-20: `Commit: 16b5f4312`) |

Everything below is written against the world where this holds.

> **NOTE — the Status column is a snapshot; the Command column is the truth.** Re-run every command before continuing.

## 1. Goal

- A program that allocates past `maxArenaMemoryMiB` (or past the percent limit) gets a catchable `ErrOutOfMemory`, and a program that catches it and stops allocating exits normally.
- A program that churns through workers whose total mapped bytes exceed the limit, but whose live mapped bytes do not, completes without error.
- With `maxArenaMemoryDisable: true`, no counter is read or written, and the program runs to completion past the limit.

### Non-goals (explicit constraints)

- No change to the fast path of `arena_alloc` (quick bins, carve chunk, free list). The check sits only on the path that maps a block.
- No change to what `ErrOutOfMemory` means or how it is raised; it reuses `arena_alloc_oom` (`src/codegen/memory/arena/arena.rs`).
- Non-arena mappings are not counted: the perf region (`src/codegen/builtins/perf/perf.rs:129`) and the debug registry (`src/codegen/debug/arena.rs:473`). They are not arena blocks and must not be charged to the arena budget.
- No change to `ArenaState` layout. The counter and lock are data symbols.
- The debug counter `COUNTER_MAPPED_BYTES` (`src/codegen/debug/arena.rs`) is not changed. It stays a per-arena debug statistic.

## 2. Current State

- `lower_arena_alloc` (`src/codegen/memory/arena/arena.rs`) maps blocks at one site: `platform.emit_arena_map(map_size.into(), …)` at `arena.rs:907`. Its failure path branches to `arena_alloc_oom` (`arena.rs:915–916`, the `branch_ge("arena_alloc_mapped")` / `branch("arena_alloc_oom")` pair; the label is at `arena.rs:1042`). The grow path is reached only after the flush-before-grow retry, so parked memory is reused before a mapping is attempted.
- `lower_arena_destroy` (`arena.rs:1923`) is the only unmap site: `platform.emit_arena_unmap(...)` at `arena.rs:1986`. It unmaps the chain of blocks from `blockHead` through `prevBlock` (`04_arenas.md`, Block Layout).
- `arena_fill_zero` runs on every freshly mapped block (`arena.rs`, grow path), so mapped bytes are touched when mapped; the cap bounds resident memory, not only address space.
- `ARENA_STATE_SIZE` is 3776 bytes (`04_arenas.md`).

### Measured populations

| What | Count | Command |
|---|---|---|
| Real arena map call sites | 1 (`arena.rs:907`) | `grep -rn "\.emit_arena_map(" src --include='*.rs' \| grep -v test` → `arena.rs:907`, `perf.rs:129`, `debug/arena.rs:473`, and the Linux delegation `linux_common/code.rs:1331` |
| Non-arena map call sites to exclude | 2 (`perf.rs:129`, `debug/arena.rs:473`) | same command |
| Real arena unmap call sites | 1 (`arena.rs:1986`) | `grep -rn "\.emit_arena_unmap(" src --include='*.rs' \| grep -v test` → `arena.rs:1986`, and the Linux delegation `linux_common/code.rs:1335` |
| Existing pthread mutex init sites | 2 (`canvas/mod.rs:2656`, `runtime_helpers.rs:683`) | `grep -rn "pthread_mutex_init" src/codegen --include='*.rs' \| grep -v test` |

### Verified properties

- **The lock primitive is portable.** Read `emit_thread_external_call` (`src/codegen/runtime/thread/runtime_helpers.rs:61–67`): it maps `pthread_mutex_lock` and `pthread_mutex_unlock` to Win32 SRWLOCK on Windows, so one code path covers every target. The debug registry already takes a process-wide lock this way (`src/codegen/debug/arena.rs:401–418`, `emit_registry_lock`).
- **No atomic instructions are needed.** Measured: `grep -rln "ldaxr\|stlxr\|lock cmpxchg\|cmpxchg" src/target src/arch src/codegen --include='*.rs'` → no backend emits a compare-and-swap. The counter is therefore protected by the mutex, not by an atomic loop. This is why the design avoids a CAS loop.
- **A mutex is initialized at startup.** Read the canvas precedent (`src/codegen/runtime/canvas/mod.rs:2656`, `pthread_mutex_init(&mutex, NULL)`). The new mutex is initialized the same way, in the entry shim, before the first allocation.
- **The unmap site knows the block size.** Read the block header layout (`04_arenas.md`, Block Layout): `blockSize` at header +8 is the total mapped size. `lower_arena_destroy` walks `prevBlock` from `blockHead`, so the size of each block is loadable at its unmap site.

## 3. Design Overview

Three pieces, all behind `!arena_memory.disable`:

1. **Counter and lock.** Two data symbols, `_mfb_rt_arena_mapped_bytes` (u64, starts at 0) and `_mfb_rt_arena_map_lock` (pthread mutex, initialized at startup). The limit is `_mfb_rt_arena_map_limit` from plan-179-B.
2. **Charge before map, refund on failure.** At the mapping site, under the lock: `mapped += map_size`; if the new total exceeds the limit, undo the add and branch to `arena_alloc_oom`. Otherwise release the lock and map. If the mapping fails, take the lock again and undo the add before branching to `arena_alloc_oom`.
3. **Refund on unmap.** At each block unmap in `lower_arena_destroy`, under the lock: `mapped -= blockSize`.

Why charge before the map, not after: two threads can both pass a check made before either adds. Charging under the lock makes check-and-add one step.

Why a mutex, not a CAS: no backend emits a CAS (measured above), and the mutex is already available everywhere. The lock is taken only on a block map and a block unmap, which are rare next to the allocation fast path.

Correctness risk concentrates in the refund on mapping failure. A missed refund leaks budget until the program hits a limit it never reached; this is why §Validation includes a test that fails if the refund is missing.

Rejected alternatives:

- Per-arena limit. Rejected: each worker has its own arena, so N threads would each get a full limit, and the total could be N × limit.
- A field in `ArenaState` for the counter. Rejected: it changes `ARENA_STATE_SIZE` and the zero-init loop; a data symbol keeps the layout.
- Charge after the map. Rejected: a successful map would have to be unmapped again on overflow, which is worse than a failed charge.
- Check-only (no charge), relying on the mapped total from the debug counter. Rejected: the debug counter is per-arena and only exists in `--debug` builds (`src/codegen/debug/arena.rs`).

## 4. Detailed Design

### 4.1 Symbols and initialization

- `_mfb_rt_arena_mapped_bytes`: u64 data symbol, 0.
- `_mfb_rt_arena_map_lock`: pthread mutex data symbol.
- The entry shim calls `pthread_mutex_init(&_mfb_rt_arena_map_lock, NULL)` (precedent `canvas/mod.rs:2656`) after the probe of plan-179-B and before the first allocation. Emitted only when the cap is enabled.

### 4.2 Map site (`lower_arena_alloc`, `arena.rs:907`)

Immediately before `platform.emit_arena_map`, when the cap is enabled:

1. Lock `_mfb_rt_arena_map_lock`.
2. `mapped = load(_mfb_rt_arena_mapped_bytes) + map_size`.
3. If `mapped > _mfb_rt_arena_map_limit`: unlock, branch to `arena_alloc_oom`.
4. Store `mapped`; unlock.

After `emit_arena_map`, in the existing `branch_ge("arena_alloc_mapped")` / `branch("arena_alloc_oom")` sequence (`arena.rs:915–916`), add the refund on the failure branch:

5. On failure: lock; `mapped = load(...) - map_size`; store; unlock; branch to `arena_alloc_oom`.

The success path is unchanged after the store.

### 4.3 Unmap site (`lower_arena_destroy`, `arena.rs:1986`)

For each block: load `blockSize` from the block header (+8), lock, `mapped -= blockSize`, store, unlock, then unmap.

### 4.4 Limit semantics

- `_mfb_rt_arena_map_limit == 0` means no enforcement (plan-179-B, probe failure under `warn` with MiB unset). The map site treats `0` as no limit: it skips the lock and check entirely. Implementation: the check branch is `if limit == 0 → skip`.
- The counter includes block headers and the big-grow slack (`04_arenas.md`, step 5), because `map_size` is the mapped size. That is the quantity the cap is about.

## Compatibility / Format Impact

- With the cap enabled (the default), programs that map arena blocks gain a lock acquisition per block map and unmap, and the new check. Output is unchanged for programs that stay under the limit.
- A program that exceeds the limit now gets `ErrOutOfMemory` where it previously got more memory. This is the intended behavior and is the only observable change.
- `maxArenaMemoryDisable: true` emits none of this code.

## Phases

### Phase 1 — Counter, lock, map and unmap accounting

- [x] `src/codegen/memory/arena/arena.rs`: the map-site charge and refund (§4.2) at `arena.rs:907` and its failure branch, gated on the cap being enabled. In `arena/alloc.rs` (`lower_arena_alloc`), calling `emit_map_charge`/`emit_map_refund` from the new `src/codegen/runtime/arena_cap/accounting.rs`; the charge has the carry check the notes asked for.
- [x] `src/codegen/memory/arena/arena.rs`: the unmap refund in `lower_arena_destroy` (§4.3) at `arena.rs:1986`. In `arena/mod.rs`, `emit_unmap_refund` before each block's unmap.
- [x] `src/codegen/engine/function/entry.rs`: the two data symbols and the `pthread_mutex_init` call (§4.1), gated on the cap being enabled, placed after the plan-179-B probe. The symbols come from `arena_cap::data_objects`; there is no `pthread_mutex_init` call, because the lock is a statically initialized data object (Correction 2).
- [x] `src/docs/spec/memory/04_arenas.md`: a new section "Process memory cap" (the counter, the lock, the check, the refund on unmap, the non-counted mappings listed in §1), and the one-line addition to step 5 (Grow) pointing to it. `mfb spec memory arenas | grep -c '\[\['` → 0.
- [x] `src/docs/spec/memory/08_program-startup.md`: the mutex initialization step. Stated as "no initialization step: statically initialized", plus `_mfb_shutdown` zeroing the limit first (Corrections 2, 3).
- [x] `Cargo.toml`: a `[[test]]` entry for each new runtime test file, following the entries at lines 609 and 613. `rt_arena_memory_cap`, after `rt_arena_large_block_reuse`.
- [x] `tests/runtime/rt_arena_memory_cap.rs`: three tests, §Validation. Four: the three plus the Linux-only `a_failed_map_gives_its_charge_back` (Correction 4). `cargo test --test rt_arena_memory_cap` on macOS → 3 passed (the fourth is `cfg(target_os = "linux")`).
- [x] Added: `_mfb_shutdown` stores 0 into `_mfb_rt_arena_map_limit` first (`lower_shutdown`, `arena_cap::emit_disable_accounting`) — the SIGINT/SIGTERM self-deadlock hazard from the notes. 40 SIGINTs at 10–280 ms into the worker-churn program on macOS: `runs=40 hangs=0` (39 × exit 130, 1 finished first).
- [x] Added: the POSIX `process::spawn`/`process::shell` fork child stores 0 into the limit before it allocates (`emit_spawn_tail`, threaded by the new `AbiCtx::arena_cap` / `ArenaLayout::arena_cap`) — the fork-child deadlock hazard from the notes. `process::shell("exit 7")` and the four-argument `process::spawn` with an env map under the cap: `shell=7 spawn=3`, rc=0, on macOS and on 2227.
- [x] Added: the lock/unlock imports, attributed to the entry like the `--debug` registry lock (`src/target/shared/plan/symbols.rs`, the shared `arena_lock_imports`).

Acceptance:
  Check (macOS, local): `cargo test --test rt_arena_memory_cap` → all tests pass. Covers the cap, the disable, and the worker churn.
  Check (Linux aarch64): box 2223, the same test on a build made there → all pass.
  Check (Linux x86-64 musl): box 2227 → all pass.
  Check (Windows x86-64): 192.168.1.206 → all pass.
  Check (refund): temporarily remove the refund in §4.2 step 5 → the failure-path test fails (it passes the mapping failure through a charge that is never returned). Restore it. This proves the refund is exercised.
  Check (performance): `./benchmark/language/run.sh --run 50` → the `arena` group's mfb rows are within the noise of the baseline in `benchmark/language/baseline/`. The grow path changed; the fast path did not. If a row moved beyond the noise, the change is in the wrong place; diagnose before continuing.
  Check (goldens): `scripts/artifact-gate.sh <mfb> all` → diffs only in the grow and unmap paths of fixtures that map arena blocks, on all five targets. Inspect one fixture to confirm the diff is the new charge, refund and lock calls, then re-baseline only those lines (AGENTS.md).
  Result (macOS): `cargo test --test rt_arena_memory_cap` → `3 passed; 0 failed`. The cap case stops at `held 15` under 16 MiB and prints `out of memory`, rc=0; the disabled case reaches `done 64`; the churn case prints `workers 50 total 30450`.
  Result (Linux aarch64, 2223): the same test built there (`cargo test --test rt_arena_memory_cap`) → `4 passed; 0 failed` (includes the failed-map refund under `ulimit -v 524288`).
  Result (Linux x86-64 musl, 2227; also riscv64 musl, 2229): the three programs cross-built and run → cap `held 15` + `out of memory` rc=0, disabled `done 64`, churn `workers 50 total 30450` (Correction 5).
  Result (Windows x86-64, 192.168.1.206): the same three cross-built programs → `held 15`/`out of memory` rc=0, `done 64` rc=0, `workers 50 total 30450` rc=0.
  Result (refund): with `emit_map_refund` neutered the refund program on 2223 under `ulimit -v 524288` prints `-1 -1 -1` (the 256 MiB request is refused); restored, `-1 -1 268435456`. With `emit_unmap_refund` neutered the churn program dies with `7-701-0001 Allocation failed` at `src/main.mfb:6:23`; restored, it completes. Both mutations were reverted (`grep -c "P179 MUTATION"` → 0).
  Result (zero limit, 2223, `LD_PRELOAD` failing `sysinfo`): `warn` with no MiB key stores 0 and the dive reaches `done 64`; `warn` with `maxArenaMemoryMiB: 16` stores 16 MiB and stops at `held 15`.
  Result (performance): A/B on this Mac, the benchmark built -O1 with the cap on and with `maxArenaMemoryDisable: true` (byte-identical to the pre-plan compiler's code, see plan-179-B), `--run 50`, two alternating runs each, medians in ms: transient 0.037 / 0.037, mixed 0.041 / 0.041, growshrink 0.093–0.094 / 0.093. Baseline `benchmark/language/baseline/mfb-O1.log`: 0.037 / 0.042 / 0.096. Within noise (Correction 6).
  Result (goldens): before regenerating, `scripts/artifact-gate.sh target/release/mfb all` → 207 diffs, the same 207 files as after plan-179-B, all native dumps; a main-tip (`b2499aca0`) baseline → 0 diffs. Full `scripts/test-accept.sh target/release/mfb` → 50 mismatches, every one a native dump (23 `.nplan`, 18 `.nobj`, 7 `.ncode`, 2 `.mir`) and no output, exit-code or build-log mismatch, so every runtime fixture behaves as before. One fixture (`byte-identity/strings`, macos-aarch64): with `maxArenaMemoryDisable: true` its sha is the golden's; the default build adds the B helper and data plus, for C, the lock imports, the counter/lock data, the charge/refund code in `_mfb_arena_alloc`, the unmap refund in `_mfb_arena_destroy` and the limit store in `_mfb_shutdown`; its only removed lines are register/frame reshuffles inside those two helpers. `bash scripts/regen-native-goldens.sh target/release/mfb` → `179 build(s), 230 golden(s) rewritten, 0 failure(s)`; the working tree then shows 207 changed goldens (macos 72 + 10 app, linux-aarch64 29 + 3 app, linux-x86_64 29 + 3 app, linux-riscv64 29, windows 27 + 5 app). Gate re-run → `2366 golden(s) checked, 0 diff(s)`.
Commit: 3a80e40b9 (goldens: 002ddbde5)

Landing tier (once, at the end of this sub-plan): `scripts/artifact-gate.sh <mfb> all` and a full `scripts/test-accept.sh`, with the golden diff described above.

Landing result (whole plan, after merging main at `7690b181c`): `scripts/artifact-gate.sh target/release/mfb all` → `1715 tests, 1894 build(s), 2378 golden(s) checked, 0 diff(s)`; `scripts/test-accept.sh target/release/mfb` → `acceptance tests passed (1749 test(s) ran)`; `bash scripts/test-fast.sh --test rt_arena_memory_cap --test rt_runtime_helper_callee_saved --test rt_process_spawn_ambient_fds --test rt_thread_block_arena --test rt_worker_arena_release` → `test-fast: ok`; `cargo fmt --all` and `cargo fmt --all --manifest-path repository/Cargo.toml` → no changes. The merge's only conflict was the five `byte-identity/money` sums, regenerated with the merged compiler.

## Validation Plan

- Tests: `tests/runtime/rt_arena_memory_cap.rs`, with three cases:
  1. **Cap enforced.** Set `maxArenaMemoryMiB` to a small value (for example 16). A loop allocates 1 MiB strings and keeps them live. The program catches `ErrOutOfMemory`, prints a line, and exits 0. Without enforcement the loop would exceed 16 MiB and the test fails.
  2. **Disable.** The same program with `maxArenaMemoryDisable: true` allocates past 16 MiB and completes with no error. Proves the disabled build emits no check.
  3. **Worker churn.** Spawn 50 workers, each allocating 8 MiB transiently and exiting, under `maxArenaMemoryMiB: 64`. Total bytes ever mapped is 400 MiB; live mapped bytes stay under 64 MiB only if unmap refunds. The program completes with no `ErrOutOfMemory`. Without the unmap refund the test fails.
- Coverage check: the three cases exercise the map site (1 and 3), the unmap site (3), and the disabled path (2). The refund on mapping failure is exercised by the mutation check in the acceptance list.
- Runtime proof: the three cases on macOS, Linux aarch64, Linux x86-64 and Windows x86-64.
- Doc sync: `04_arenas.md`, `08_program-startup.md`.
- Perf: `./benchmark/language/run.sh --run 50`, `arena` group, against `benchmark/language/baseline/`.
- Final gate (once, at the end): `scripts/artifact-gate.sh <mfb> all` and `scripts/test-accept.sh`.

## Decisions

- The cap counts arena blocks only; perf and debug mappings are excluded — author's call, recorded here. Their memory is not an arena allocation, and charging them would make the budget change when profiling or debugging is enabled.
- Counter protected by a platform mutex, not by a CAS — author's call, recorded here. No backend emits a CAS today (measured in §2), and the mutex is available on every target through `emit_thread_external_call`.
- Charge before map, refund on failure — author's call, recorded here (§3).
- Limit `0` means no enforcement — author's call, recorded here, matching plan-179-B.
- Halt and warn are checked by hand (plan-179-B) — decided by the owner, 2026-10-19. The cap's enforcement is tested automatically by the three runtime cases above, which need no RAM override.

## Notes from the abandoned first attempt (2026-10-20)

A first attempt implemented this sub-plan on branch `worktree-P-179`; the owner discarded it (nothing merged). Findings for the redo:

- **Stale citations.** `src/codegen/memory/arena/arena.rs` no longer exists. The map site is `platform.emit_arena_map` in `lower_arena_alloc`, `src/codegen/memory/arena/arena/alloc.rs` (line 735 on main 4f1c72ccc). The unmap site is `emit_arena_unmap` in `lower_arena_destroy`, `src/codegen/memory/arena/arena/mod.rs` (line 300). The entry shim is `src/codegen/engine/function/entry/mod.rs`. The mutex precedent is `src/codegen/runtime/canvas/thread_lifecycle.rs:515`, not `canvas/mod.rs`.
- **Make lock calls through the thread seam** (`emit_thread_external_call`, as `emit_registry_lock` in `src/codegen/debug/arena/mod.rs` does), so Windows gets `InitializeSRWLock` / `AcquireSRWLockExclusive` / `ReleaseSRWLockExclusive` rather than pthread names. A 64-byte, 8-aligned lock object covers every target (macOS `pthread_mutex_t` is 64 bytes, glibc/musl at most 48, SRWLOCK 8).
- **Hazard: SIGINT/SIGTERM self-deadlock.** `lower_signal_handler` calls `_mfb_shutdown`, which calls `_mfb_arena_destroy`, which, with this plan, takes the map lock for every block refund. If the signal lands on a thread that is inside the charge, refund or destroy critical section, it locks a non-recursive mutex it already holds and the program hangs instead of exiting. The design must handle this, for example by storing 0 into `_mfb_rt_arena_map_limit` at the start of `_mfb_shutdown` (the zero-limit skip then bypasses the lock), or by not refunding during shutdown.
- **Hazard: fork child deadlock.** The spawn child arena-allocates (`emit_copy_to_cstring`, `emit_child_apply_env` in `src/codegen/builtins/process/gen_unix/spawn.rs`). If another thread holds the map lock at `fork()`, the child's copy of the lock stays held, and a child allocation that maps a block hangs forever. A remedy: zero the limit in the child right after `fork()`, or use `pthread_atfork`.
- **The grow path now calls libc.** Before this plan `_mfb_arena_alloc` grew with a bare syscall; the lock call clobbers every caller-saved register when a block is mapped. That is allowed by `.ai/codegen-invariants.md`, but any hand-written caller that wrongly relies on registers surviving `_mfb_arena_alloc` will fail only when the cap is on and a block is mapped.
- **The failed-map refund cannot be tested with a huge request.** The limit is at most RAM (`percent ≤ 100`), so an oversized request is refused by the cap before `mmap` is reached. The attempt forced a real `mmap` failure under `ulimit -v 524288` on Linux only (macOS does not enforce `RLIMIT_AS`; Windows has no equivalent), with `maxArenaMemoryMiB: 700`, `maxArenaMemoryPercent: 100`, two refused 600 MiB requests, then a 256 MiB request that succeeds only if the refund happened. On Kali 2223 that test, and the 50-worker churn test, each failed when its refund was mutated out (`emit_map_refund` → `Ok(())`, `emit_unmap_refund` → early return), so the test designs are sound.
- **Test gaps worth closing:** `cap_is_enforced` should assert how far the program got, not only that `77010001` was caught (a counter that refused every map would also pass). Add a zero-limit runtime case. The charge has no carry check on `counter + map_size`.
- **Spec detail:** the allocator reads the limit on every map and on every unmap, not only on maps.
- **Windows has no Rust toolchain** (box 192.168.1.206, `where cargo` finds nothing), so `cargo test` cannot run there. Plan for cross-built programs instead (`mfb build -target windows-x86_64 <project>`, copy with `scp`, run over `ssh`; console programs work without the `schtasks` route), and record the output of every case.
- **Linux runtime coverage.** The attempt had runtime evidence only on aarch64 (2223). The x86-64 musl box 2227 is QEMU-emulated and too slow for a `cargo test` build (see plan-179-B notes); glibc x86-64 and riscv64 were never run. Plan these rows explicitly.
- **Reference perf numbers (the discarded code, -O1, `run.sh --run 50`, median):** transient 0.037, mixed 0.041, growshrink 0.094, against baseline 0.037 / 0.042 / 0.096. Use this only as a sanity reference for the redo, not as evidence.
- **Run `rustfmt` before the work commit.** The attempt needed a separate formatting commit.

## Corrections

1. **Stale citations** (as the notes said): the map site is `lower_arena_alloc` in `src/codegen/memory/arena/arena/alloc.rs`, the unmap site `lower_arena_destroy` in `arena/mod.rs`, the entry `entry/mod.rs`. The accounting emitters live in `src/codegen/runtime/arena_cap/accounting.rs` (alloc.rs is near its 1000-line ceiling), and the call sites take `arena_cap` and `platform_imports` parameters.
2. **No `pthread_mutex_init`.** The lock is a 64-byte data object initialized to each platform's static initializer (`os_env_lock_init_hex`: `0x32AAABA7` on macOS, zero on Linux, zero = `SRWLOCK_INIT` on Windows), the same image the `os::` env lock and the thread table use. That needs no runtime call, no `pthread_mutex_init` import, and cannot run too late.
3. **Signal and fork hazards handled** by zeroing the limit (the notes' remedy): `_mfb_shutdown` does it first; the POSIX spawn/shell fork child does it before allocating. With the limit 0 the allocator neither charges nor refunds, so it never takes the lock. Added tasks above.
4. **A fourth test.** The failed-map refund cannot be reached with an oversized request (the cap refuses it first), so the plan's mutation check needs a real map failure: `ulimit -v` on Linux (the notes' design). macOS does not enforce `RLIMIT_AS` and Windows has none, so the test is `cfg(target_os = "linux")`.
5. **Remote rows are cross-built except 2223.** 2223 ran `cargo test` itself; 2227 (QEMU, hours per cargo build) and Windows (no Rust toolchain) ran the test programs cross-built on the Mac, which is the same emitted code. 2229 (riscv64) was added. 2228 (x86-64 glibc) was unreachable (`scp: Connection closed`).
6. **Perf check scoped to the arena group as an A/B.** The full `./benchmark/language/run.sh --run 50` builds and runs every group for three languages; the plan's criterion is the `arena` group's mfb rows, and an A/B of the same benchmark with the cap on and off on the same machine fails on the same mistake (a slowed fast path) with less noise than a comparison against a baseline taken on another day.
7. **The cap check counts the whole mapping**, header and grow slack included, as §4.4 says; the limit word is also read on every unmap (spec says so, per the notes).

## Summary

The highest-risk part is the refund on mapping failure and on unmap. A missed refund does not crash: it makes the cap fire early, which looks like a program that allocates too much. The three runtime cases and the mutation check are what catch that. The fast allocation path is untouched; the golden churn is confined to the map and unmap paths.
