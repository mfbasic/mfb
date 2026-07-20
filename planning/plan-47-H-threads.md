# plan-47-H: Threads (thread:: over CreateThread + SRWLOCK/CONDITION_VARIABLE)

Last updated: 2026-07-19
Effort: large (3h–1d)  — the top of the sub-plan band; the four phases below are individually medium and land separately.
Depends on: **per phase — the single header dependency was wrong.**
  - H1 (collapse 3 emission routes onto one `sync_symbol`): **nothing**
  - H2 (rename-compatible Win32 arms + init-check gating): **nothing**
  - H3 (spawn / release / timed wait): **plan-47-B** — the shadow space + outgoing
    stack-arg tail. This document already said so at §Phase 3 and §Open Decisions;
    the header contradicted its own body.
  - H4 (advertise `thread.*`, kernel32 imports, fixtures): **plan-47-D**

Make the `thread::` surface work on `windows-x86_64` by adding a **platform switch**
to the shared thread trampoline and sync helpers, so that every place that today
emits a `pthread_*` call emits the corresponding Win32 primitive instead
(`CreateThread`/`CloseHandle`, `SRWLOCK`, `CONDITION_VARIABLE`), and advertising
`thread.*` in the Windows backend's `runtime_calls`.

The single behavioral outcome: the `tests/rt-behavior/threads/**` fixtures (33
directories today) and the stdin-broadcast paths built for `windows-x86_64` and run
on Windows/Wine produce stdout/stderr and exit codes **byte-identical** to the
linux-x86_64 build of the same programs — including the resource-plane and
`STATE`-carrying fixtures (`thread-transfer-bidirectional-rt`,
`thread-transfer-state-rt`, `thread-send-file-ownership-rt`) — while every existing
target's emitted bytes stay unchanged.

**Correction (2026-07-20): this sub-plan is NOT different in kind from 47-G/G.**
The original text claimed those "add *new* methods to the Windows `CodegenPlatform`"
while only F edits shared lowering. Measured, that is false: there is no `emit_socket`
or `emit_connect` on the trait at all — G rewrites **32** hardcoded POSIX symbol
literals across `shared/code/net/{mod,io,poll}.rs`, and E rewrites **6** across
`io_helpers.rs` and `term.rs`. G is the same work as this sub-plan at 38% the scale;
only 47-C touches no shared code. The technique below (collapse to one chokepoint,
prove zero-byte diff, then add the Windows arm) is **the reusable pattern for the whole
feature**, not an F-specific device — clone it as G1 and E1.

What remains true is the consequence: **this edits shared lowering code that every
backend compiles through** — `src/target/shared/code/runtime_helpers.rs`,
`runtime_helpers_thread.rs`, `stdin_broadcast.rs`, and `os.rs`. A mistake here is
not a Windows bug, it is a *linux-aarch64* bug. The byte-identical guard
(`scripts/artifact-gate.sh`) is therefore load-bearing on every commit of this
plan, not just at the end.

References (read before starting):

- `planning/plan-47-windows-x86_64.md` — the master; §Phase F, and the hard
  non-goal "No change to any existing target's output bytes."
- `src/target/shared/code/runtime_helpers.rs` — thread control block + queue
  layout (`:3`–`:60`), `emit_thread_external_call` (`:70`), `thread_symbol`
  (`:62`), `lower_thread_start_helper` (`:383`), `lower_thread_trampoline`
  (`:723`).
- `src/target/shared/code/runtime_helpers_thread.rs` — `ThreadSimpleOp` (`:3`),
  `emit_thread_deadline` (`:11`), `simple_thread_handle_helper` (`:63`), the
  queue read/write helpers with `pthread_cond_timedwait` (`:740`, `:936`,
  `:1262`).
- `src/target/shared/code/stdin_broadcast.rs` — `emit_libc` (`:86`), the log
  mutex/cond usage (27 `pthread_*` sites).
- `src/target/shared/code/os.rs:41`–`:84` — `OS_ENV_LOCK_SYMBOL`,
  `OS_ENV_LOCK_SIZE`, `os_env_lock_init_hex`.
- `src/target/shared/code/error_constants.rs:488`–`:537` — the stdin broadcast log
  block map; `:303` `SHUTDOWN_SYMBOL`.
- `src/target/linux_common/plan.rs:406`–`:441` — the per-call pthread import set
  the Windows plan mirrors; `src/target/macos_aarch64/plan.rs:620` for the second
  precedent.
- plan-54 / bug-257 (`planning/old-plans/`, `bugs/completed-bugs/`) — the thread
  resource plane + `STATE` model this must preserve exactly.

## Prerequisites

Per phase, matching the dependency split in the header:

| Phase | Must be true | Command | Status 2026-07-20 |
|---|---|---|---|
| H1, H2 | Byte-identity goldens for all four existing targets | `find tests -path '*/golden/*' -name '*.ncode*' \| while read f; do b="${f##*/}"; b="${b%.*}"; echo "${b##*.}"; done \| sort -u` | **NOT MET — `linux-riscv64` has 0** |
| H3 | plan-47-B has landed (shadow space + outgoing stack-arg tail) | `rg -n 'shadow_space_bytes' src/` | **NOT MET** |
| H4 | plan-47-D has landed (a runnable `.exe` and import tables) | `ls src/target/win_x86_64/plan.rs` | **NOT MET** |

> **NOTE — the Status column is a snapshot; the Command column is the truth.** Re-run
> every row before you continue and again before you decide to stop. Never act on a
> status you did not just verify. **If you stop, report the status of every row**, not
> only the one that blocked you.

**H1 and H2 block on nothing but row 1** — they are inert shared refactors whose entire
proof is a zero-byte diff, so a target with no goldens makes that proof vacuous.


## 1. Goal

- Every `pthread_*` symbol emitted by shared thread/sync/stdin lowering is
  selected through **one** platform-aware mapping function, and that function
  returns Win32 primitives for `windows-x86_64` and today's exact `pthread_*` /
  `_pthread_*` names for every other target.
- The Windows realization is:
  - `pthread_create` → `CreateThread` (handle **returned**, stored to
    `THREAD_OFFSET_OS_HANDLE`); `pthread_attr_init` / `pthread_attr_setstacksize`
    → folded into `CreateThread`'s `dwStackSize` parameter (no attr object);
    `pthread_detach` → `CloseHandle`.
  - `pthread_mutex_init` → `InitializeSRWLock`; `pthread_mutex_lock` /
    `pthread_mutex_unlock` → `AcquireSRWLockExclusive` /
    `ReleaseSRWLockExclusive`.
  - `pthread_cond_init` → `InitializeConditionVariable`; `pthread_cond_wait` /
    `pthread_cond_timedwait` → `SleepConditionVariableSRW`;
    `pthread_cond_signal` → `WakeConditionVariable`; `pthread_cond_broadcast` →
    `WakeAllConditionVariable`.
  - `clock_gettime` (used only to build the `cond_timedwait` deadline) →
    `GetTickCount64`, with the deadline expressed the way the Windows wait
    primitive needs it (§5).
- `src/target/win_x86_64/mod.rs` advertises the `thread.*` block of
  `runtime_calls` (the 12 entries `linux_common::RUNTIME_CALLS` carries at
  `src/target/linux_common/mod.rs:186`–`:197`), and `plan.rs` declares the
  kernel32 import set for those calls, mirroring
  `src/target/linux_common/plan.rs:406`.
- `tests/rt-behavior/threads/**` and the `thread::openStdIn`/`closeStdIn`
  fixtures pass on Windows/Wine with output byte-identical to linux-x86_64.

### Non-goals (explicit constraints)

- **No change to any existing target's output bytes.** macos-aarch64,
  linux-aarch64, linux-x86_64, linux-riscv64 must be byte-identical after every
  phase. This is the master's hard non-goal and it is *this* sub-plan's dominant
  risk, because the edits are in shared code. `scripts/artifact-gate.sh` on every
  commit.
- **No change to the thread control-block or queue layout for existing targets.**
  `THREAD_BLOCK_SIZE = 120` and the field offsets at `runtime_helpers.rs:3`–`:27`,
  and `THREAD_QUEUE_BLOCK_SIZE = 248` with the 64-byte primitive reserves at
  `:42`–`:60`, stay exactly as they are (§4).
- **No change to the thread resource plane or the `STATE` model.** The two
  direction-isolated resource queues (`THREAD_OFFSET_RESOURCE_INBOUND_QUEUE = 104`,
  `THREAD_OFFSET_RESOURCE_OUTBOUND_QUEUE = 112`, `runtime_helpers.rs:26`–`:27`)
  and the `STATE` deep-copy at `builder_arena_transfer.rs:297`
  (`copy_resource_to_current_arena`) are untouched. Windows changes *which
  primitive* a queue blocks on, never *which queue* (§6).
- **No `TerminateThread`, ever** — not in cancel, not in drop, not in shutdown
  (§7).
- **No new `CodegenPlatform` required method.** If a hook is unavoidable it is
  added with a default body so the other four backends compile and emit
  unchanged (master §Compatibility).
- **No semantic change to `thread::` for any platform.** Cancellation stays
  cooperative; `waitFor` stays "wait on the condvar for `THREAD_STATE_COMPLETED`,
  then release the OS handle". Windows does not get a different concurrency model.
- **No language, IR, resolver, or `builtins/thread.rs` change.** This is entirely
  in the lowering + plan layers.

## 2. Current State

Every claim below is cited to code read for this plan.

### 2.1 The pthread call sites and their two chokepoints

Shared lowering emits pthread symbols from four files:

| file | `"pthread_*"` literals |
| --- | --- |
| `src/target/shared/code/runtime_helpers_thread.rs` | 41 |
| `src/target/shared/code/stdin_broadcast.rs` | 27 literals across **32** `emit_libc` call sites |
| `src/target/shared/code/runtime_helpers.rs` | **21** (draft said 17) |
| `src/target/shared/code/os.rs` | 2 |

They reach the encoder through **three** routes, not one:

1. `emit_thread_external_call` (`runtime_helpers.rs:70`) — 42 calls in
   `runtime_helpers_thread.rs`, 16 in `runtime_helpers.rs`. It calls
   `thread_symbol` (`:62`), which today is the *only* platform switch in the
   whole thread path:
   ```rust
   if platform.target() == "macos-aarch64" { format!("_{name}") } else { name.to_string() }
   ```
2. `emit_libc` (`stdin_broadcast.rs:86`) — a second, independent local helper for
   the broadcast log.
3. `platform.emit_libc_call(...)` called directly, with the symbol spelled inline
   — `os.rs:102` (`pthread_mutex_lock`) and `os.rs:138`
   (`pthread_mutex_unlock`).

`lower_thread_start_helper` bypasses all three for the spawn sequence, emitting
`abi::branch_link(...)` + `external_branch(...)` inline against locally computed
symbol names (`runtime_helpers.rs:612`–`:683`).

**`thread_symbol` is the seam this plan generalizes.** It already proves the
pattern (a per-target symbol rewrite in one place) and it already proves the
hazard (`macos-aarch64` is matched by a string compare, so a new target that
forgets to extend it silently inherits the Linux spelling).

### 2.2 Thread spawn

`lower_thread_start_helper` (`runtime_helpers.rs:383`) allocates the control
block, the child arena state, and **four** queues — data inbound/outbound and
resource inbound/outbound (`:552`–`:610`) — then:

- Reserves 64 bytes of stack for a `pthread_attr_t` (`ATTR_OFFSET`, `:399`–`:400`,
  comment: "64 bytes covers musl/glibc (56) and macOS (64)").
- Calls `pthread_attr_init`, then `pthread_attr_setstacksize` with `8 * 1024 *
  1024` (`:630`–`:646`) — an explicit 8 MiB worker stack, because musl's 128 KiB
  default overflowed on the regex engine's ~230 KiB frame (`:622`–`:629`).
- Calls `pthread_create(&cb->os_handle, &attr, trampoline, cb)` — note
  `abi::add_immediate(abi::ARG[0], "%v9", THREAD_OFFSET_OS_HANDLE)` at `:649`:
  the handle is written **through a pointer** by the callee.
- Treats a non-zero `RET[0]` as failure (`:684`–`:686` → `spawn_error` →
  `ErrInterrupted`).

### 2.3 The trampoline

`lower_thread_trampoline` (`runtime_helpers.rs:723`) is machine-floor code with a
**hand-managed frame** (`FRAME_SIZE = 80`, `:737`, locals at `sp+0..72`,
`:738`–`:746`) that the register allocator never runs over — the header comment at
`:728`–`:736` explains the scratch-register confinement to `abi::SCRATCH[4]`/`[5]`
precisely because the x86 residual-scratch pool would otherwise alias
`abi::CURRENT_THREAD`. It:

- Parks `LR`/arena/`CURRENT_THREAD`/closure, moves `ARG[0]` (the control block)
  into `CURRENT_THREAD`, loads the worker's arena and closure, and calls the
  worker entry (`:749`–`:768`).
- Stores the four result registers to the frame (`:769`–`:781`).
- Optionally auto-unsubscribes from the stdin broadcast log (`:788`–`:795`).
- Closes and broadcasts the data inbound queue (`:796`–`:872`) and then **both
  resource-plane queues** in a loop (`:876`–`:879` and following), so a parent
  parked in `transferResource`/`acceptResource` wakes.
- Returns `0` in `abi::RET[0]` and restores the frame (`:1048`–`:1056`).
- Ends with a hard assertion that the instruction stream names **no physical
  register** (`regalloc::find_physical_operand`, `:1060`–`:1065`, plan-34-D).

### 2.4 Wait / cancel / drop / poll

`simple_thread_handle_helper` (`runtime_helpers_thread.rs:63`), `FRAME_SIZE = 48`,
locals at `sp+8..40` (`:69`–`:75`):

- **`WaitFor`** (`:144`) locks the outbound queue mutex, then loops on
  `pthread_cond_wait(¬Empty, mutex)` until `THREAD_OFFSET_STATE` reads
  `COMPLETED` or `CLOSED` (`:163`–`:186`), harvests the four result words, marks
  the block `CLOSED`, unlocks, and calls **`pthread_detach`** on the handle loaded
  **by value** from `THREAD_OFFSET_OS_HANDLE` (`:230`–`:243`).
  **There is no `pthread_join` anywhere in the tree** — completion is signalled
  through the condvar, and `detach` only releases the OS-side thread record.
- **`Cancel`** (`:308`) sets the cancelled flag and closes + broadcasts all four
  queues. The comment at `:302`–`:307` records bug-205: cancel/drop originally
  touched only the two *data* queues, so a worker parked in a blocking
  `acceptResource` never woke — a permanent hang, and on drop a detached leaked
  thread.
- **`Drop`** (`:494`) mirrors cancel and also `pthread_detach`s (`:663`).
- **`Poll`** (`:670`) and the queue read/write helpers use
  `pthread_cond_timedwait` (`:740`, `:936`, `:1262`) with an **absolute**
  deadline built by `emit_thread_deadline` (`:11`): `clock_gettime(0, &ts)` then
  `ts += timeoutMs` normalized to nsec (`:22`–`:59`). The call site treats a
  **non-zero** return as "timed out" (`:741`–`:743`, `:937`–`:939`).
- `pthread_cond_signal` is used at `:970` and `:1312`.

### 2.5 Sync-object storage

Three fixed-size reserves, all currently sized for the largest
`pthread_mutex_t`/`pthread_cond_t` across supported libcs:

- **Queue block** (`runtime_helpers.rs:42`–`:60`): mutex at offset 0,
  `notEmpty` cond at 64, `notFull` cond at 128, then the scalar fields
  (`capacity` 192, `count` 200, `head` 208, `tail` 216, `closed` 224, `values`
  232, `pendingFree` 240), `THREAD_QUEUE_BLOCK_SIZE = 248`. The three primitives
  are addressed implicitly as `queue + {0, 64, 128}`.
- **Stdin broadcast log** (`error_constants.rs:499`–`:500`):
  `STDIN_LOG_MUTEX_OFFSET = 0`, `STDIN_LOG_CV_OFFSET = 64`, then
  `STDIN_LOG_INITIALIZED_OFFSET = 128` and the rest of the documented block map
  through `STDIN_LOG_REGISTRY_OFFSET = 208`. The comment at `:489`–`:490` states
  the 64-byte reserve rationale, and `:491`–`:497` records bug-326-D1: the mutex
  offset constant has no reader because the log's *base address* is passed as
  `ARG[0]`, i.e. the mutex is addressed implicitly at offset 0.
- **`os::` env/pwd lock** (`os.rs:41`–`:47`): `OS_ENV_LOCK_SIZE = 64`, a single
  statically-initialized global; `os_env_lock_init_hex` (`:77`) emits all zeros
  except macOS, which gets `_PTHREAD_MUTEX_SIG_init = 0x32AAABA7` in the first
  word (`:79`–`:82`).
- Plus the 64-byte `pthread_attr_t` stack scratch in
  `lower_thread_start_helper` (`:399`–`:400`).

> The master's Phase F cites "`os.rs:32,61`" for the mutex/cond storage sizes.
> **That citation is wrong.** `os.rs:31` is `ERRNO_ENOMEM` and `:61` is blank;
> the real size constant is `os.rs:47`, and the *primary* mutex/cond storage is
> `runtime_helpers.rs:42`–`:43` plus `error_constants.rs:499`–`:500`. Use the
> citations in this document.

### 2.6 The plan (import) layer is already per-backend

`src/target/linux_common/plan.rs:406`–`:441` maps the whole `thread.*` call set to
a 13-symbol import list bound to `self.libpthread()`;
`src/target/macos_aarch64/plan.rs:620` is the second precedent. Nothing shared
needs to change to give Windows a different list — `src/target/win_x86_64/plan.rs`
declares its own, exactly as 47-D did for the floor.

### 2.7 What `_mfb_shutdown` does — and does not do

`lower_shutdown` (`entry_and_arena.rs:1880`–`:1931`, symbol at
`error_constants.rs:303`) reads the main-arena global, zeroes it (idempotency
gate), then optionally drains the stdout buffer (`:1907`–`:1910`), turns the
terminal off (`:1911`–`:1914`), and destroys the arena (`:1915`–`:1918`).
**It performs no thread operation at all** — no join, no cancel, no handle
cleanup. Worker threads are torn down by process exit. The signal handler
(`:1938`) runs the same body and then `_exit`s.

## 3. Design Overview

Four layered pieces. The first is pure refactoring with zero behavior change and
is where almost all of the byte-identity risk is retired.

1. **One chokepoint (Phase 1).** Introduce a single
   `sync_symbol(platform, LogicalSync) -> &'static str` mapping in
   `runtime_helpers.rs` beside `thread_symbol`, keyed by a small
   `LogicalSync` enum (`MutexInit`, `Lock`, `Unlock`, `CondInit`, `CondWait`,
   `CondTimedWait`, `CondSignal`, `CondBroadcast`, `ThreadCreate`, `ThreadRelease`,
   `Clock`). Route all three existing routes through it: `emit_thread_external_call`,
   `stdin_broadcast::emit_libc`, and the two inline `os.rs` sites. For every
   non-Windows target it returns exactly today's string (including the macOS
   leading underscore), so the emitted bytes are provably unchanged — this is the
   phase `scripts/artifact-gate.sh` must certify with a **zero-byte diff**.

2. **The Windows arm of the mapping (Phase 2).** Add the Win32 names. The three
   *shape-compatible* families — lock/unlock, cond init, signal/broadcast — are a
   pure rename: `AcquireSRWLockExclusive(PSRWLOCK)` and
   `ReleaseSRWLockExclusive(PSRWLOCK)` take one pointer exactly like
   `pthread_mutex_lock`/`unlock`; `WakeConditionVariable` /
   `WakeAllConditionVariable` take one pointer exactly like
   `pthread_cond_signal`/`broadcast`. The two `*_init` calls become
   `InitializeSRWLock` / `InitializeConditionVariable`, which return `void` —
   handled by §5.1.

3. **The three shape-incompatible primitives (Phase 3).** Spawn (§5.2), release
   (§5.3), and timed wait (§5.4) do not map by rename; each gets an explicit
   Windows arm in the emitting helper. This is where the correctness risk
   concentrates.

4. **Advertisement + fixtures (Phase 4).** `runtime_calls` + the kernel32 import
   set + the rt-behavior runs.

**Where the risk concentrates.** In descending order:

- **Shared-code byte identity.** 85 call sites in code compiled for all five
  targets. Mitigated by making Phase 1 a mechanical, provably-inert refactor
  gated on a zero-byte artifact diff before any Windows string exists.
- **The timed wait (§5.4).** Three divergences at once — relative vs absolute
  timeout, milliseconds vs `timespec`, and *inverted* success polarity. A wrong
  polarity turns "timed out" into "ready" and the queue helper reads an empty
  queue. This is a silent-wrong-value class, the worst one per `.ai/compiler.md`.
- **Frame shadow space (§5.5).** The thread helpers use hand-managed frames with
  hardcoded `sp`-relative local offsets and then make external calls. Under Win64
  the caller must reserve 32 bytes at `[rsp+0..31]` at the call. If 47-B does not
  supply a shadow-aware locals base, every one of these frames silently
  corrupts its own locals on the first external call.
- **The four-queue invariant (§6).** Any Windows arm that handles the data queues
  but not the resource queues reproduces bug-205 (hang) and, via a leaked
  detached thread, bug-257's lifetime class.

**Rejected alternatives.**

- *Emit a Win32 shim layer — real functions named `pthread_mutex_lock` etc.
  compiled into the image, wrapping SRWLOCK.* Rejected: `mfb` has no C toolchain
  in the build (master non-goal), so the shim would itself have to be
  hand-lowered — strictly more machine-floor code than mapping the names, with an
  extra call frame per lock on the hottest path in the runtime.
- *Link a pthreads-for-Windows implementation (pthreads-w32, or the MSVCRT
  `_beginthreadex` family).* Rejected: an external dependency, and it drags in a
  CRT — both hard non-goals of the master plan (§Non-goals).
- *Use `CRITICAL_SECTION` instead of `SRWLOCK`.* Rejected: `CRITICAL_SECTION` is
  a 40-byte struct that **requires** a runtime `InitializeCriticalSection` and a
  matching `DeleteCriticalSection`, and it is recursive — a different semantics
  from `pthread_mutex_t` default (non-recursive). `SRWLOCK` is one pointer,
  statically zero-initializable (`SRWLOCK_INIT` is `{0}`), non-recursive, and is
  the only lock type `SleepConditionVariableSRW` pairs with. The master listed
  `CRITICAL_SECTION` as an alternative; this plan closes that fork in favour of
  `SRWLOCK`.
- *Resize the 64-byte primitive reserves down to 8 bytes for Windows.* Rejected —
  see §4.
- *Use `WaitForSingleObject` to implement `thread::waitFor`.* Rejected — see §5.3.
  It is in the master's Phase F sketch but the repo's `waitFor` does not join.

## 4. Sync-object storage: parameterize by *not* varying it

The master's Phase F says "resize the mutex/cond storage for the Windows
objects." **The correct engineering answer is not to resize it.**

`SRWLOCK` is `typedef struct { PVOID Ptr; } SRWLOCK` — 8 bytes on x64.
`CONDITION_VARIABLE` is `typedef struct { PVOID Ptr; } CONDITION_VARIABLE` — also
8 bytes. Both fit inside the existing 64-byte reserves with 56 bytes to spare.
The reserves are already **supersets**, chosen (`error_constants.rs:489`,
`os.rs:43`–`:46`) as "the largest primitive across every supported platform", not
as an exact fit for any one of them.

So the layout is parameterized per platform in the only way that satisfies the
master's hard non-goal by construction: **the offsets are platform-invariant, and
Windows uses a prefix of each reserve.** Concretely, and with no code change at
all in Phase 1–3:

- Queue block: `SRWLOCK` at `queue+0`, `notEmpty` `CONDITION_VARIABLE` at
  `queue+64` (`THREAD_QUEUE_NOT_EMPTY_OFFSET`), `notFull` at `queue+128`
  (`THREAD_QUEUE_NOT_FULL_OFFSET`); scalar fields from 192 unchanged;
  `THREAD_QUEUE_BLOCK_SIZE` stays 248.
- Stdin log: `SRWLOCK` at `log+0` (`STDIN_LOG_MUTEX_OFFSET`),
  `CONDITION_VARIABLE` at `log+64` (`STDIN_LOG_CV_OFFSET`); the rest of the block
  map from 128 unchanged.
- `os::` env lock: `OS_ENV_LOCK_SIZE` stays 64; the `SRWLOCK` occupies the first
  8 bytes.
- The 64-byte `pthread_attr_t` scratch (`runtime_helpers.rs:399`) becomes dead on
  Windows (no attr object exists) — the slot stays reserved rather than being
  conditionally removed, so `FRAME_SIZE` is identical on all targets.

The **only** documentation-level work in Phase 3 is to update three comments
(`runtime_helpers.rs:399`, `error_constants.rs:489`, `os.rs:43`) to say the
reserve also covers Windows' 8-byte `SRWLOCK`/`CONDITION_VARIABLE`, and to
generalize `os_env_lock_init_hex` (`os.rs:77`) — which already emits all zeros for
every non-macOS target, and all-zero **is** `SRWLOCK_INIT`, so Windows needs no
new branch, only a comment saying why the existing branch is correct.

Cost of not shrinking: 168 wasted bytes per queue block × 4 queues = 672 bytes per
spawned thread, plus 56 in the log and 56 in the env lock. That is negligible next
to the 8 MiB worker stack, and it buys a guaranteed-zero-diff for four shipping
targets.

**If a future platform ever needs more than 64 bytes**, the seam is a
`CodegenPlatform::sync_primitive_reserve() -> usize` with `fn ... { 64 }` as the
default body, and every one of the offsets above computed from it in a single
`const fn`. That hook is **not** added by this plan (adding it now would be a
hook with one caller and no divergence to justify it — the trap recorded in the
`bug-350` memory note). It is written down here so the next implementer does not
have to rediscover it. See Open Decisions.

## 5. The Windows realization, primitive by primitive

### 5.1 The void-returning initializers

`pthread_mutex_init` and `pthread_cond_init` return `int`, and
`emit_thread_queue_alloc` (`runtime_helpers.rs:83`) checks each one:
`compare_immediate(abi::RET[0], "0"); branch_ne(&init_error)` at `:192`–`:193`,
`:209`–`:211`, `:226`–`:228`, routing a non-zero result to an
`ErrInterrupted` result.

`InitializeSRWLock(PSRWLOCK)` and `InitializeConditionVariable(PCONDITION_VARIABLE)`
return `void` — `rax` is undefined after the call, so the existing check would
branch on garbage.

**Resolution:** in `emit_thread_queue_alloc`, gate the three
`compare/branch_ne(&init_error)` pairs on a platform predicate; on Windows emit
the call and skip the check. Do **not** substitute `move_immediate(RET[0], 0)`
before the compare — that emits two dead instructions on the hot spawn path and
buries the reason. The `init_error` label and its error block stay (they are still
reachable on every other target), so nothing else in the helper changes.

Note the second argument: both pthread `*_init` calls pass a NULL attr in
`ARG[1]` (`:180`, `:197`, `:214`). The Win32 initializers take one argument;
leaving `ARG[1]` staged is harmless under Win64 (`rdx` is caller-saved and
ignored), so the staging instructions may stay to keep the two paths' instruction
streams as close as possible.

### 5.2 Spawn: `CreateThread`

```
HANDLE CreateThread(LPSECURITY_ATTRIBUTES lpThreadAttributes,   // rcx: NULL
                    SIZE_T                dwStackSize,          // rdx: 8 MiB
                    LPTHREAD_START_ROUTINE lpStartAddress,      // r8:  trampoline
                    LPVOID                lpParameter,          // r9:  control block
                    DWORD                 dwCreationFlags,      // stack: 0
                    LPDWORD               lpThreadId);          // stack: NULL
```

Differences from the `pthread_create` sequence at `runtime_helpers.rs:612`–`:691`:

- **Six arguments, two on the stack.** Args 5 and 6 go above the shadow space.
  This is the first consumer in shared lowering of the x86 outgoing stack-arg
  tail that 47-B must implement (`abi.rs:39`–`:58`, `OUTGOING_ARGS_BASE`;
  `abi.rs:16`–`:24` currently errors past the register cap). If 47-B has not
  landed that, 47-H is blocked — this is the concrete dependency.
- **No attr object.** The `pthread_attr_init` / `pthread_attr_setstacksize` pair
  (`:635`–`:646`) is skipped entirely; the 8 MiB value moves from
  `setstacksize`'s argument into `dwStackSize`. The rationale comment at
  `:622`–`:629` (musl's 128 KiB default vs the regex engine's ~230 KiB frame)
  applies verbatim to Windows, whose default is 1 MiB — well under the 230 KiB +
  headroom the comment describes, so the explicit 8 MiB is **required**, not
  cosmetic. Note `dwStackSize` on Windows is *reserve* (address space), matching
  the comment's "reserved lazily (virtual)" claim.
- **The handle is returned, not written through a pointer.** pthread takes
  `&cb->os_handle` in `ARG[0]` (`:649`). Windows returns the `HANDLE` in `rax`,
  so the sequence must `store_u64(abi::RET[0], "%v9", THREAD_OFFSET_OS_HANDLE)`
  after the call — and `%v9` must be **reloaded from `CB_OFFSET`** first, because
  every external call destroys the caller-saved file (`.ai/compiler.md`, "Native
  Codegen Register Lifetimes").
- **Failure polarity is inverted.** `pthread_create` returns 0 on success
  (`:684`–`:686`: `branch_ne → spawn_error`). `CreateThread` returns `NULL` on
  failure, so the Windows arm is `compare_immediate(RET[0], "0"); branch_eq(&spawn_error)`.
  Both routes land on the same `spawn_error` block → `ErrInterrupted`, so the
  observable error is identical.

### 5.3 The trampoline signature, and why no shim function is needed

pthread's start routine is `void *(*)(void *)`; `CreateThread`'s is
`DWORD (*)(LPVOID)` (`LPTHREAD_START_ROUTINE`). Under Win64 that is: one
pointer-sized argument in `rcx`, return value in `eax`.

The trampoline as written is *already* both. It reads its single argument from
`abi::ARG[0]` (`:756`) and returns by `move_immediate(abi::RET[0], "Integer", "0")`
(`:1049`). Once 47-B realizes `abi::ARG[0]` as `rcx` and `abi::RET[0]` as `rax`
for this target, the C-level signature difference **disappears** — `DWORD` is the
low 32 bits of the same `rax` the pthread version returns `NULL` in, and the value
written is `0` either way. **No separate `DWORD` shim function is required**, and
none should be added; a shim would be a second machine-floor frame that the
plan-34-D physical-register assertion (`:1060`) would have to police as well.

Two things *do* have to be true, and both are 47-B's contract (§5.5):

1. The trampoline's hand-managed frame must reserve Win64 shadow space for the
   external calls it makes (it makes at least six: one lock, two broadcasts, one
   unlock, times the resource-queue loop).
2. Win64's callee-saved bank is `{rbx, rbp, rdi, rsi, r12–r15, xmm6–xmm15}` —
   wider than SysV's. The trampoline already saves `abi::CURRENT_THREAD`,
   `ARENA_STATE_REGISTER`, and `CLOSURE_ENV_REGISTER` in its own frame
   (`:753`–`:755`), and declares `link_register` + `CURRENT_THREAD` in
   `CodeFrame::callee_saved` (`:1077`–`:1080`). Because it is the *callee* of
   `CreateThread`, it must not destroy `rsi`/`rdi` either. Phase 3 must confirm
   which physical registers `abi::SCRATCH[4]`/`[5]` realize to under the Win64
   register model (they are `x13`/`x14` → `r9`/`r10` under SysV per the comment
   at `:735`–`:736`); `r9`/`r10` are caller-saved under Win64 too, so the
   confinement argument survives — but it must be *checked*, not assumed, because
   47-B changes `map_scratch_register`.

### 5.4 Timed wait — the highest-risk conversion

Today (`runtime_helpers_thread.rs:11`–`:59`, call sites `:740`, `:936`, `:1262`):

```
clock_gettime(CLOCK_REALTIME=0, &ts);      // ts is an ABSOLUTE deadline
ts += timeoutMs;                            // normalized into sec/nsec
loop { ... pthread_cond_timedwait(&cond, &mutex, &ts); if (ret != 0) -> timed out }
```

Windows:

```
BOOL SleepConditionVariableSRW(PCONDITION_VARIABLE cv,   // rcx
                               PSRWLOCK             lock, // rdx
                               DWORD                dwMs, // r8  — RELATIVE
                               ULONG                flags);// r9  — 0 = exclusive
```

Three divergences, each independently capable of a silent wrong answer:

1. **Relative, not absolute.** The wait loop re-enters the wait with the *same*
   `ts` after every spurious wake-up, which is correct for an absolute deadline
   and **wrong** for a relative one: passing the original `timeoutMs` each time
   restarts the clock, so a program with repeated spurious wakes waits forever.
   Resolution: keep `emit_thread_deadline`'s structure but make it compute a
   **deadline in milliseconds** on Windows — `GetTickCount64()` (a `ULONGLONG`
   millisecond tick, monotonic, no struct, kernel32) plus `timeoutMs`, stored in
   the *first* word of the existing `TIMESPEC_OFFSET`/`ERROR_OFFSET` scratch slot
   (the slot is 16 bytes for a `timespec`; 8 are enough here, so no frame change).
   Then at each wait site the Windows arm recomputes
   `dwMs = deadline - GetTickCount64()`, clamped at 0, and passes it in `r8`.
2. **Milliseconds, not `timespec`.** Handled by the above — and it *simplifies*
   the deadline helper on Windows: the whole sec/nsec normalization at
   `:39`–`:57` (the divide, the multiply-subtract, the 1e9 carry) is replaced by
   one add, because `timeoutMs` is already milliseconds.
3. **Inverted polarity.** `pthread_cond_timedwait` returns **0 on success**,
   non-zero (`ETIMEDOUT`) on timeout — the call sites branch
   `compare_immediate(RET[0], "0"); branch_ne(&timeout)` (`:741`–`:743`,
   `:937`–`:939`, and at `:1262`'s site). `SleepConditionVariableSRW` returns a
   **BOOL: non-zero on success, 0 on timeout/failure**. The Windows arm must
   branch `branch_eq(&timeout)`. **Getting this backwards makes every timed
   receive report "ready" on timeout and then read an empty queue** — exactly the
   silent-wrong-value class `.ai/compiler.md` calls the worst one here.

`pthread_cond_wait` (the untimed form, `:183`) maps to the same
`SleepConditionVariableSRW` with `dwMs = INFINITE (0xFFFFFFFF)` and `flags = 0`.
Its call sites do not inspect the return value, so no polarity work is needed
there.

`clock_gettime` is in the Linux import set (`linux_common/plan.rs:435`) purely to
serve this deadline; on Windows it is replaced by `GetTickCount64` and
`clock_gettime` never appears in the Windows import list.

### 5.5 Frame shadow space — the contract this plan needs from 47-B

Three helpers in scope use hand-written `sp`-relative local offsets *and* make
external calls:

| helper | file:line | `FRAME_SIZE` | locals |
| --- | --- | --- | --- |
| `lower_thread_start_helper` | `runtime_helpers.rs:392`–`:400` | 160 | `sp+8 .. sp+120` |
| `lower_thread_trampoline` | `runtime_helpers.rs:737`–`:746` | 80 | `sp+0 .. sp+64` |
| `simple_thread_handle_helper` | `runtime_helpers_thread.rs:69`–`:75` | 48 | `sp+8 .. sp+40` |

Under Win64 a caller must reserve 32 bytes at `[rsp+0..31]` for the callee to home
its four register arguments. **Any local living below `sp+32` is scribbled on by
the first external call.** The trampoline's `LR_OFFSET = 0`, `ARENA_OFFSET = 8`,
`X20_OFFSET = 16`, `CLOSURE_OFFSET = 24` are all inside that window — a Windows
build without shadow handling loses its return address on the first
`AcquireSRWLockExclusive`.

47-H does **not** solve this locally by hand-adding 32 to every constant (that
would fork three frame layouts and re-open byte identity). The requirement on
47-B is: `abi::subtract_stack` / the frame finalizer must place hand-managed
locals above a platform-sized outgoing-args reservation, so a helper's
`sp+K` local addressing stays written as `sp+K` in shared code and is *realized*
at `sp+32+K` on Win64. Phase 3's first task is to **verify** 47-B provides this
and to stop with a stated blocker if it does not — per AGENTS.md, that is a
genuine external dependency, not a thing to paper over.

## 6. What must not move: the resource plane and STATE

plan-54 / bug-257 fixed a cross-thread `STATE` confusion, and bug-205 fixed a
resource-plane hang. Neither fix lives at the primitive layer, but both are
*reachable* from a careless platform switch.

- **The plane split is a queue-count invariant.** A thread carries four queues:
  data inbound/outbound (`THREAD_OFFSET_INBOUND_QUEUE = 40`,
  `THREAD_OFFSET_OUTBOUND_QUEUE = 48`) and resource inbound/outbound
  (`THREAD_OFFSET_RESOURCE_INBOUND_QUEUE = 104`,
  `THREAD_OFFSET_RESOURCE_OUTBOUND_QUEUE = 112`, `runtime_helpers.rs:19`–`:27`).
  `lower_thread_start_helper` allocates all four (`:552`–`:610`); the trampoline
  closes and broadcasts all four on exit (`:796`–`:900`); cancel and drop close
  all four (`runtime_helpers_thread.rs:302`–`:307`, the bug-205 comment).
  Because Phase 1 routes every emission through one mapping, **the queue count is
  structurally preserved** — there is no per-queue Windows code to forget. Phase 3
  must not introduce any per-queue special-casing that could break this, and the
  fixtures `thread-transfer-bidirectional-rt`, `thread-dual-cancel`, and
  `thread-drop-cleanup` are the ones that detect it.
- **STATE is a payload, not a primitive.** A stateful resource's `STATE` record is
  deep-copied into the *receiver's* arena by `copy_resource_to_current_arena`
  (`builder_arena_transfer.rs:297`, doc comment `:288`–`:296`, STATE handling at
  `:329`–`:339`) — the bug-257 fix. Windows changes nothing here; the transfer
  still rides the resource queue and the copy still runs with the arena switched
  to the destination. `thread-transfer-state-rt` is the fixture.
- **`base_resource_name` was composite-blind.** It is at
  `src/builtins/resource.rs:243` and is consumed by the plane's `RES` element
  handling (`src/builtins/thread.rs:142`–`:149`, `:757`, `:792`) and by
  `ir::verify` (`src/ir/verify/mod.rs:3842`, `:4968`–`:4971`). These are
  *frontend/IR* layers — `windows-x86_64` reaches them through exactly the same
  path every other target does, because the target only ever enters at
  `CodegenPlatform`. **This is the concrete reason the Windows path is not a
  "drop-in" claim that needs testing but a structural invariant**: no code in
  this sub-plan touches type-name handling at all. The plan-54 fixtures still get
  run on Windows because a *lowering* bug (e.g. a resource queue whose condvar is
  never woken) presents identically to a plane bug.

## Compatibility / Format Impact

- **New:** a `sync_symbol` mapping + `LogicalSync` enum in
  `src/target/shared/code/runtime_helpers.rs`; Windows arms in three emitting
  helpers; a kernel32 thread-import list in `src/target/win_x86_64/plan.rs`; the
  `thread.*` entries in the Windows backend's `runtime_calls`.
- **Unchanged:** every existing target's emitted bytes; the thread control-block
  layout (`THREAD_BLOCK_SIZE = 120`) and queue layout
  (`THREAD_QUEUE_BLOCK_SIZE = 248`); the stdin log block map; `OS_ENV_LOCK_SIZE`;
  the `CodegenPlatform` / `NativePlanPlatform` required method sets; the
  `thread::` language surface, `builtins/thread.rs`, the resolver, and IR verify;
  `_mfb_shutdown`'s behavior on every platform.
- **Spec/doc sync:** `mfb spec` topics that describe the thread runtime's
  primitives (search `src/docs/spec/**` for `pthread`) gain the Windows mapping,
  per `.ai/specifications.md`'s "keep the embedded specification current with
  every compiler change".

## Phases

### Phase 1 — One chokepoint for every sync symbol (inert refactor)

Collapse the three emission routes onto a single platform-aware mapping, with
**no new platform and no behavior change**. Safe to land alone and the phase that
retires the byte-identity risk for everything after it.

- [ ] Add `enum LogicalSync` + `fn sync_symbol(platform: &dyn CodegenPlatform,
      op: LogicalSync) -> &'static str` in
      `src/target/shared/code/runtime_helpers.rs`, beside `thread_symbol` (`:62`).
      For every currently-supported target it returns today's exact string,
      reusing `thread_symbol`'s macOS underscore rule.
- [ ] Convert `emit_thread_external_call` (`runtime_helpers.rs:70`) to take a
      `LogicalSync` instead of a `&str`, and update all 58 call sites in
      `runtime_helpers.rs` (16) and `runtime_helpers_thread.rs` (42). Keep a
      `&str` overload only if `clock_gettime`-style non-sync symbols make it
      necessary; prefer adding `LogicalSync::Clock`.
- [ ] Convert `stdin_broadcast::emit_libc` (`stdin_broadcast.rs:86`) to route its
      27 sites through `sync_symbol`.
- [ ] Convert the two inline sites in `os.rs` (`:102`, `:138`) to `sync_symbol`.
- [ ] Convert `lower_thread_start_helper`'s inline spawn-symbol computation
      (`runtime_helpers.rs:612`–`:621`) to `sync_symbol`
      (`ThreadCreate`/`AttrInit`/`AttrSetStackSize`), removing the two ad-hoc
      `platform.target() == "macos-aarch64"` compares.
- [ ] Tests: a unit test in `src/target/shared/code/tests.rs` asserting
      `sync_symbol` returns the underscored spelling for `macos-aarch64` and the
      bare spelling for each Linux target, for every `LogicalSync` variant
      (an exhaustive `match` keeps a future variant from being forgotten).

Acceptance: `cargo test` green; `scripts/test-accept.sh target/debug/mfb
target/accept-actual` green with **no golden changes**; `scripts/artifact-gate.sh`
reports a **zero-byte diff** for all four existing targets. `cargo fmt` and
`cargo clippy --all-targets` clean.
Commit: —

### Phase 2 — The rename-compatible Windows primitives

Add the Windows arm for the six primitives whose signature matches pthread's
argument shape exactly. Lands with no `thread.*` advertisement, so nothing can
build a threaded Windows program yet — it is a pure mapping addition.

- [ ] In `sync_symbol`, add the `windows-x86_64` arm for `Lock` →
      `AcquireSRWLockExclusive`, `Unlock` → `ReleaseSRWLockExclusive`,
      `MutexInit` → `InitializeSRWLock`, `CondInit` →
      `InitializeConditionVariable`, `CondSignal` → `WakeConditionVariable`,
      `CondBroadcast` → `WakeAllConditionVariable`.
- [ ] Gate the three `RET[0]`-is-zero checks after the `*_init` calls in
      `emit_thread_queue_alloc` (`runtime_helpers.rs:192`, `:209`, `:226`) on a
      platform predicate — skipped on Windows because the Win32 initializers
      return `void` (§5.1). Leave the `init_error` block intact.
- [ ] Update the three storage-reserve comments to record that 64 bytes also
      covers Windows' 8-byte `SRWLOCK`/`CONDITION_VARIABLE`
      (`runtime_helpers.rs:399`, `error_constants.rs:489`, `os.rs:43`), and
      extend `os_env_lock_init_hex`'s doc comment (`os.rs:70`–`:76`) to state
      that the all-zero non-macOS initializer is also `SRWLOCK_INIT` (no code
      change).
- [ ] Tests: extend the Phase 1 `sync_symbol` test with the `windows-x86_64`
      expectations; add a lowering test that `emit_thread_queue_alloc` on the
      Windows platform emits no post-init return check and on linux-x86_64 emits
      exactly three.

Acceptance: `cargo test` green; `scripts/artifact-gate.sh` still zero-byte for
all four existing targets; a `windows-x86_64` build of a threaded program still
fails the capability gate cleanly (`thread.*` not yet advertised), never emits a
broken `.exe`.
Commit: —

### Phase 3 — Spawn, release, and timed wait (the shape-incompatible three)

The correctness concentrator; lands behind the Phase 2 mapping and in front of
the fixtures.

- [ ] **Verify the 47-B shadow-space contract first** (§5.5): confirm that
      hand-managed `sp+K` locals in `lower_thread_trampoline`,
      `lower_thread_start_helper`, and `simple_thread_handle_helper` are realized
      above the Win64 32-byte outgoing reservation, and that
      `abi::SCRATCH[4]`/`[5]` do not alias `abi::CURRENT_THREAD` under the Win64
      register model (the hazard documented at `runtime_helpers.rs:728`–`:736`).
      If either does not hold, stop and report it as a 47-B blocker.
- [ ] Windows spawn arm in `lower_thread_start_helper`
      (`runtime_helpers.rs:612`–`:691`): skip the attr pair; stage
      `CreateThread(NULL, 8 MiB, trampoline, cb, 0, NULL)` — args 5/6 via the
      outgoing stack tail (`abi::outgoing_stack_arg_store`, `abi.rs:56`); reload
      `%v9` from `CB_OFFSET` and `store_u64(RET[0], %v9, THREAD_OFFSET_OS_HANDLE)`;
      invert the failure branch to `branch_eq(&spawn_error)` (§5.2).
- [ ] Map `ThreadRelease` to `CloseHandle` in `sync_symbol`, and confirm both
      `pthread_detach` sites (`runtime_helpers_thread.rs:242` in `WaitFor`, `:663`
      in `Drop`) load the handle **by value** from `THREAD_OFFSET_OS_HANDLE` —
      which is already the correct shape for `CloseHandle(HANDLE)` (§5.3). No
      `WaitForSingleObject`, no `TerminateThread`.
- [ ] Windows arm in `emit_thread_deadline` (`runtime_helpers_thread.rs:11`):
      `GetTickCount64()` + `timeoutMs` → an 8-byte millisecond deadline in the
      existing scratch slot, replacing the `clock_gettime` + sec/nsec
      normalization (§5.4). Add `LogicalSync::Clock` → `GetTickCount64`.
- [ ] Windows arm at the three `CondTimedWait` sites
      (`runtime_helpers_thread.rs:740`, `:936`, `:1262`): recompute
      `dwMs = deadline - GetTickCount64()` clamped at 0, call
      `SleepConditionVariableSRW(cv, lock, dwMs, 0)`, and **invert the polarity**
      to `branch_eq(&timeout)`. Map `CondWait` to the same call with
      `dwMs = 0xFFFFFFFF`.
- [ ] Tests: lowering unit tests asserting, for the Windows platform, (a) the
      spawn sequence contains `CreateThread` with two outgoing stack args and a
      store of `RET[0]` to `THREAD_OFFSET_OS_HANDLE`, (b) the timed-wait sites
      branch on **equal**-to-zero where the Linux sites branch on not-equal, and
      (c) no emitted symbol anywhere in the thread path is `TerminateThread` or
      `pthread_*`. Assert the linux-x86_64 streams for the same helpers are
      unchanged.

Acceptance: the Windows lowering unit tests pass and the polarity assertion in
(b) is present; `scripts/artifact-gate.sh` still zero-byte for all four existing
targets; `cargo clippy --all-targets` clean.
Commit: —

### Phase 4 — Advertise `thread.*` and prove it at runtime (highest-risk work last)

- [ ] `src/target/win_x86_64/mod.rs`: add the 12 `thread.*` entries to
      `runtime_calls`, matching the block at
      `src/target/linux_common/mod.rs:186`–`:197`.
- [ ] `src/target/win_x86_64/plan.rs`: map the full `thread.*` call set — the
      15 arms at `src/target/linux_common/plan.rs:407`–`:421`, including the four
      resource-plane calls the bug-176 C comment (`:397`–`:401`) says must not be
      omitted — to the kernel32 import list: `CreateThread`, `CloseHandle`,
      `InitializeSRWLock`, `AcquireSRWLockExclusive`, `ReleaseSRWLockExclusive`,
      `InitializeConditionVariable`, `SleepConditionVariableSRW`,
      `WakeConditionVariable`, `WakeAllConditionVariable`, `GetTickCount64`.
      Handle `thread.openStdIn`/`closeStdIn` with the stdin-broadcast subset, as
      `stdin_broadcast_imports` does at `:402`–`:405`.
- [ ] Spec sync: add the Windows primitive mapping to the `mfb spec` topics that
      describe the thread runtime (grep `src/docs/spec/**` for `pthread`), per
      `.ai/specifications.md`.
- [ ] Tests: run every directory under `tests/rt-behavior/threads/**` (33 today)
      built for `windows-x86_64` on Windows/Wine, comparing stdout/stderr/exit
      code against the linux-x86_64 build. Include the stdin-broadcast pair
      (`func_thread_openStdIn_valid`, `func_thread_closeStdIn_valid`) and the
      negative pair under `tests/syntax/threads/**`.

Acceptance: all 33 `tests/rt-behavior/threads/**` fixtures produce output
**byte-identical** to the linux-x86_64 build on Windows/Wine — specifically
including `thread-transfer-state-rt`, `thread-transfer-bidirectional-rt`,
`thread-dual-cancel`, `thread-drop-cleanup`, and `thread-queue-timeout-cancel`
(the timed-wait polarity detector). `scripts/test-accept.sh` and `cargo test`
green; `scripts/artifact-gate.sh` zero-byte for the four existing targets.
Commit: —

## Validation Plan

- **Tests.**
  - Unit: `sync_symbol` exhaustive per-target mapping;
    `emit_thread_queue_alloc`'s init-check gating; the Windows spawn sequence;
    the timed-wait polarity inversion; a "no `pthread_*` and no
    `TerminateThread` symbol is emitted for `windows-x86_64`" sweep over the
    thread helper streams.
  - Negative: a program calling `thread::start` built for `windows-x86_64`
    *before* Phase 4 must fail the capability gate with the target's "not yet
    supported" build error, never emit an `.exe` (the master's per-surface
    gating rule).
  - Fixtures: `tests/rt-behavior/threads/**` (33 dirs) and
    `tests/syntax/threads/**`.
- **Runtime proof.** Build each `tests/rt-behavior/threads/**` program for both
  `linux-x86_64` and `windows-x86_64`; run the Linux build locally and the `.exe`
  on a Windows runner or Wine; `diff` stdout, stderr, and exit code. The proof
  program for the plane invariant is `thread-transfer-state-rt` (a `STATE`-carrying
  resource crossing the plane, per plan-54); the proof program for the timed-wait
  conversion is `thread-queue-timeout-cancel`.
- **Regression guard.** `scripts/artifact-gate.sh` after **every** commit, not
  just at the end — this sub-plan edits code compiled into all five backends.
  Phase 1's acceptance is specifically a zero-byte diff.
- **Doc sync.** `mfb spec` thread-runtime topics under `src/docs/spec/**` (per
  `.ai/specifications.md`); update `.ai/compiler.md`'s target notes only if the
  Windows thread path adds a new register-lifetime hazard worth recording.
- **Acceptance.** `scripts/test-accept.sh target/debug/mfb target/accept-actual`
  (per `.ai/compiler.md` §Validation), `cargo test`, `cargo fmt`,
  `cargo clippy --all-targets`. Per AGENTS.md: no test or golden is edited to
  make this change pass without a written proof that the assertion was wrong.

## Open Decisions

- **Shrink the 64-byte sync reserves for Windows, or keep them?** Recommend
  **keep** (§4): `SRWLOCK`/`CONDITION_VARIABLE` are 8 bytes each and fit inside
  the existing reserve, so every offset stays platform-invariant and the four
  existing targets are byte-identical by construction. Shrinking saves ~672 bytes
  per spawned thread against an 8 MiB stack and would fork the queue layout. If a
  future platform ever needs *more* than 64, add
  `CodegenPlatform::sync_primitive_reserve()` with a `64` default — not now. (§4)
- **`SRWLOCK` vs `CRITICAL_SECTION`.** Recommend **`SRWLOCK`**: 8 bytes,
  statically zero-initializable, non-recursive (matching default
  `pthread_mutex_t`), and the only lock `SleepConditionVariableSRW` accepts.
  `CRITICAL_SECTION` is 40 bytes, recursive, and needs a paired
  `DeleteCriticalSection` the current code has no place to call. The master left
  this open; this plan closes it. (§3)
- **`WaitForSingleObject` in `thread::waitFor`?** Recommend **no**. The repo's
  `waitFor` does not join — it waits on the outbound queue's condvar for
  `THREAD_STATE_COMPLETED` and then releases the handle
  (`runtime_helpers_thread.rs:144`–`:243`); there is no `pthread_join` in the
  tree. Introducing `WaitForSingleObject` would give Windows a *different*
  completion protocol from every other target, for no behavioral gain, and would
  race the condvar path. It stays out of the import list. (§5.3, §7 below)
- **`GetTickCount64` vs `QueryPerformanceCounter` for the deadline.** Recommend
  **`GetTickCount64`**: millisecond-resolution, monotonic, single kernel32 call,
  no frequency query, and the unit already matches `timeoutMs` and
  `SleepConditionVariableSRW`'s `dwMs`. `QueryPerformanceCounter` buys resolution
  the `thread::` timeout API cannot express. (§5.4)
- **Where the shadow-space reservation lands for hand-managed frames.** Recommend
  47-B own it (a shadow-aware locals base in the frame finalizer) so shared
  lowering keeps writing `sp+K`. The alternative — 47-H adding 32 to every
  hardcoded offset behind a platform `const` — forks three frame layouts and
  re-opens byte identity. If 47-B did not deliver it, Phase 3 stops and reports a
  blocker. (§5.5)

## 7. `TerminateThread` and the shutdown path

Recorded explicitly because the master's Phase F sketch lists `TerminateThread`.
**It is not used anywhere in this plan.** `TerminateThread` stops a thread at an
arbitrary instruction: it does not release the thread's stack, does not run any
unwind, and — decisively here — can stop a worker **while it holds a queue
`SRWLOCK`**, permanently deadlocking every other thread on that queue, including
the main thread's `_mfb_shutdown`. It would also abandon an arena mid-mutation,
which is the failure class bug-257 and the arena free-list notes are about.

What the three teardown paths actually do:

- **`thread::cancel`** stays *cooperative*: it sets the cancelled flag and
  closes + broadcasts all four queues (`runtime_helpers_thread.rs:308`, with the
  bug-205 comment at `:302`–`:307`) so a parked worker re-checks and returns. The
  worker chooses when to stop. Windows changes only the primitive that performs
  the broadcast (`WakeAllConditionVariable`).
- **`thread::waitFor` / `thread::drop`** call `pthread_detach` → `CloseHandle`.
  On Windows `CloseHandle` releases the process's reference to the thread object;
  the thread keeps running and the kernel reclaims the object when it exits —
  semantically the same "I will not join this" statement `detach` makes.
- **`_mfb_shutdown`** does **nothing thread-related** on any platform: it drains
  the stdout buffer, turns the terminal off, and destroys the main arena
  (`entry_and_arena.rs:1907`–`:1918`). Remaining worker threads are terminated by
  process exit — `ExitProcess` on Windows (47-D's `emit_program_exit`), which
  terminates every thread in the process, exactly as `exit`/`_exit` does on
  Linux/macOS. That equivalence is why the shutdown path needs no Windows arm at
  all, and it is the correct place for "kill the threads" to happen: at process
  teardown, after the arena is already released, not at an arbitrary instruction
  while a lock is held.

## Summary

The real risk in this sub-plan is not Windows — it is that the work happens
**inside shared lowering that all five backends compile through**, so the
byte-identity guard is a per-commit gate rather than a final check. Phase 1
retires most of that risk by collapsing three ad-hoc emission routes (58 calls
through `emit_thread_external_call`, 27 through `stdin_broadcast::emit_libc`, 2
inline in `os.rs`) into one platform-aware mapping with provably identical
output, after which the Windows work is additive.

The remaining engineering risk is concentrated in three signature mismatches, of
which the timed wait is the dangerous one: relative-vs-absolute, ms-vs-`timespec`,
and an **inverted success polarity** that would silently turn every timeout into
a spurious "ready". Spawn (handle returned rather than written through a pointer,
inverted failure test, two stack args) and release (`CloseHandle` for
`pthread_detach`) are mechanical by comparison. The trampoline needs **no
`DWORD` shim** — once 47-B realizes `ARG[0]`/`RET[0]` as `rcx`/`rax`, the existing
body already *is* an `LPTHREAD_START_ROUTINE` — but it does depend on 47-B
supplying Win64 shadow space for hand-managed frames, which Phase 3 verifies
before writing any code.

Left untouched: the thread control-block and queue layouts, the two
direction-isolated resource queues and the plan-54 `STATE` deep-copy that closed
bug-257, `base_resource_name` and every frontend/IR layer above the
`CodegenPlatform` seam, `_mfb_shutdown`, and the emitted bytes of all four
shipping targets.


## Corrections

<!-- Filled in during execution. -->

- 2026-07-20 — **The header's single `Depends on: plan-47-D` contradicted this
  document's own body, three times** (§Phase 3's first task, §Phase 3's opening, and
  §Open Decisions all say the blocker is 47-B's shadow space). Dependencies are now
  declared per phase: H1 nothing, H2 nothing, H3 47-B, H4 47-D. **H1+H2 blocking on
  nothing is the single biggest de-risking move available in this feature** — they are
  inert shared refactors that can land before 47-B exists.
- 2026-07-20 — **"different in kind from 47-F/E/G" is false.** There is no
  `emit_socket`/`emit_connect` on `CodegenPlatform` at all, so 47-I rewrites 37 hardcoded
  POSIX socket literals in `shared/code/net/` and 47-G rewrites 6 in `io_helpers.rs`/
  `term.rs`. G is this sub-plan's shape at 38% the scale; E at 7%. Only 47-C1 touches no
  shared code. **Phase 1's chokepoint technique is the reusable pattern for the whole
  feature** and has been cloned as G1 and E1.
- 2026-07-20 — **Effort `large` is over the sub-plan band.** The split rule says large
  plans split into small/medium sub-plans before starting; this document's own header
  already conceded "the four phases below are individually medium and land separately".
  The per-phase dependency table above *is* that split.
- 2026-07-20 — Counts re-measured: `runtime_helpers.rs` holds **21** pthread literals,
  not 17; `stdin_broadcast.rs` has 27 literals across **32** `emit_libc` call sites;
  `emit_thread_external_call` has **15** call sites in `runtime_helpers.rs` (the 16th
  match is the definition at `:70`) and 42 in `runtime_helpers_thread.rs`, so the
  "58 call sites" figure is **57**. The aggregate "85 call sites" is **91** routed sites
  (86 pthread-bearing). Re-derive before using any of these as a completion checklist.
- 2026-07-20 — **"compiled for all five targets" is four today.** A fifth exists only
  after 47-B registers Windows.
- 2026-07-20 — **`thread_symbol` is not "the only platform switch in the thread path".**
  There are three: `:62` plus inline `== "macos-aarch64"` tests at `:612` and `:617`.
  (§Phase 1 later names those two correctly — the summary contradicted itself.)
- 2026-07-20 — `lower_shutdown` is `entry_and_arena.rs:1868`, not `:1880`.
  `builtins/thread.rs:757` and `:792` are **not** `base_resource_name` consumers — they
  are plan-54 unit tests; the invariant argument cites the wrong lines.
