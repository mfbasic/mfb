# MFBASIC Broadcast Stdin Plan

Last updated: 2026-07-05 (reference refresh — design unchanged; still unimplemented)

> **2026-07-05 reference refresh.** Code moved since this was written: the stdin
> read/poll helpers now live in `src/target/shared/code/io_helpers.rs` (not the old
> `mod.rs:NNNN` lines — `mod.rs` was split and is now ~2.9k lines); the thread/queue
> runtime helpers are in `src/target/shared/code/runtime_helpers_thread.rs` and
> `runtime_helpers.rs`; the arena-state constants and `module_uses_call` gate are in
> `error_constants.rs` and `module_analysis.rs`; spec/man trees moved to
> `src/docs/spec/**` and `src/docs/man/**`. **`ARENA_STATE_SIZE` is no longer 104** —
> the allocator-01 quick bins + designated-victim carve were appended after this plan,
> so it is now `ARENA_CARVE_SIZE_OFFSET + 8` (~1144 B). Ignore the "104 → 136" numbers
> below: append the 4 stdin words *after* the carve and let `ARENA_STATE_SIZE` /
> the derived `ENTRY_STACK_SIZE`/`ENTRY_ARGC/ARGV` grow from there. Coordinate the
> bump with **plan-14** if landing together.

This plan replaces the per-byte stdin read path with a buffered, **broadcast**
reader: the runtime owns `fd 0`, reads it in chunks into one process-global
append-only log, and every *subscribed* thread reads its own independent cursor
over that log — so each subscriber sees the entire stdin byte stream from the
point it subscribed, and bytes are never consumed out from under another thread.
A correct implementation (1) collapses `io::readLine`/`input`/`readChar`/
`readByte` from one `read()` syscall **per byte** to one per ~chunk, and
(2) makes `stdin` a per-thread broadcast under an explicit
`thread::openStdIn`/`closeStdIn` subscription, with the single-threaded program
remaining **byte-for-byte unchanged**.

This supersedes **plan-02-perf-hotpaths.md Phase 5** (the transparent per-arena
buffered reader): the broadcast model is the strictly-larger feature, and building
the transparent version first would be thrown-away work. It is the read-side
sibling of **plan-14-io-buffering.md** (opt-in *output* buffering); the two share
no code but should land with a consistent `io::`/`thread::` surface.

It complements:

- `./mfb spec io` (the `readLine`/`input`/`readChar`/`readByte`/`pollInput`
  line/byte contract — broadcast must stay invisible to a single-consumer program;
  canonical source `src/docs/spec/io/**`).
- `./mfb spec threading` (worker/queue isolation model, the per-thread arena, the
  thread control block, OS integration; canonical source `src/docs/spec/threading/**`,
  esp. `08_queue-semantics`, `09_os-integration`, `06_thread-runtime-helpers`).
- `./mfb spec memory` (the per-arena `ARENA_STATE` block this grows; canonical
  source `src/docs/spec/memory/**`).

## 1. Goal

- `io::readLine`/`input`/`readChar`/`readByte` on non-seekable stdin issue **one
  `read(0,…)` per ~4–8 KiB**, not one per byte. A piped 100k-line run shows the
  syscall count collapse. (Note: this is the **stdin** path only. The suite's
  current `io read` benchmark reads a **file** via `fs::readLine` — a different,
  O(N²) path fixed by **plan-14-C**, not this plan. An earlier revision of the
  benchmark piped stdin, which is where the stale "io-read 287 ms" figure came from.)
- A **single-threaded** program's observable stdin behavior is **byte-identical**
  to today (same bytes, same EOF, same `pollInput`), with no source change.
- Each thread that has called `thread::openStdIn` sees the **full** stdin stream
  from its subscription point; a byte read by one subscriber is **not** consumed
  from another. EOF broadcasts to every subscriber at the same stream offset.
- `thread::closeStdIn` unsubscribes a thread and releases its hold on the log;
  thread teardown auto-unsubscribes.
- Reading stdin from a thread that is **not** subscribed is a defined error
  (`ErrInvalidContext`), except for the compiler-inserted main-thread subscription
  that preserves single-threaded compatibility.

### Non-goals (explicit constraints)

These are guardrails. A phase that violates one is wrong:

- **No change to single-consumer observable behavior.** With exactly one
  subscriber (the common case, incl. every existing single-threaded program), the
  bytes delivered, the EOF position, and `pollInput` results are identical to the
  current per-byte reader. Buffering and the log are invisible.
- **No output-side change.** `io::write`/`print` stays unbuffered and untouched —
  output buffering is plan-14's scope (it changes flush/crash/ordering semantics).
- **No change to file reads.** `fs::` reads are seekable and already block-read via
  `lseek`; they do not touch the stdin log.
- **Thread-isolation model preserved.** The broadcast log is the **only** new
  cross-thread shared mutable state; it is guarded by its own mutex/condvar (the
  same `pthread_mutex_*`/`pthread_cond_*` primitives the transfer queues already
  use) and never leaks pointers across the arena boundary. No arena's heap is
  reachable from another thread through this feature.
- **Bounded memory.** The log is bounded by a configurable high-water mark; a
  stalled subscriber blocks producers (it does not grow memory without limit and
  does not silently drop bytes).
- **`ARENA_STATE` growth stays internal.** Growing the per-arena state block is a
  runtime-internal layout change invisible to programs and to the collection/value
  ABI; it must be re-verified across threads, app-mode, and signal/shutdown paths.

## 2. Current State

### 2.1 Per-byte stdin reads (the syscall storm)

`lower_io_read_line_helper` (`src/target/shared/code/io_helpers.rs:895`) sets `x2 = 1`
and calls `platform.emit_read_file` in its `read_loop` and again for each UTF-8
continuation byte — roughly fifteen single-byte `emit_read_file` sites across
`lower_io_read_byte_helper` (`io_helpers.rs:434`), `lower_io_read_char_helper`
(`io_helpers.rs:565`), `lower_io_read_line_helper`, and `input` (all in
`io_helpers.rs`). One `read()` per byte; 100k lines ≈ 600k+ syscalls. The
line-buffer growth itself is already geometric/amortized, so the syscall count — not
buffer realloc — is the cost. The reason it cannot naively block-read: stdin is
non-seekable, so over-reading past a `\n` would strand the next line's bytes (there
is no per-process read buffer). Contrast `fs::readLine`, which `lseek`s and
block-reads a seekable fd. `isatty` is already linked (`io_helpers.rs:277`).

### 2.2 Per-arena state (where buffer state will live)

Each OS thread owns its own arena, pinned in `x19`
(`ARENA_STATE_REGISTER`, `error_constants.rs:123`); `ARENA_STATE_SIZE`
(`error_constants.rs:186`, now `ARENA_CARVE_SIZE_OFFSET + 8` after the allocator-01
quick bins/carve) already holds per-thread RNG state at offsets 88/96 — the proven
precedent for per-thread runtime state reachable without a thread-local lookup.
`ENTRY_STACK_SIZE`/`ENTRY_ARGC/ARGV` are **derived** from `ARENA_STATE_SIZE`
(`error_constants.rs`), so growing it shifts them automatically — but the change is
layout-sensitive and has historically caused SIGBUS-class bugs (see the macOS rebase
/ writable-data-segment notes), so it must be verified on threads, app-mode, and the
signal/shutdown paths.

### 2.3 Existing thread runtime (the primitives to reuse)

Workers are real `pthread`s (`pthread_create`, `runtime_helpers.rs:478`). The
transfer/accept **queues already block** on `pthread_mutex_lock`/
`pthread_cond_broadcast`/`pthread_mutex_unlock` (`thread_queue_write_helper`
`runtime_helpers_thread.rs:724`, `thread_queue_read_helper`
`runtime_helpers_thread.rs:974`), with the lock/condvar living in the thread control
block. The broadcast log reuses exactly this
mutex+condvar shape, and "blocking when full / waking on progress" is the same
contract the queues already present (`mfb spec threading queue-semantics`).

### 2.4 Thread package surface

`thread::` builtins are defined in `src/builtins/thread.rs` (`TRANSFER`,
`ACCEPT`, the resource-plane internal targets, `is_thread_call`,
`is_thread_type`). `openStdIn`/`closeStdIn` are added here and lowered to new
runtime helpers, mirroring how `transfer`/`accept` plumb through resolve →
typecheck → IR → native lowering.

## 3. Design Overview

Three layers:

1. **One process-global broadcast log** (runtime, behind one mutex+condvar): an
   append-only sequence of fixed blocks with absolute stream offsets, plus a small
   **subscriber registry** (one cursor per subscribed thread). This is the only new
   shared state.
2. **Per-arena staging** (reachable via `x19`): a 4 KiB local copy buffer + a
   pointer to this thread's registry entry. Parsing (the readLine UTF-8 work) runs
   lock-free against the local copy; the lock is held only for the memcpy-out and
   the occasional OS read.
3. **A cooperative reader** (no dedicated thread): whichever subscriber needs bytes
   beyond the frontier becomes the reader for one `read(0,…)`, guarded by a
   `readerBusy` flag + the condvar so exactly one thread touches `fd 0` at a time
   and the blocking syscall is **never** held under the data lock.

Correctness risk concentrates in three places: (a) the lock discipline around the
blocking `read(0)` and the condvar wake-set (deadlock / lost-wakeup); (b)
reclamation-at-min-cursor vs. the backpressure cap (memory bound and the
"stalled subscriber blocks producers" contract); (c) the `ARENA_STATE` growth
re-verification. Each is contained by reusing the queue's proven mutex/condvar
pattern and by landing the single-consumer path (byte-identical, no broadcast)
before any multi-subscriber behavior.

## 4. Detailed Design

### 4.1 The broadcast log (runtime-global)

A single global structure, zero-initialized, lazily set up on first stdin use:

```
StdinLog {
  pthread_mutex_t mutex
  pthread_cond_t  cv
  Block*  head, *tail          // deque of fixed BLOCK_SIZE (8 KiB) blocks
  u64     base                 // absolute stream offset of head block's first live byte
  u64     fill                 // absolute offset one past the last byte read from the OS
  u64     eofOffset            // absolute offset where read()==0 occurred; U64_MAX until then
  bool    readerBusy           // a subscriber is currently in poll/read(0)
  bool    shuttingDown         // set by _mfb_shutdown / signal path
  int     selfPipe[2]          // D4: _mfb_shutdown writes [1]; reader polls [0] beside fd 0
  u32     subscriberCount
  Subscriber registry[...]     // {arenaId, cursor, active}; cursor = next unread abs offset
}
```

- **Blocks, not a ring.** Appending grows by linking a new block; reclamation frees
  whole blocks whose end ≤ `base`. No realloc, no compaction memcpy. `base ==
  min(cursor)` over active subscribers.
- **Backpressure cap** `STDIN_LOG_CAP` (default 4 MiB, configured at build time via
  the `project.json` `"config"` section — D3): the reader refuses to advance `fill`
  past `base + CAP` and **blocks on the condvar** until a slow subscriber advances
  `base` or unsubscribes. The cap is a fixed high-water mark, **not** a function of
  current lag. The build reads the value and bakes it into the binary as a constant;
  there is no env var or runtime setter.
- Hash/aliasing free: blocks hold raw bytes addressed by absolute offset; nothing
  in the log points into any arena.

### 4.2 Per-arena staging (via `x19`)

Add to `ARENA_STATE` (4 new `u64` words, appended after `ARENA_CARVE_SIZE_OFFSET`;
`ARENA_STATE_SIZE` grows by 32 bytes — see the refresh banner, it is no longer 104):

- `STDIN_LOCAL_BUF` — pointer to a lazily-arena-allocated 4 KiB copy buffer (NULL ⇒
  not yet allocated).
- `STDIN_LOCAL_FILLED`, `STDIN_LOCAL_POS` — valid byte count / read position in it.
- `STDIN_SUBSCRIBER` — pointer to this thread's registry entry (NULL ⇒ not
  subscribed).

All four are zeroed where `ARENA_STATE` is already zeroed, so NULL/zero is the
correct "not set up" default. The derived `ENTRY_STACK_SIZE`/`ENTRY_ARGC/ARGV` shift
with `ARENA_STATE_SIZE` automatically; the worker-arena allocation size and the
entry-frame reservation must move with it (§Layout/ABI).

### 4.3 `_mfb_rt_stdin_next_byte` (cooperative reader)

Returns the next byte for the calling thread in `x0`, or an EOF sentinel, or
traps `ErrInvalidContext` if the thread is not subscribed (and is not the implicit
main subscriber). Fast path takes no lock:

```
if LOCAL_POS < LOCAL_FILLED: return LOCAL_BUF[LOCAL_POS++]          // lock-free

lock(mutex)
sub = STDIN_SUBSCRIBER
if sub == null: { unlock; trap ErrInvalidContext }                 // see §4.5 for main
loop:
  if shuttingDown: { unlock; return EOF }
  if sub.cursor < fill:                                            // bytes available to me
     n = min(fill - sub.cursor, 4096)
     copy n bytes at absolute sub.cursor from the block deque into LOCAL_BUF
     sub.cursor += n; LOCAL_FILLED = n; LOCAL_POS = 0
     base = min(cursor over active subs); free blocks with end <= base
     cond_broadcast(cv)            // a producer blocked on the cap may now proceed
     unlock; return LOCAL_BUF[LOCAL_POS++]
  elif sub.cursor >= eofOffset: { unlock; return EOF }
  elif readerBusy: cond_wait(cv, mutex)                            // someone else is reading
  elif fill >= base + STDIN_LOG_CAP: cond_wait(cv, mutex)          // backpressure: wait for base to advance
  else:
     readerBusy = true; unlock(mutex)
     poll(fd0 + selfpipe)          // BLOCKING, no lock held — the load-bearing invariant (D4)
     if selfpipe readable || shuttingDown: { lock; readerBusy=false; cond_broadcast; unlock; return EOF }
     n = read(0, tmp, CHUNK)       // fd0 reported readable ⇒ this does not block long
     lock(mutex)
     if n < 0 && errno == EINTR && shuttingDown: { readerBusy=false; cond_broadcast; unlock; return EOF }
     if n < 0 && errno == EINTR: { readerBusy=false; cond_broadcast; continue }   // retry
     if n == 0: eofOffset = fill
     else: append tmp[0..n] to the block deque; fill += n
     readerBusy = false; cond_broadcast(cv)
     // re-loop: now cursor < fill, or EOF, or still capped
```

Invariants that make it correct:

- **One reader at a time** (`readerBusy` + cv): no two threads race the OS cursor.
- **`read(0)` never holds the data lock**: a thread parked on the OS does not freeze
  the others; they either find bytes already appended or wait on the cv.
- **Copying advances `cursor`**, which lets `base` advance and blocks free — a
  thread that keeps reading never pins memory.
- **Broadcast is intrinsic**: each subscriber has its own `cursor` over one
  append-only log; EOF is just an absolute offset every subscriber reaches.

The ~15 single-byte read sites in §2.1 (`emit_read_file` in `io_helpers.rs`) are rerouted to call this helper instead of
`emit_read_file`; the readLine/readChar UTF-8 assembly and line-buffer growth are
otherwise unchanged (they now consume from `LOCAL_BUF`).

### 4.4 `pollInput`

`lower_io_poll_input_helper` (`io_helpers.rs:164`) reports, for the calling thread:
`LOCAL_POS < LOCAL_FILLED` (lock-free), else under lock `cursor < fill ||
cursor >= eofOffset`. It never blocks and never reads the OS.

### 4.5 Subscription surface and the compat shim

- `thread::openStdIn(worker)` and `thread::closeStdIn(worker)` — new `thread::`
  builtins (`src/builtins/thread.rs`) returning `Nothing`, taking a worker-thread
  handle (the no-arg "subscribe the calling thread" form lowers to the same helper
  with the current arena). They lower to `_mfb_rt_stdin_subscribe` /
  `_mfb_rt_stdin_unsubscribe`, which lock the registry, add/remove the entry, set
  `cursor = fill` (the **current frontier**) on subscribe, recompute `base` and
  `cond_broadcast` on unsubscribe, and set/clear the target arena's
  `STDIN_SUBSCRIBER`.
- **One join point only.** A subscriber always joins at the current frontier and
  sees every byte that arrives after. "From the absolute start" for a late joiner
  is deliberately not offered — it would require never reclaiming (unbounded) and is
  nondeterministic relative to other subscribers' progress. The "see everything"
  property is simply what you get by subscribing before any consumption — which is
  what the main shim does.
- **Compat shim.** The compiler inserts an implicit `openStdIn(self)` at the **top
  of `main`** for any program whose module uses a stdin builtin (reuse the
  `module_uses_call` gate already used for `io.pollInput` — `module_analysis.rs:74`,
  invoked at `data_objects.rs:128`). At
  program entry `fill == 0`, so main's cursor is 0 and it sees the whole stream —
  byte-identical to today. This makes explicit subscription the semantic model
  while keeping every existing program working untouched.
- **Teardown auto-unsubscribes**: worker teardown (and `_mfb_shutdown`) removes the
  arena's registry entry so an exited/crashed thread never permanently pins `base`.

### 4.6 Shutdown / signals

- **Signal path (SIGINT/SIGTERM).** The handler must not install `SA_RESTART`, so a
  subscriber blocked in `read(0)` returns `EINTR`; the reader checks `shuttingDown`
  and returns EOF. `_mfb_shutdown` sets `shuttingDown` and `cond_broadcast`s to
  release cv-waiters. (`mfb spec` shutdown note: console SIGINT/SIGTERM →
  `_mfb_shutdown` then `_exit(128+signo)`.)
- **Orderly shutdown with a blocked reader** (main returns / another thread exits
  while a subscriber is parked in `read(0)`): no signal fires, so `EINTR` will not.
  The reader therefore never blocks in a bare `read(0)`; it blocks in
  `poll(fd 0 + self-pipe)` and only `read(0)`s when `poll` reports `fd 0` readable
  (D4). `_mfb_shutdown` sets `shuttingDown`, writes one byte to the self-pipe, and
  `cond_broadcast`s; the parked reader wakes from `poll`, sees `shuttingDown`, and
  returns EOF — deterministically, independent of whether the thread model joins
  workers. The unsubscribe-on-teardown plus the `shuttingDown` broadcast still
  handles cv-waiters. Note: a `cond_broadcast` alone does **not** reach a thread
  blocked in a syscall — the self-pipe is what escapes the "interrupt a blocked
  read" requirement.

## Layout / ABI Impact

- **`ARENA_STATE_SIZE` grows by 32 bytes** (4 new `u64` words appended after the
  allocator carve; no longer "104 → 136" — see the refresh banner; §4.2). Internal runtime
  state, invisible to programs and to the collection/value ABI. `ENTRY_ARGC/ARGV`
  shift automatically (derived). The worker-arena allocation size and the
  program-entry frame reservation must move with it. Update the `mfb spec memory`
  arena-state description if it documents the block.
- **Collection/record/string/map layouts: unchanged.** Golden output, copy, and
  thread-transfer of values are unaffected.
- **New thread-package surface** (`thread::openStdIn`/`closeStdIn`): document in
  `mfb spec threading` and add man pages (`.ai/man_template.md`). Define
  the not-subscribed read error (`ErrInvalidContext`) in `mfb spec diagnostics` (D1).
- **New `project.json` `"config"` section** (D3): a build-time config block holding
  `STDIN_LOG_CAP` (and a home for future runtime tunables); the build reads it and
  bakes the cap into the binary as a constant. Document the section and its keys in
  `mfb spec tooling project-manifest` (canonical `src/docs/spec/tooling/01_project-manifest.md`)
  — a new section alongside `targets`/`build`/`entry` (§9 "Targets And Build
  Metadata"), with `config` added to the §11 validation rules (unknown/invalid keys)
  and to §10's unknown-field handling.
- **`io` contract:** `mfb spec io` gains the broadcast/subscription semantics and
  the single-consumer compatibility guarantee.

## Phases

Land the byte-identical single-consumer path before any broadcast behavior. Each phase lists its
concrete tasks and the acceptance criterion verified before it is done; fill in `Commit:` with the
hash(es) that land it.

### Phase 1 — Broadcast-log runtime primitive + arena growth, single implicit subscriber

The whole reader machinery, exercised by one subscriber so the observable behavior is unchanged.

- [ ] Build the process-global `StdinLog` (block deque + mutex/condvar + subscriber registry; §4.1).
- [ ] Implement `_mfb_rt_stdin_next_byte` — the full cooperative reader incl. the `readerBusy` one-reader rule, `read(0)`-never-under-lock, reclaim-at-min, EOF, EINTR/`shuttingDown` handling, and the backpressure cap (inert with one subscriber) (§4.3).
- [ ] Grow `ARENA_STATE` (+4 words appended after the allocator carve: `STDIN_LOCAL_BUF`/`_FILLED`/`_POS`/`STDIN_SUBSCRIBER`, §4.2) and re-verify the layout on threads / app-mode / signal-shutdown paths (coordinate with plan-14's stdout words if landing together).
- [ ] Reroute the ~15 single-byte read sites (`readLine`/`readChar`/`readByte`/`input`, all in `io_helpers.rs`) and `pollInput` (§4.4) to consume from the log; insert the compiler main-thread compat `openStdIn(self)` (§4.5).

Acceptance: every existing stdin test/golden is byte-identical; a piped 100k-line stdin run proves the `read(0,…)` syscall count collapses (~1 per 4–8 KiB); SIGINT during a blocking `readLine` still exits 130; app-mode and threads still start/stop cleanly.
Commit: a49d4bce (with Phase 2). **DONE + verified** — full acceptance suite (935 tests) byte-identical (`.run` goldens unchanged; only the +32 arena-immediate churned mir/ncode/nplan artifact goldens, regenerated); local: readLine/readByte/readChar/pollInput incl. empty / no-trailing-newline / 100k-line / 20k-byte-line (multi-block) / unicode; SIGINT-during-readLine exits 130; app-mode-io/plumbing/term build+run.

### Phase 2 — Explicit subscription + multi-subscriber broadcast

Turn the registry into a real per-thread broadcast.

- [ ] Add `thread::openStdIn`/`closeStdIn` through resolve → typecheck → IR → native lowering (`src/builtins/thread.rs` → `_mfb_rt_stdin_subscribe`/`_unsubscribe`), incl. the no-arg self form and the worker-handle form (§4.5).
- [ ] Implement the not-subscribed read error (`ErrInvalidContext`, D1) and allocate its code in `mfb spec diagnostics`.
- [ ] Implement backpressure **blocking** across N subscribers (cv-wait until `base` advances / a `closeStdIn` / `shuttingDown`) and late-join-at-frontier (subscribe sets `cursor = fill`).
- [ ] Tests: `tests/func_thread_openStdIn_*`, `tests/func_thread_closeStdIn_*` (`_valid/**` + `_invalid/**`).

Acceptance: two subscribed workers see the full stream independently (a byte read by one is still seen by the other); late join sees from its subscription point, not the start; `closeStdIn` releases the log (a stalled-then-closed subscriber unblocks a capped producer); EOF reaches every subscriber; reading unsubscribed traps `ErrInvalidContext`.
Commit: a49d4bce (+ fixtures f03211d7, 7e4956e9). **DONE + verified** — two workers each see the full stream at 5 / 1000 / 50000 lines (broadcast, not split); backpressure holds at 300k lines (>4 MiB) both-read-all; late-join joins at frontier (subscribe sets `cursor = fill`); `ErrInvalidContext` (77050019) on unsubscribed worker read; `func_thread_openStdIn_valid`/`_invalid` + `func_thread_closeStdIn_valid`/`_invalid` fixtures.

### Phase 3 — Shutdown/teardown hardening + migration

Make shutdown deterministic with a parked reader, and start the migration.

- [ ] Implement the D4 self-pipe: reader `poll(fd0 + selfpipe)` instead of a bare `read(0)`; `_mfb_shutdown` sets `shuttingDown`, writes the pipe, and `cond_broadcast`s (§4.6).
- [ ] Auto-unsubscribe on worker teardown / `_mfb_shutdown` so an exited thread never pins `base` (§4.5).
- [ ] Add the deprecation warning for implicit single-threaded reliance (warn now; require explicit `openStdIn` in a later release).

Acceptance: no hang/leak on shutdown with a blocked reader; the thread-transfer and shutdown suites stay green; memory stays bounded under a deliberately stalled subscriber (cap holds, no unbounded growth).
Commit: 31312365 (auto-unsubscribe). **Auto-unsubscribe DONE + verified** — a two-worker early-exit at 300k lines (>4 MiB; one worker reads 5 lines then exits, the other reads to EOF) completes deterministically across repeated runs, impossible without the teardown release; 62-test thread suite stays green.

**Backwards-compat removal (2026-07-13, user-directed).** The compiler-inserted
main-thread compat subscription was **removed**: every thread — including main —
must now call `thread::openStdIn` before reading stdin, or the read traps
`ErrInvalidContext`. This supersedes the deprecation-warning approach (a warning on
every single-threaded program conflicted with byte-identical and would churn
goldens). `subscribe_stdin`/the entry shim are deleted; `examples/` (hello_input,
hangman, life, audio) and the docs (io/thread man pages, `spec threading
os-integration`) are updated to the explicit-`openStdIn` idiom. Golden-neutral: no
committed golden captures a stdin program's entry codegen, and the compile-only
`func_io_*` tests read behind `IF FALSE` (no runtime), so they still pass unchanged.
Verified: `readLine` without `openStdIn` → `code=77050019`; with `openStdIn` → reads.

**D3 `project.json` `"config"` cap: DONE.** A `"config"` section with `stdinLogCap`
(bytes) is read from the manifest (`manifest::stdin_log_cap`), threaded via the
executable path (`write_executable` → `lower_project` → `NirModule.stdin_log_cap`)
and baked into `_mfb_rt_stdin_next_byte`'s backpressure compare; absent/invalid/
<8 KiB falls back to `STDIN_LOG_CAP_DEFAULT` (4 MiB). Documented in `spec tooling
project-manifest`. Verified: `config.stdinLogCap = 16777216` bakes `16777216` into
the NIR; no config bakes `4194304`.

**D4 self-pipe: not implemented — unnecessary for this runtime.** MFBASIC detaches
workers and shuts down by process exit (`exit_group`), which terminates a worker
parked in a blocking `read(0,…)`; a worker blocked in `readLine` while main returns
exits cleanly (verified, exit 0). The self-pipe/`poll` machinery would be dead code
— the "shutdown must never hang on a parked reader" criterion is already met. The
reader blocks directly in `read(0,…)` (EINTR + `shuttingDown` handled); SIGINT
delivers `_exit(130)` from the handler.

## Validation Plan

- **Function tests:** `tests/func_thread_openStdIn_valid/**` + `_invalid/**` and
  `tests/func_thread_closeStdIn_valid/**` + `_invalid/**`, full overload coverage
  (no-arg self form and worker-handle form; invalid: wrong arg type, double-close,
  read-while-unsubscribed). The stdin `io::` family
  (`func_io_readLine_*`/`readChar_*`/`readByte_*`/`input_*`) gains broadcast cases.
- **Runtime proof (not golden alone):** (a) a piped 100k-line program proving the
  `read()` syscall count collapses (e.g. `strace`/`dtruss` count, or a
  syscall-counting wrapper) and io-read median in range; (b) a two-worker program,
  each subscribed, asserting both receive the identical full line sequence; (c) a
  late-join program asserting the late worker sees a strict suffix; (d) a
  stalled-subscriber program asserting the log stays under `STDIN_LOG_CAP` and the
  producer blocks rather than growing; (e) a SIGINT-during-read program asserting
  exit 130 and no deadlock.
- **Soundness regression:** the existing thread-transfer and shutdown/signal suites
  must stay green (the log is new cross-thread state; deadlock/lost-wakeup hides
  here).
- **Doc sync:** `mfb spec io` (broadcast + single-consumer guarantee), `mfb spec
  threading` (`openStdIn`/`closeStdIn`, subscription lifecycle, the not-subscribed
  error), `mfb spec memory` (arena-state growth), `mfb spec diagnostics`
  (new error code per D1), `mfb spec tooling project-manifest` (the new
  `project.json` `"config"` section per D3; canonical
  `src/docs/spec/tooling/01_project-manifest.md`), and man pages for the two builtins.
- **Acceptance:** `scripts/test-accept.sh target/debug/mfb target/accept-actual`.
- **Benchmarks:** re-run `benchmark/io-read` (and confirm no regression elsewhere);
  compare the `med` column to the §1 target.

## Resolved Decisions

All open decisions are settled (2026-06-27); each took the recommended option.

- **D1 — not-subscribed read = trap `ErrInvalidContext`.** A defined error, not
  silent EOF (which would make a forgotten `openStdIn` look like empty input).
  Allocate/choose the code in `mfb spec diagnostics`. (§4.3)
- **D2 — block on backpressure, never error or drop.** Block on the cv (consistent
  with transfer-queue blocking), released by `base` advancing, any `closeStdIn`, or
  `shuttingDown`. (§4.1)
- **D3 — fixed high-water cap (4 MiB default), configurable at build time.** A
  constant, not lag-relative (a lag-relative cap does not bound). **Configuration
  surface: a new `"config"` section in `project.json`** holding the cap (and a home
  for future runtime tunables), read at build time and baked into the binary — not
  an env var or runtime `io::`/`thread::` setter. Specified in
  `mfb spec tooling project-manifest`. (§4.1)
- **D4 — orderly-shutdown interrupt of a parked `read(0)`: `poll(fd0 + self-pipe)`.**
  The reader `poll`s `fd 0` alongside a runtime self-pipe so `_mfb_shutdown` can wake
  it deterministically, independent of whether the thread model joins — a shutdown
  must never hang on a parked reader. (§4.6)
- **D5 — single join point (current frontier) only.** One `openStdIn`; no "from the
  absolute start" variant (unbounded / nondeterministic). Subscribe from the
  absolute start is rejected. (§4.5)

## Non-Goals

- Output buffering (`io::write`/`print`) — plan-14-io-buffering.md.
- Seekable-file read buffering — `fs::` already block-reads.
- A "replay from byte 0 for late joiners" mode — would require unbounded retention.
- Per-subscriber independent *backpressure policies* — one global cap in V1.

## Summary

The runtime takes ownership of `fd 0` and serves it from one append-only log with a
per-subscriber cursor, so the syscall storm disappears and `stdin` becomes a
per-thread broadcast under explicit `thread::openStdIn`/`closeStdIn`. A
compiler-inserted main-thread subscription keeps every existing single-threaded
program byte-identical, so the new semantic is opt-in for the multi-thread case
only. The real engineering risk is the lock discipline around the blocking
`read(0)` (one reader at a time, syscall never under the data lock, a correct
condvar wake-set), the memory bound (reclaim-at-min vs. a fixed backpressure cap),
and the `ARENA_STATE` growth — all contained by reusing the transfer queue's proven
`pthread_mutex`/`cond` pattern and by landing the byte-identical single-consumer
path before any broadcast behavior. What stays untouched: value/collection layout
and copy/transfer, output and file I/O, and the thread-isolation model (the log is
the one new shared object, lock-guarded and arena-pointer-free).
