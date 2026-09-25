# plan-156-A: the process-global thread table and control blocks

Last updated: 2026-09-24
Overall Effort: huge (>3d) — the whole plan-156 feature: `Thread` and `ThreadWorker`
become `RES` resources backed by a process-global, growable thread table (A–F)
Effort: large (3h–1d)
Depends on: nothing

## The plan-156 feature

Today a `Thread OF M TO O` is a special owned value. It's bound with `LET`/`MUT`,
moved into any function it is passed to, and closed by `waitFor`, all by rules
that exist only for threads (spec §14.3, §16). That model produced three open
bugs:

- **bug-691:** `RES` on a thread is accepted and ignored.
- **bug-692:** a record can hold, and then copy, a `Thread`.
- **bug-693:** `STATE` on a `ThreadWorker` crashes the compiler.

It also produced the wind-example failure: a helper given the handle closed it
on return.

plan-156 makes a thread an ordinary resource, decided with the user on 2026-09-24:

- **The thread table.** A process-global, growable table of control blocks.
  Each holds one lock, a condition variable for blocking waits, the run state,
  the cancel flag, the message and resource queues, and the worker's stored
  result. Everything in a control block lives in memory the block owns. None of
  it is in any thread's arena.
- **Two views, two resources.** `Thread` (the parent view) and `ThreadWorker`
  (the worker view) are each a standard resource record in their holder's
  arena: tag, handle, closed flag, and **their own STATE**, plus a pointer to
  the control block. Neither view's STATE is shared.
- **No user-visible close.** Only the runtime closes the worker view, when the
  worker function returns. At that moment the result is copied into the
  control block and the worker's arena is returned to the OS.
- **Dropping the parent view** at the end of its scope follows the same scope
  rules as every resource. Its hidden drop op cancels and detaches a running
  worker.
- **`thread::waitFor`** waits until the worker view is closed, then copies the
  stored result out. It is repeatable and closes nothing.
- **Passing a view to a function** is a borrowed pointer (§15). `RETURN` moves
  it. It can be stored as a `RES` pointer in a record field, list element or
  map value (§15.6).
- **Transfers.** The parent view may be `thread::transfer`red to another
  thread, but never to its own worker: a new compile error where provable, a
  new runtime error otherwise. A `ThreadWorker` can never be transferred (new
  compile error).
- **Clean break.** `LET`/`MUT` bindings of a thread become a compile error;
  `RES` is the only spelling.

**Checkable outcome of the whole feature:**
- The wind example's original helper works: `withNews(t)` borrows the thread
  and the caller's next `thread::isRunning(t)` still answers.
- bug-691/692/693's reproductions behave as their documents specify.
- A worker's arena is unmapped when it closes.

| Letter | Delivers | Effort |
|---|---|---|
| **A** (this) | the table + control blocks in table-owned memory, queues and result moved in; language unchanged | large |
| B | the worker arena returned to the OS at close | medium |
| C | the two view records, the hidden drop op, borrow-on-pass, repeatable `waitFor`; `Thread` becomes a resource in the verifier (fixes bug-691) | large |
| D | `RES` storage in records, lists and maps, parent-view transfer, the two new errors (fixes bug-692) | large |
| E | per-view STATE and the `STATE` parse rule (fixes bug-693) | medium |
| F | clean break: `LET`/`MUT` threads rejected, corpus migration, spec and man | large |

## This letter

A moves everything the two views will share out of today's 136-byte thread
block, which is allocated from the **parent's** arena, and into a control block
owned by a process-global table. The queues, the run and cancel state, and the
result all move. Language semantics do not change in A: the handle a
`LET t = thread::start(...)` binding holds becomes the control-block pointer,
and every existing thread test passes unchanged.

**Checkable outcome:**
- `rt-behavior/threads/**` and the thread Rust tests pass with no golden
  change except the expected `.ncodesum` shifts.
- A program that starts 5,000 threads sequentially grows the table past its
  initial capacity and completes.
- Message throughput is within 10% of today's (benchmark in Phase 3).

References:

- Spec §16 (`src/docs/spec/language/16_threads.md`) and
  `mfb spec threading control-block`, `threading queue-semantics`,
  `threading thread-runtime-helpers`.
- `.ai/compiler.md`, `.ai/codegen-invariants.md` (register clobbers, arena
  rules), `.ai/arch-abi.md` (per-target calling conventions: the table code runs
  on all five targets), `.ai/testing-gates.md`.
- bug-622 (owner count), bug-646 (pending-free lists), bug-649/650/655 (message,
  resource-plane and seed ownership): the rules A must preserve or supersede.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| Nothing else touches the thread runtime concurrently | `git log --oneline -1 -- src/codegen/runtime/thread` → the commit this plan was written against, or a reviewed later one | MET (2026-09-24) |
| The release compiler is current | `ls -la target/release/mfb` newer than the last `src/` commit | re-check |

plan-156 does not depend on bug-690 (the arena large-block reuse): the control
block's memory is table-owned, not a general-arena client (§4.2).

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again
> before you decide to stop. If you stop, report the status of *all*
> prerequisites.

## 1. Goal

- Every thread operation reaches its shared state through a control block
  allocated by the thread table, not through the parent's arena.
- The table grows without a fixed limit.
- Existing behavior is unchanged.

### Non-goals (explicit constraints)

- No source-language change: no new syntax, no rule codes, no man-page change.
  The spec changes only in `threading/07_control-block.md` and
  `threading/08_queue-semantics.md`, which describe the runtime layout.
- The worker's arena is not freed yet (B).
- The handle's ownership rules stay as they are: the bug-622 owner count,
  move-on-pass and one-shot `waitFor` all stay until C.
- Message copy semantics stay the same (copy on send, copy on receive,
  §16 "Everything that crosses the boundary is copied").

## 2. Current State

All cited by the 2026-09-24 research. The symbols are in
`src/codegen/runtime/thread/runtime_helpers.rs` unless noted.

- **The thread block.** `THREAD_BLOCK_SIZE = 136`, allocated by
  `lower_thread_start_helper` from the **spawning thread's** arena with
  `_mfb_arena_alloc`, together with the worker's arena state and four queues.
  Fields (`THREAD_OFFSET_*`):

  | Offset | Field | Meaning |
  |---|---|---|
  | 0 | STATE | RUNNING/COMPLETED/CLOSED |
  | 8 | CANCELLED | cancel flag |
  | 16 / 24 / 32 | RESULT_TAG / RESULT_VALUE / RESULT_ERROR | the result |
  | 40 / 48 | INBOUND_QUEUE / OUTBOUND_QUEUE | message queues |
  | 56 | OS_HANDLE | OS thread handle |
  | 64 | ENTRY | entry function |
  | 72 | DATA | seed |
  | 80 | ARENA_STATE | the worker's arena state |
  | 88 | PARENT_ARENA_STATE | the parent's arena state |
  | 96 | RESULT_SOURCE | the error's source location |
  | 104 / 112 | RESOURCE_INBOUND_QUEUE / RESOURCE_OUTBOUND_QUEUE | resource queues |
  | 120 | OWNERS | owner count (bug-622) |
  | 128 | DATA_SIZE | seed size (bug-655) |

  Spec drift: `mfb spec threading control-block` still says 128 bytes.
- **Locking.** State is guarded by the **outbound queue's mutex**; there is no
  block lock. The trampoline (`lower_thread_trampoline`) stores the result
  registers into RESULT_* (a raw pointer into the worker's arena), sets
  COMPLETED and broadcasts.
- **waitFor** (the WaitFor arm of `simple_thread_handle_helper`): waits on the
  outbound `not_empty` condvar, reads the result, sets CLOSED and joins. The
  caller copies the value into its own arena at the call site
  (`builder_values.rs:runtime_call_result_is_copied_at_call_site`).
- **Queues.** `THREAD_QUEUE_BLOCK_SIZE = 272`: mutex at 0, condvars at 64/128,
  ring, closed flag, pending-free list at 240.
  - A sender deep-copies a message into its **own** arena (bug-498). The
    reader copies it again at the call site (`copy_value_to_current_arena`,
    `src/codegen/memory/arena/builder_arena_transfer.rs`).
  - The reader parks the previous block on the pending-free list, and the
    sender frees that list on its next write (bug-646).
  - The resource queues carry records moved with
    `copy_resource_to_current_arena` plus their STATE (bug-650).
- **Synchronization primitives.** Only pthread mutex/condvar, through
  `emit_thread_external_call`, mapped on Windows by
  `emit_windows_thread_call` to SRWLOCK/CONDITION_VARIABLE. There are no
  compare-and-swap or atomic-add ops. `abi::store_release_u64` /
  `load_acquire_u64` exist for AArch64 only (`src/target/shared/abi.rs`).
- **Precedents for process-global state.**
  - The debug arena registry (`src/codegen/debug/arena.rs`): memory from
    `emit_arena_map`, a **statically initialised** mutex
    `_mfb_rt_debug_arena_lock` (`os_env_lock_init_hex`, `OS_ENV_LOCK_SIZE=64`),
    and a fixed 1,024 slots.
  - The stdin log (`STDIN_LOG_SYMBOL`): a mutex and condvar, lazy init,
    `CodegenPlatform::emit_heap_alloc`/`emit_heap_free` (malloc, or HeapAlloc on
    Windows).
  - The canvas group table is fixed and lock-free by design, so it is not a
    precedent for a growable table.
  - `rg '"realloc"' src` → none, so no growable global table exists yet.

### Measured populations

| What | Count | Command |
|---|---|---|
| Lines in the thread runtime + thread cleanup | 4,601 (`runtime_helpers.rs` 1,605, `runtime_helpers_thread.rs` 2,400, `builder_thread_cleanup.rs` 587, `mod.rs` 6+3) | `wc -l src/codegen/runtime/thread/*.rs src/codegen/cleanup/thread/*.rs` |
| `THREAD_OFFSET_` occurrences | 179 in 12 files (Rust: 175 in 9; `runtime_helpers_thread.rs` 75, `runtime_helpers.rs` 66, `builtins/thread/lowering.rs` 11, `builder_thread_cleanup.rs` 8) | `rg -o 'THREAD_OFFSET_' src \| wc -l`; `rg -c 'THREAD_OFFSET_' src` |
| `_mfb_rt_thread_*` runtime functions | 18 (+4 `_mfb_rt_stdin_*`) | `mfb build --ncode` of a copy of `tests/byte-identity/thread`, function-header symbols |
| Targets the runtime is emitted for | 5: macos-aarch64, linux-aarch64, linux-x86_64, linux-riscv64, windows-x86_64 | `ls tests/byte-identity/thread/golden/*.ncodesum` |
| Platform branches in the thread runtime | 7 | `rg -n 'PlatformFamily::\|platform.arch\(\)' src/codegen/runtime/thread/runtime_helpers.rs` |
| Thread `rt-behavior` fixtures | 56 files with bindings across `rt-behavior/**` (47 in `threads/`) | census 2026-09-24 (`/tmp/thrcensus/classify.py`) |

### Verified properties

- A resource parameter needs no `RES` keyword: `FUNC w(f AS fs::File)` builds,
  and the file stays open across calls (probe run 2026-09-24).
- The worker arena is never unmapped today. The only caller of
  `_mfb_arena_destroy` is `lower_shutdown`
  (`src/codegen/os/process/process_lifecycle.rs`), for the main arena, and
  bug-547 skips even that when the program uses threads
  (`src/codegen/engine/builder/mod.rs`). `rg -n ARENA_DESTROY_SYMBOL src`.
- UNVERIFIED: whether x86_64 or riscv64 have any lowering for
  `store_release_u64`. A's design uses only mutex-guarded access, so A doesn't
  depend on it.

## 3. Design Overview

**Where design uncertainty concentrates:** giving a control block memory that
belongs to no thread, while still reusing the value-copy machinery that today
targets "the current arena". Phase 1 is the experiment that proves or
disproves it.

**Where correctness risk concentrates:** moving the queues under the block
lock. Every blocking call (`send`, `receive`, `poll`, `accept`, `waitFor`,
cancellation wakeups) changes which mutex it holds and which condvar it waits
on. A lost wakeup is a hang. Phase 3 is scheduled last and behind the
existing timeout and cancel tests.

**Byte-identity is not this letter's gate.** The runtime helpers change, so
every target's `tests/byte-identity/**/*.ncodesum` that contains thread code
is **expected** to diff: `thread`, `resource-xfer-slots`, and any fixture
linking the thread runtime. `.ast`/`.ir` must **not** diff: A changes no
language surface. An `.ast`/`.ir` diff is a bug to root-cause.

Rejected alternatives:
- **Views look the block up by slot index + generation on every op.** Every
  send/receive would take the global table lock. Rejected: the view holds the
  block pointer. The slot index exists for the table's own bookkeeping, and
  the generation is a debug-build guard.
- **A fixed-size table with a limit error.** The user chose growth
  (2026-09-24).
- **Raw heap blocks for results and messages, with new copy primitives.**
  Rejected in favour of a per-block arena (§4.2), which reuses the existing,
  tested deep-copy code.

## 4. Detailed Design

### 4.1 The table

- `_mfb_rt_thread_table`, a process-global data symbol:
  - a statically initialised table mutex (same bytes as
    `_mfb_rt_debug_arena_lock`, `os_env_lock_init_hex`);
  - `slots` (a pointer), `capacity`, `used`, and a free-slot list head.
- The slot array holds **control-block pointers**; the blocks themselves never
  move. Growth doubles the array (`emit_heap_alloc`, copy, `emit_heap_free` of
  the old array) under the table mutex.
- The table lock is taken only at start (allocate a slot) and at final
  release (free the slot). No per-message operation touches it.
- Initial capacity: 64 slots (a pointer each). Growth is tested past 4,096.

### 4.2 The control block

- Allocated with `emit_heap_alloc` (not from any arena).
- Holds the block mutex and condvars, and the fields from §2 that are shared
  state (state, cancel, the result, the queues, the seed size), in a new
  `CONTROL_OFFSET_*` layout.
- Also holds the **block arena**: an arena state owned by the block, mapped
  with the existing `emit_arena_map` and destroyed with `_mfb_arena_destroy`
  when the block is freed.
- Results, queued messages and queued resource records are deep-copied into
  the block arena **while the block lock is held**. The copy runs by pointing
  `ARENA_STATE_REGISTER` at the block arena for the length of the copy
  (`copy_value_to_current_arena`), then restoring it. That save and restore is
  Phase 1's experiment; see `.ai/codegen-invariants.md` on `x19`.
- Per-thread fields stay out of the block: the OS handle, entry, the worker
  arena state, and the parent arena state (until C gives each view its own
  record).

### 4.3 Queues under the block lock

- The four queues become rings inside the block, all guarded by the block
  mutex.
  - `not_empty` and `not_full` become per-direction condvars in the block.
  - The pending-free lists (bug-646) are deleted. A reader copies a message
    out of the block arena into its own arena, then frees the block-arena
    copy immediately. That's legal because the block arena belongs to neither
    thread and is only touched under the lock.
- Blocking waits use `pthread_cond_wait`/`timedwait` on the block condvars
  with the block mutex. They never hold the table mutex.

### 4.4 Lifetime

- A block is freed (slot released, block arena destroyed, block heap-freed)
  when **both** the handle side has released it and the worker has finished.
- In A, "handle side released" is today's owner count reaching 0. C replaces
  it with the parent view's drop.
- A per-block `refs` word (2 at start), decremented under the block lock,
  decides who frees. That bookkeeping is internal; the spec's "no reference
  counting" is about user-visible semantics.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work; `- [~]` for partial, with a line saying what remains;
> strike through moot tasks with evidence. Fill each `Commit:` line as the
> phase lands. **An unticked box means NOT DONE.**

### Phase 1 — block arena experiment (no behavior change)

Proves §4.2's premise before anything depends on it.

- [ ] Add `lower_thread_block_arena_create` / `_destroy` helpers
      (`src/codegen/runtime/thread/`) that map an arena state with
      `emit_arena_map` and destroy it with `_mfb_arena_destroy`.
- [ ] Add a codegen helper that saves `ARENA_STATE_REGISTER`, points it at a
      given arena, runs `copy_value_to_current_arena` for a value, and
      restores it. It is called from nothing yet except a test.
- [ ] Tests: `tests/runtime/rt_thread_block_arena.rs`. Copy a String, a record
      with a list, and a union into a block arena from a worker, destroy it,
      and run 10,000 iterations. A `--debug` build's report shows the block
      arena returned all its bytes (`arena.<n>.live_bytes 0` for the block
      arena; the block arena registers with the debug registry like any
      arena).

Acceptance: the helper round-trips all three value shapes and the block
arena's live bytes return to 0 on every target the Linux/macOS runners reach.
  Check: `cargo test --test rt_thread_block_arena` → pass on macos-aarch64
  locally and linux-aarch64 on box 2223 (est. 5 min; Linux aarch64 is the
  second arena ABI, and x86_64 is covered by the final gate).
Commit: —

### Phase 2 — the table and the control block

The control block replaces the parent-arena thread block. The queues are
still their own objects, now allocated in the block arena.

- [ ] `error_constants.rs`: `CONTROL_OFFSET_*` and `CONTROL_BLOCK_SIZE`.
      Remove the moved `THREAD_OFFSET_*` constants, and re-point every user
      (175 Rust occurrences in 9 files, §2).
- [ ] The `_mfb_rt_thread_table` data symbol, emitted beside the debug
      registry in `src/codegen/engine/builder/mod.rs`, plus slot alloc/free
      helpers with growth (§4.1).
- [ ] `lower_thread_start_helper`: take a slot, allocate the block with
      `emit_heap_alloc`, create the block arena, and put the queues in it.
      The handle value is the block pointer.
- [ ] The trampoline: under the block lock, copy the result into the block
      arena, set COMPLETED, and broadcast.
- [ ] The WaitFor/Drop arms: read the state and result from the block, then
      free the block with §4.4's rule (`refs`) in place of
      `emit_release_thread_plumbing`'s parent-arena frees.
- [ ] Tests: `tests/runtime/rt_thread_table_growth.rs`. Start and `waitFor`
      5,000 threads sequentially, and hold 200 live at once. The debug report
      shows `live_bytes` back at baseline after all are waited.

Acceptance: every existing thread test passes unchanged and the growth test
passes.
  Check: `bash scripts/test-accept.sh target/release/mfb $(mktemp -d) 'rt-behavior/thread*/**'`
  → 0 failures (est. 4 min), plus
  `cargo test --test rt_scope_drop_leaks --test rt_thread_table_growth` → pass
  (est. 3 min).
Commit: —

### Phase 3 — queues under the block lock (largest blast radius last)

- [ ] Fold the four queue rings into the block (§4.3): one mutex, and
      per-direction `not_empty`/`not_full` condvars.
- [ ] Delete the pending-free lists and their reclaimers (bug-646, and the
      bug-650 walks become block-arena destruction).
- [ ] Re-point cancel, drop and timeout wakeups
      (`emit_close_resource_queues`, the Cancel arm) at the block condvars.
- [ ] Benchmark: `benchmark/` thread rows (list them in Corrections when
      run) before and after on macos-aarch64. Throughput within 10%.
- [ ] Update `src/docs/spec/threading/07_control-block.md` and
      `08_queue-semantics.md` to the new layout and locking. Cite the new
      symbols with `[[path:Symbol]]` (`.ai/specifications.md`).

Acceptance: the timeout, cancel and interrupt fixtures pass; the benchmark is
within 10%.
  Check: `bash scripts/test-accept.sh target/release/mfb $(mktemp -d) 'rt-behavior/thread*/**'`
  → 0 failures (est. 4 min); `cargo test --bin mfb spec` → pass (est. 2 min).
Commit: —

## Validation Plan

- Tests: `rt_thread_block_arena.rs`, `rt_thread_table_growth.rs`, and the
  existing thread `rt-behavior` fixtures unchanged.
- Coverage check: the growth test must actually grow the table. Assert in a
  `--debug` build that the table capacity reported at exit is > 64 (add a
  `thread.table_capacity` debug-report key, documented in
  `mfb spec tooling debug-report`).
- Runtime proof: `examples/network-server` (threads per connection) serves
  1,000 requests; the `--debug` report's live bytes return to baseline.
- Doc sync: `threading/07_control-block.md`, `08_queue-semantics.md`,
  `tooling/09_debug-report.md` (the new key).
- Final gate for A (run once): `bash scripts/artifact-gate.sh target/release/mfb all`
  with `.ncodesum` regenerated only for the expected fixtures (est. 15–20 min
  per `.ai/testing-gates.md`; the helpers are emitted per target, so only a
  full-target build catches an ABI slip). The full `cargo test` runs once, at
  the end of F.

## Open Decisions

- The block arena's copy-in through a re-pointed `ARENA_STATE_REGISTER`
  (recommended) vs a dedicated "copy into arena X" entry in
  `builder_arena_transfer.rs`. Decide on Phase 1's result.

## Corrections

## Summary

A is plumbing with the language untouched. Its risk is the lock
consolidation in Phase 3 (a lost wakeup is a hang) and the arena-register
save and restore in Phase 1; both are proven before anything depends on
them.
