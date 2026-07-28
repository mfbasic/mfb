# bug-64: `os::getEnv`/`os::environ`/`os::userName` wrap non-thread-safe libc globals that race concurrent `os::setEnv`/`os::unsetEnv` (use-after-free / torn reads)

Last updated: 2026-07-09
Effort: small (<1h)

The `os::` package env and user helpers wrap inherently non-thread-safe C APIs with no
synchronization. In a multithreaded MFBASIC program, thread A running `os::environ()` (or
holding a pointer returned by `os::getEnv`) while thread B runs `os::setEnv`/`os::unsetEnv`
races on libc's `environ` array: `setenv` can reallocate/relocate `environ` and free the
old strings, so A's walk or the `getenv` result becomes a use-after-free / torn read.
`os::userName` similarly returns a pointer into libc's static `getpwuid` buffer, which a
concurrent `getpwuid`/`getpwnam` overwrites.

This is the same class as the known OS-08 (chdir thread-CWD) finding: a process-global libc
resource accessed without a lock across MFBASIC threads. Severity LOW — single-threaded use
is safe, and the helpers marshal into arena memory promptly, shrinking (not closing) the
window. The single correct behavior a fix produces: concurrent env/pwd access from MFBASIC
threads does not read freed/torn memory (via serialization or the `*_r` variants), or the
thread-unsafety is explicitly documented as a constraint.

References (all under `src/target/shared/code/os.rs`):

- `getenv` at `:259` and `:368` (result pointer read at `:261`).
- `lower_environ` walking `environ` at `:591-748` (entries read at `:599`).
- `getpwuid` static `passwd` at `:1030-1037` (`pw_name` read at `:1037`).
- `setenv` at `:444` — the concurrent mutator.
- Same class: OS-08 (chdir thread-CWD). Thread model: MFBASIC threads share the process,
  per the thread-resource-plane design.
- Found during the goal-01 compiler source review of `src/target/shared/code/`.

## Failing Reproduction

A two-thread program: thread A loops `os::environ()` / `os::getEnv("X")`; thread B loops
`os::setEnv("X", ...)` / `os::unsetEnv("X")`.

- Observed: intermittent crash or garbage strings under a race detector / ASan, as A reads
  an `environ` entry B has freed or relocated, or the `getpwuid` buffer is overwritten
  mid-marshal.
- Expected: no use-after-free / torn read; each `os::` call returns a self-consistent
  snapshot or is serialized.

Contrast: single-threaded env/pwd use is safe (the same idiom the fs helpers use); the
marshal into arena memory shrinks the window but does not close it.

## Root Cause

`os::getEnv`/`environ`/`userName` read `getenv`/`environ`/`getpwuid` — process-global, not
thread-safe against a concurrent `setenv`/`getpwuid` — with no lock and no use of the `_r`
reentrant variants. The pointers they read have no lifetime guarantee against a concurrent
mutator.

## Goal

- Concurrent `os::` env/pwd access from MFBASIC threads does not read freed/torn memory.
- Either the access is serialized behind a runtime lock, or the `_r` variants are used
  (`getpwuid_r`), or the thread-unsafety is documented as an explicit constraint.

### Non-goals (must NOT change)

- Single-threaded behavior.
- The `os::` API surface.

## Blast Radius

- `os::getEnv`, `os::environ`, `os::setEnv`, `os::unsetEnv`, `os::userName` — all share the
  process-global libc state.
- OS-08 (chdir) is the sibling; a shared "os-global lock" could cover both.

## Fix Design

Serialize env/pwd access behind a single runtime mutex (the same one OS-08 would use), so a
reader holds it across the marshal-into-arena and a writer holds it across `setenv`; and/or
switch `userName` to `getpwuid_r` with a caller-provided buffer. If a lock is deemed too
heavy for the common single-threaded case, document `os::` env/pwd helpers as not
thread-safe against concurrent mutation and rely on program discipline — but a lock is the
correct default.

## Phases

### Phase 1 — decision + test

- [x] Decide: runtime lock vs `_r` variants vs documented constraint (recommend lock, share
      with OS-08). Add a two-thread race test under a sanitizer.

### Phase 2 — the fix

- [x] Serialize env/pwd access (and/or `getpwuid_r`), or land the documentation + guard.

### Phase 3 — validation

- [x] `scripts/test-accept.sh`; race test clean under the sanitizer; single-threaded
      behavior unchanged.

## Validation Plan

- Regression test(s): the concurrent env-read/env-mutate sanitizer test.
- Runtime proof: race detector clean.
- Doc sync: `os::` man pages note the thread-safety guarantee (or constraint).
- Full suite: `scripts/test-accept.sh`.

## Summary

`os::` env and user helpers read non-thread-safe libc globals that a concurrent
`os::setEnv` can free or relocate, a use-after-free reachable from a multithreaded program.
Same class as OS-08; the fix is a shared runtime lock (or `_r` variants), or an explicit
documented constraint. LOW — single-threaded use is safe.

## Resolution

Fixed by serializing every `os::` env/pwd helper behind a single process-global
`pthread_mutex_t` (the runtime already uses pthread mutexes for its thread queues;
the compiler has no atomics, so a lock is the only viable primitive). Chose the lock
over the `_r` variants because `getenv`/`environ` have no reentrant form — a lock is
required for env regardless, so it also covers `userName` (no need for `getpwuid_r`).

Decision: **runtime lock**, shared across all env/pwd helpers (the OS-08 sibling can
reuse the same global).

### Implementation

- `src/target/shared/code/os.rs`
  - New `_mfb_rt_os_env_lock` global (`OS_ENV_LOCK_SYMBOL`), a 64-byte
    `pthread_mutex_t` (covers the largest supported libc; glibc-aarch64 = 48,
    macOS = 64). `os_env_lock_init_hex(target)` emits its **static**
    `PTHREAD_MUTEX_INITIALIZER` bytes so no runtime initializer runs and there is no
    init race: all-zero on Linux (glibc/musl); `_PTHREAD_MUTEX_SIG_init` (0x32AAABA7)
    in the first `__sig` word on macOS, which libc lazily first-use-initializes on the
    first `pthread_mutex_lock`.
  - `emit_env_lock` takes the lock at helper entry (after incoming `String*` args are
    saved to vregs); `emit_env_unlock_return` releases it at the single `done` label,
    preserving the four result registers (tag/value/message/source) across the
    caller-saved-clobbering `pthread_mutex_unlock` via vregs the allocator keeps live.
    Wired into `getEnv`/`getEnvOr`/`hasEnv`/`environ`/`setEnv`/`unsetEnv`/`userName`.
    The lock is held across the marshal-into-arena / two-pass `environ` walk / pwd copy,
    so a reader never touches memory a concurrent `setenv` relocates or frees.
    (`hostName`/`executablePath` read only a local stack buffer — no shared global, no
    lock.)
- `src/target/shared/code/mod.rs` — emits the writable `_mfb_rt_os_env_lock` data
  object (target-specific init bytes) when the module uses any env/pwd helper.
- `src/target/shared/plan/symbols.rs` — adds the platform-correct
  `pthread_mutex_lock`/`unlock` imports (borrowed from the `thread.drop` import set,
  so `_pthread_mutex_*`/libSystem on macOS and libpthread/libc on Linux), attributed
  to each env/pwd helper the module emits.
- `tests/rt-behavior/os/os-env-thread-race-rt/` — new regression test: two writer
  threads relocate `environ` while two reader threads walk it; asserts deterministic
  success (`env race survived`, exit 0).

### Runtime proof (macOS-aarch64)

Four-thread stress harness (2 writers × `os::setEnv`/`unsetEnv`, 2 readers ×
`os::environ`/`os::getEnv`, 20 000 iters each):

- **Before** (lock temporarily neutralized): 20/20 plain runs failed with
  `77050010 numeric overflow` (a torn `environ` entry length corrupted the
  accumulator); 8/8 runs crashed with SIGSEGV (exit 139) under Guard Malloc
  (`DYLD_INSERT_LIBRARIES=/usr/lib/libgmalloc.dylib`) — confirmed use-after-free.
- **After** (fix): 0/20 plain failures and 0/8 Guard-Malloc failures; all runs exit 0.

Single-threaded behavior unchanged: all seven `os::` env/pwd rt-behavior fixtures
(`func_os_getEnv/environ/userName/unsetEnv/hasEnv/...`) still match their `.run`
goldens; 2442 in-tree unit tests pass.

Goldens: the ast/ir/build.log/native goldens for every `os::` env/pwd program shift
(new writable global + `pthread_mutex_*` imports + lock/unlock instructions). The
orchestrator regenerates them via `scripts/test-accept.sh`.
