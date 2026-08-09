# plan-91-B: thread::sleep — worker-side cancellation-aware sleep

Last updated: 2026-08-09
Effort: medium (1h–2h)
Depends on: plan-91-A (parent-side `thread::sleep` complete and landed)

Add the worker-side overload `thread::sleep(t AS ThreadWorker OF Msg TO Out, ms
AS Integer)`. Like the package's other worker-side blocking waits
(`receive`/`send`/`accept`), a worker sleep in progress wakes early and fails
with `ErrInterrupted` (77050009) when the parent requests cancellation
(`thread::cancel`, or dropping the parent handle). A worker sleep that runs to
completion without cancellation returns after ≥ `ms` ms.

The single behavioral outcome of plan-91-B: inside an `ISOLATED FUNC` worker, a
`thread::sleep(t, 500)` returns `Ok` after ~500 ms when left alone, but returns
`ErrInterrupted` promptly (well before 500 ms) when the parent calls
`thread::cancel(t)` mid-sleep.

References:

- `mfb man thread isCancelled`, `mfb man thread receive` — the worker-side
  cancellation contract this overload joins ("Runtime-managed worker queue waits
  … wake and fail with `ErrInterrupted` when cancellation is requested").
- `.ai/arch-abi.md`, `.ai/codegen-invariants.md`, `.ai/testing-gates.md`,
  `.ai/collections.md` — read before the codegen/gate work.
- `src/docs/spec/threading/*.md`, `src/docs/spec/language/16_threads.md`.
- `.ai/man_template.md` — the `sleep.md` update follows the same skeleton.

## Prerequisites

These are a precondition on this sub-plan, not a dependency to negotiate.

| Must be true | Command | Status |
|---|---|---|
| plan-91-A complete: parent `thread::sleep` shipped & green | `rg -n '"thread.sleep"' src/target/shared/code/mod.rs` (dispatch arm present) AND `cargo test builtins::thread` green | MET — dispatch arm at mod.rs:2254; `cargo test --bin mfb builtins::thread` 24 passed (worktree P-91) |
| plan-91-A archived or in-tree, parent sleep spec/catalog present | `rg -n THREAD_SLEEP_SPEC src/target/shared/runtime/` | MET — THREAD_SLEEP_SPEC in thread_specs.rs:64 + catalog.rs:171 (worktree P-91) |

If plan-91-A is not complete, plan-91-B cannot start, full stop. plan-91-B does
NOT re-implement or promote any part of 91-A; it extends the already-registered
`thread.sleep` name with a second overload and a second runtime helper.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command before you continue and before you decide to stop.

## 1. Goal

- `thread::sleep(t AS ThreadWorker OF Msg TO Out, ms AS Integer)` type-checks
  inside `ISOLATED FUNC` worker code and returns `Nothing`.
- `ms < 0` → `ErrInvalidArgument`; `ms = 0` → immediate no-op (same convention as
  the parent form).
- `ms > 0` with no cancellation → returns after ≥ `ms` ms.
- `ms > 0` with cancellation requested during the sleep → wakes promptly and
  fails with `ErrInterrupted` (77050009), matching worker `receive`/`send`.
- A parent-side `send` arriving during a worker sleep does NOT shorten it (the
  deadline is absolute; a spurious condvar wake re-enters the wait for the
  remaining time).
- Works on macOS AArch64, Linux x86-64, Linux AArch64, and Windows x86-64.

### Non-goals (explicit constraints)

- No change to the parent-side overload shipped in plan-91-A.
- No new field in the 120-byte thread control block. The worker sleep reuses the
  existing inbound-queue mutex + not-empty condition variable (which
  `thread::cancel` already broadcasts) — see Verified properties.
- No asynchronous kill: cancellation stays cooperative. The worker sleep only
  observes the cancel flag / condvar broadcast; it does not interrupt
  package/native code (consistent with `mfb man thread`).
- No change to any unrelated program's `.ncode`.

## 2. Current State

- **Direction split (parent vs worker) happens at codegen:**
  `src/target/shared/code/builder_values.rs:2093-2156`. A user call inspects the
  *static type* of the handle argument and rewrites the runtime target — e.g.
  `thread.send` → `thread.emit` for a `ThreadWorker`, `thread.receive` →
  `thread.read` for a parent `Thread`. plan-91-B adds the analogous split for
  `thread.sleep`: worker handle → a new internal target (e.g.
  `thread.sleepWorker`); parent handle keeps `thread.sleep` (91-A's helper).
- **Worker-side blocking-wait precedent:** the `ThreadReadMode::WorkerSelf` path
  dispatched in `runtime_helpers.rs:430-488` reads the worker's own inbound queue
  — locks the inbound-queue mutex, waits on its not-empty condvar, and fails with
  `ErrInterrupted` when cancellation is observed. This is the exact wait/lock/
  interrupt shape the worker sleep mirrors, minus the "dequeue a message" step.
- **Cancel path:** `thread::cancel` sets `THREAD_OFFSET_CANCELLED` (offset 8),
  closes the worker's data and resource queues, and broadcasts their condition
  variables (`mfb man thread cancel`/`isCancelled`). The inbound queue's
  not-empty condvar is therefore broadcast on cancel.
- **Absolute-deadline math:** `emit_thread_deadline`
  (`runtime_helpers_thread.rs:11-61`) turns `ms` into an absolute `timespec`
  (`clock_gettime` + `sec = ms/1000`, `nsec = (ms%1000)*1e6`, with nsec carry) —
  exactly what `pthread_cond_timedwait` consumes.
- **Windows condvar wait:** `emit_windows_thread_call`
  (`runtime_helpers.rs:150-168`) already translates `pthread_cond_timedwait` to
  `SleepConditionVariableSRW` with a 20 ms poll and an `ETIMEDOUT` (110) return
  contract, and `pthread_mutex_lock`/`unlock` to `AcquireSRWLockExclusive`/
  `ReleaseSRWLockExclusive`. So a worker sleep built from those primitives gets
  Windows support with NO new Win32 arm.
- **Thread control block offsets:** `runtime_helpers.rs:3-50` —
  `THREAD_OFFSET_CANCELLED = 8`, `THREAD_OFFSET_INBOUND_QUEUE = 40`; queue block
  `THREAD_QUEUE_NOT_EMPTY_OFFSET = 64` (condvar), plus the queue's own mutex.

### Measured populations

| What | Count | Command |
|---|---|---|
| Thread runtime specs (before 91-B) | 17 + 1 (91-A) | `rg -c RuntimeHelperSpec …/thread_specs.rs` (re-run after 91-A lands) |
| Worker blocking-wait precedents to mirror | 3 (receive, send, accept worker forms) | `rg -n 'WorkerSelf\|thread\.emit\|thread\.receive' src/target/shared/code/runtime_helpers.rs` |

Re-run these after plan-91-A lands, before scheduling 91-B's phases — 91-A adds
one spec and may shift line numbers.

### Verified properties

Two claims the whole design rests on. **Verify both as Phase 1 tasks before
writing the helper** — they are cheap reads and they de-risk everything after.

- **UNVERIFIED — `thread::cancel` broadcasts the inbound-queue not-empty
  condvar the worker sleep will wait on.** Read the cancel helper and confirm it
  broadcasts `THREAD_OFFSET_INBOUND_QUEUE`'s `THREAD_QUEUE_NOT_EMPTY_OFFSET`
  condvar (the man page says it broadcasts the data queues' condition variables;
  confirm in code which condvar and under which mutex).
- **UNVERIFIED — the worker inbound-queue wait uses that same mutex+condvar and
  checks `THREAD_OFFSET_CANCELLED` to raise `ErrInterrupted`.** Read the
  `ThreadReadMode::WorkerSelf` receive path (`runtime_helpers.rs:430-488`) and
  copy its lock/wait/cancel-check structure verbatim, dropping the dequeue.

If either read shows the inbound condvar is the wrong wake source, fall back to
the alternative in §3 (dedicated cancel condvar) — but do NOT braid: that becomes
a documented correction here, not silent scope.

## 3. Design Overview

Three layered pieces, mirroring plan-91-A's structure:

1. **Descriptor.** Add a second overload to the already-registered `thread.sleep`
   name: `(ThreadWorker OF Msg TO Out, Integer) → Nothing`, mirroring how
   `thread::receive`/`send` accept both handle sides. Flip
   `func_thread_sleep_worker_invalid` (added in 91-A) to a `_valid` test.

2. **Direction split + worker helper.** In `builder_values.rs:2093-2156`, add the
   `thread.sleep` case: if the handle's static type is a worker
   (`is_worker_thread_type`), rewrite the target to `thread.sleepWorker`; else
   leave `thread.sleep`. Register `THREAD_SLEEP_WORKER_SPEC`
   (returns `Nothing`), add it to the catalog, add the dispatch arm, and
   implement `lower_thread_sleep_worker_helper`:

   - `ms < 0` → `ErrInvalidArgument`; `ms == 0` → `Ok` (same as parent).
   - Compute the ABSOLUTE deadline with `emit_thread_deadline`.
   - Lock the inbound-queue mutex. Loop:
     - if `THREAD_OFFSET_CANCELLED` is set → unlock, return `ErrInterrupted`;
     - if now ≥ deadline → unlock, return `Ok`;
     - else `pthread_cond_timedwait(inbound_not_empty, inbound_mutex, &deadline)`
       and re-loop. A spurious wake (parent `send` broadcasts not-empty) simply
       re-checks the flag and deadline; because the deadline is absolute, the
       remaining sleep is preserved — a send never shortens the sleep.
   - Windows: the `pthread_mutex_*` / `pthread_cond_timedwait` primitives already
     translate to SRWLOCK / `SleepConditionVariableSRW` (`runtime_helpers.rs:
     101-168`), so no new Win32 arm is required.

3. **Docs/spec/gates.** Revise `sleep.md` to add the worker overload and a
   cancellation section; update the spec; refresh byte-identity.

**Where correctness risk concentrates:** the worker wait loop (piece 2) — lock
discipline (never return while holding the mutex), the cancel-flag/deadline
ordering, and the "spurious wake does not shorten the sleep" property. It is
scheduled last, behind an rt-behavior test that asserts BOTH the full-duration
path and the cancel-wakes-early path.

**Byte-identity is NOT the core gate.** This adds behavior; the gate is the
rt-behavior cancel test. Byte-identity only pins unrelated programs as unchanged.

**Rejected alternatives:**
- *Dedicated per-thread "cancel" condvar in the TCB* — rejected as the default:
  it adds a control-block field and requires `cancel` to broadcast a new condvar,
  more infrastructure than reusing the inbound condvar `cancel` already
  broadcasts. Kept only as the fallback if the Verified-property read disproves
  the inbound-condvar approach.
- *Worker sleep = `nanosleep` (like the parent form)* — rejected: `nanosleep`
  cannot be woken by `cancel`, so it would violate the package invariant that
  worker waits wake on cancellation.
- *Wait on a fresh local condvar the worker owns* — rejected: `cancel` has no
  handle to that local condvar, so it could never wake it.

## 4. Detailed Design — `lower_thread_sleep_worker_helper`

```
entry:
  ; c_arg(0) = worker handle (or current-thread reg, per WorkerSelf convention),
  ; c_arg(1) = ms
  move %vMS, c_arg(1)
  compare %vMS, 0
  branch_lt  err_arg          ; ms < 0 → ErrInvalidArgument
  branch_eq  ok               ; ms == 0 → no-op
  ; absolute deadline
  store %vMS -> [sp + timeout]
  emit_thread_deadline(timeout_off, deadline_off)   ; deadline = now + ms
  ; resolve inbound queue base + its mutex/condvar (mirror WorkerSelf receive)
  load %vQ, [handle + THREAD_OFFSET_INBOUND_QUEUE(40)]
  pthread_mutex_lock(%vQ + <mutex off>)
loop:
  load %vC, [handle + THREAD_OFFSET_CANCELLED(8)]
  compare %vC, 0
  branch_ne  interrupted      ; cancel requested → ErrInterrupted
  clock_gettime(now); if now >= [deadline] branch done
  pthread_cond_timedwait(%vQ + NOT_EMPTY(64), %vQ + <mutex off>, &deadline)
  branch loop                 ; spurious/timeout/cancel wake → re-check
done:
  pthread_mutex_unlock(%vQ + <mutex off>); goto ok
interrupted:
  pthread_mutex_unlock(%vQ + <mutex off>); set ErrInterrupted; return
ok:
  set RESULT_OK; return
err_arg:
  set ErrInvalidArgument; return
```

Copy the exact mutex offset, condvar offset, `clock_gettime`-vs-deadline compare,
and lock/unlock emission from the `ThreadReadMode::WorkerSelf` receive path
(`runtime_helpers.rs:430-488`) — do not re-derive them. The ONLY structural
differences from worker `receive` are: (a) the loop's success exit is "deadline
reached" instead of "message dequeued", and (b) there is no dequeue/copy step.

## Compatibility / Format Impact

- **Adds** a second overload to `thread::sleep` (worker) and one runtime symbol
  `_mfb_rt_thread_thread_sleepWorker`.
- **Unchanged:** the parent overload (91-A), all other `thread::` functions, the
  thread control block layout (no new field), and unrelated programs' `.ncode`.

## Phases

> Tick `- [x]` in the same commit as the work; fill each `Commit:` when it lands.

### Phase 1 — Verify design premises + descriptor overload

Do the two cheap reads FIRST (they can redirect the whole design), then add the
worker overload.

- [x] Verify property 1: read the `cancel` helper; confirms it broadcasts the
      inbound-queue not-empty condvar (recorded in Corrections,
      `runtime_helpers_thread.rs:331-346`).
- [x] Verify property 2: read `ThreadReadMode::WorkerSelf` receive
      (`runtime_helpers_thread.rs:1139-1368`); recorded the mutex offset (queue
      base + 0), condvar offset (+64), and cancel-check (handle+8) in Corrections.
- [x] `src/builtins/thread.rs`: added the worker handle to `thread.sleep`.
      Following the `send`/`receive` idiom (a SINGLE `ov(P_SLEEP)` overload whose
      Custom return type is resolved by `resolve_call`), the change is in
      `resolve_call` (`is_parent_thread_type` → `is_thread_type`) and
      `expected_arguments` (adds "or ThreadWorker …"); `call_param_names` is
      handle-side-agnostic and unchanged. No separate `P_SLEEP_WORKER` is needed
      (see Corrections).
- [x] Tests: flipped `tests/syntax/threads/func_thread_sleep_worker_invalid` →
      `func_thread_sleep_worker_valid` (build succeeds, IR shows `thread.sleep`;
      the worker split lands at codegen in Phase 2). Negative-arg cases stay in
      `func_thread_sleep_invalid`. Updated inline unit test
      `resolve_sleep_parent_only` → `resolve_sleep_both_handle_sides`.

Acceptance: both premises confirmed in writing (Corrections) AND
`cargo test --bin mfb builtins::thread` green (24 passed) AND a worker-handle
`thread::sleep(t, ms)` type-checks to `Nothing` (syntax test green).
Commit: b179f68d2

### Phase 2 — Worker helper + direction split (largest blast radius)

- [x] `src/target/shared/code/builder_values.rs`: added the `"thread.sleep"` case
      rewriting to `"thread.sleepWorker"` when the handle static type is a worker
      (mirrors the send→emit / receive→read split).
- [x] `thread_specs.rs`: added `THREAD_SLEEP_WORKER_SPEC` (`returns: "Nothing"`).
      `catalog.rs`: registered it + added `"thread.sleepWorker"` to
      `CODE_LAYER_ONLY_CALLS`. `mod.rs`: routed `"thread.sleepWorker"` to
      `lower_thread_helper` AND force-emit its helper body whenever
      `_mfb_rt_thread_thread_sleep` is present (companion emission — see Corrections).
- [x] `runtime_helpers.rs`: added the `"thread.sleepWorker" => …` dispatch arm;
      implemented `lower_thread_sleep_worker_helper` in `runtime_helpers_thread.rs`
      per §4 (reuses `emit_thread_deadline` + the WorkerSelf lock/wait/cancel
      structure; absolute deadline, cancel-check → `ErrInterrupted`, spurious wake
      re-loops without shortening the sleep).
- [x] Advertised `thread.sleepWorker`: added to the macOS/Linux pthread import
      arms and the Windows `mod.rs` runtime_calls (mirroring `thread.emit`). No new
      libc import needed — pthread mutex/cond are pulled by `thread.start`; Windows
      reuses the existing `pthread_cond_timedwait → SleepConditionVariableSRW`
      translation (confirmed in the cross-compiled `-ncode`).
- [x] Tests: `tests/rt-behavior/threads/thread-sleep-worker-rt` — worker
      `sleepThenReturn` sleeps 200 ms then returns 5; asserts full-duration
      completion (elapsed ≥ 150 ms + result 5) → "result 5"/"slept full".
      `tests/rt-behavior/threads/thread-sleep-worker-cancel-rt` — worker
      `sleepUntilCancel` starts a 5000 ms sleep; parent waits ~50 ms, cancels, and
      `waitFor` auto-propagates the worker's `ErrInterrupted` (77050009), caught in
      ~0.2 s → "interrupted". Two new workers added to `thread_runtime_workers`.

Acceptance: the cancel test proves the worker sleep wakes early with
`ErrInterrupted` (observed "interrupted", ~0.2 s ≪ 5000 ms), AND the no-cancel
test proves it sleeps the full duration ("slept full"), AND full `cargo test` is
green (0 failures). All 47 thread fixtures pass with the regenerated `.mfp`.
Cross-compiles clean for linux-x86_64/aarch64/windows-x86_64.
Commit: —

### Phase 3 — Docs, spec, byte-identity

- [ ] `src/docs/man/builtins/thread/sleep.md`: add the worker overload to
      Synopsis/Parameters and a "Cancellation" paragraph (worker sleep wakes with
      `ErrInterrupted` on `thread::cancel`; parent sleep is uninterruptible). Add
      `ErrInterrupted` to the Errors table. Keep `.ai/man_template.md` structure.
- [ ] Spec: update `src/docs/spec/threading/*.md` and `language/16_threads.md`
      to document the worker-side cancellation-aware sleep.
- [ ] Byte-identity: refresh the `tests/byte-identity/thread/` fixtures — pin a
      worker-sleep program's `.ncode` and confirm unrelated programs are
      unchanged from the 91-A baseline.

Acceptance: man-coverage and spec-sync gates pass; full `cargo test` (including
byte-identity + acceptance goldens) is green.
Commit: —

## Validation Plan

- Tests: syntax (worker-valid now), rt-behavior (full-duration completion;
  cancel-wakes-early with `ErrInterrupted`; send-does-not-shorten), rt-error
  (worker `ms < 0`), byte-identity (worker-sleep pinned; unrelated unchanged),
  descriptor unit tests.
- Coverage check: the cancel rt-behavior test must actually run a worker on the
  host target and observe the early `ErrInterrupted` — confirm the new test dirs
  are in the runtime suite so a green gate means the wait loop executed.
- Runtime proof: a program whose worker sleeps 2000 ms while the parent cancels
  at ~100 ms returns `ErrInterrupted` in well under 2000 ms (observable
  wall-clock); the same program without the cancel returns after ~2000 ms.
- Doc sync: `sleep.md` (both overloads), spec threading/16_threads; gates green.
- Acceptance: full `cargo test`; rustfmt/clippy per `.ai/build-tooling.md`.

## Open Decisions

- **Internal worker target name.** Recommended `thread.sleepWorker`
  (parallels the `thread.emit`/`thread.read` internal names). Alternative
  `thread.sleepInterruptible`. Cosmetic; pick one and use it consistently across
  the spec/catalog/dispatch.
- **Which condvar the worker waits on.** Recommended: reuse the inbound-queue
  not-empty condvar `cancel` already broadcasts (no new TCB field). Fallback: a
  dedicated cancel condvar — only if Phase 1's Verified-property read disproves
  the inbound-condvar approach. (§3)

## Corrections

- **Premise 1 CONFIRMED** — `thread::cancel` broadcasts the inbound-queue
  not-empty condvar. `simple_thread_handle_helper` `ThreadSimpleOp::Cancel`
  (`src/target/shared/code/runtime_helpers_thread.rs:326-346`): under the
  inbound-queue mutex it sets `THREAD_OFFSET_CANCELLED = 1` (line 331-332),
  stores the queue's `THREAD_QUEUE_CLOSED_OFFSET = 1` (333-334), then
  `pthread_cond_broadcast`s `inbound + THREAD_QUEUE_NOT_EMPTY_OFFSET` (335-346).
  So the inbound not-empty condvar IS the wake source cancel drives.
- **Premise 2 CONFIRMED** — the worker inbound wait uses that same mutex+condvar
  and checks the cancel flag → `ErrInterrupted`. `thread_queue_read_helper` with
  `ThreadReadMode::WorkerSelf` (`runtime_helpers_thread.rs:1139-1368`): mutex =
  the queue base pointer passed directly to `pthread_mutex_lock` (queue offset 0,
  lines 1210-1222); in the wait loop, when `worker_self`, it loads
  `THREAD_OFFSET_CANCELLED` (handle+8) and branches to `interrupted` →
  `ERR_INTERRUPTED_CODE` when nonzero (1253-1259, 1366-1368); it waits on
  `queue + THREAD_QUEUE_NOT_EMPTY_OFFSET` via `pthread_cond_timedwait` (1286-1298)
  / `pthread_cond_wait` (1305-1316). The inbound-condvar design in §3/§4 stands —
  no fallback to a dedicated cancel condvar needed.
- **No separate `P_SLEEP_WORKER` overload.** Phase 1 planned
  `ov(P_SLEEP_WORKER)`, but the thread package's dual-handle calls
  (`send`/`receive`) use ONE `ov(P_*)` overload with a `Thread`-typed first param
  and let `resolve_call` (return type `Custom`) accept either handle via
  `is_thread_type`. The descriptor param type is not independently enforced for
  these Custom-return functions, so adding a second overload is unnecessary and
  would diverge from the established idiom. Matched it: single `ov(P_SLEEP)`,
  `resolve_call` widened to `is_thread_type`.
- **Key offsets for the worker sleep helper** (from the two reads above):
  inbound queue base = `handle + THREAD_OFFSET_INBOUND_QUEUE (40)`; mutex =
  queue base + 0; not-empty condvar = queue base + `THREAD_QUEUE_NOT_EMPTY_OFFSET (64)`;
  cancel flag = `handle + THREAD_OFFSET_CANCELLED (8)`.
- **The synthesized `thread.sleepWorker` helper body must be force-emitted.** The
  NIR only names `thread.sleep`; the worker split to `thread.sleepWorker` happens
  in the code layer AFTER `runtime_symbols` is collected, so the call site emits a
  reloc to `_mfb_rt_thread_thread_sleepWorker` with no defining body → "native code
  internal relocation target ... is not defined". Fixed exactly like the
  send→emit / receive→read companions: in `code/mod.rs`, when
  `_mfb_rt_thread_thread_sleep` is in `runtime_symbols`, also push
  `_mfb_rt_thread_thread_sleepWorker`. (This is a wiring site the phase list did
  not call out; it is mandatory for any code-layer-split helper.)
- **The `thread_runtime_workers` package `.mfp` was regenerated** (added
  `sleepThenReturn`/`sleepUntilCancel`) and copied over all 24 committed consumer
  copies. Verified this churns no consumer golden: the consumers' `.ir` are
  byte-identical (unused workers don't affect a consumer's own lowering) and all
  47 thread fixtures pass under `test-accept`. Done surgically (single-package
  build + `find tests -name … -exec cp`), not via the tree-wide
  `sync-package-mfp.sh`, to bound the blast radius.

## Summary

The risk is concentrated in Phase 2's worker wait loop: lock discipline, the
cancel-flag/deadline ordering, and the absolute-deadline guarantee that a
spurious condvar wake (from a parent `send`) does not shorten the sleep. Phase 1
front-loads the two cheap reads that could redirect the design. Everything the
parent form already established (name registration, ms convention, target
plumbing) is reused, not rebuilt. Untouched: the thread control block layout, the
parent overload, and every other `thread::` function.
