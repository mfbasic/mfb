# plan-91-A: thread::sleep — parent-side plain sleep

Last updated: 2026-08-09
Overall Effort: large (3h–1d)   — the whole plan-91 `thread::sleep` feature
Effort: medium (1h–2h)
Depends on: nothing

Add a `thread::sleep(t AS Thread OF Msg TO Out, ms AS Integer)` builtin that
blocks the *calling* (parent) thread for `ms` milliseconds and returns nothing.
This sub-plan ships the parent-side overload end to end — descriptor, runtime
helper, all targets, docs, tests — as a complete, independently-valuable unit.
The worker-side, cancellation-aware overload is plan-91-B and lands after this.

The single behavioral outcome of plan-91-A: a program that calls
`thread::sleep(t, 50)` on a live parent `Thread` handle runs for at least ~50 ms
of wall-clock before the next statement, on every native target.

References:

- `mfb man thread` (package overview; `thread::sleep` is NOT yet listed — 12
  functions today: accept, cancel, closeStdIn, isCancelled, isRunning,
  openStdIn, poll, receive, send, start, transfer, waitFor).
- `.ai/arch-abi.md` (per-arch ABI traps — x86-64 SysV, Win64, riscv64, macOS
  AArch64), `.ai/codegen-invariants.md`, `.ai/testing-gates.md` — read before the
  codegen and gate work.
- `src/docs/spec/threading/*.md`, `src/docs/spec/language/16_threads.md` — spec
  sources of truth to keep in sync (AGENTS.md: spec stays current with every
  compiler change).
- `.ai/man_template.md` — the per-function man-page skeleton `sleep.md` must
  follow; `scripts/update_man.sh` carries the authoring rules.

## Prerequisites

These are a precondition on the whole feature, not a dependency to negotiate.

| Must be true | Command | Status |
|---|---|---|
| Working tree builds & thread tests green at HEAD | `cargo test builtins::thread` | MET — 23 passed; 0 failed (2026-08-09, worktree P-91) |
| No half-landed `thread::sleep` already present | `rg -n 'thread\.sleep' src/ → no matches` | MET (rg returned nothing 2026-08-09, worktree P-91) |

Everything below is written against the world where these hold.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again
> before you decide to stop.

## 1. Goal

- `thread::sleep(t AS Thread OF Msg TO Out, ms AS Integer)` type-checks, returns
  `Nothing`, and blocks the calling thread for the requested duration.
- `ms < 0` → `ErrInvalidArgument` (77050002) before sleeping.
- `ms = 0` → returns immediately (no-op), consistent with `poll`/`receive`'s
  `timeoutMs = 0` convention.
- `ms > 0` → the calling thread does not proceed until at least `ms` ms elapse
  (retrying across signal-interrupted `nanosleep`, so signals do not cut the
  sleep short).
- Works on macOS AArch64, Linux x86-64, Linux AArch64, and Windows x86-64.

### Non-goals (explicit constraints)

- No worker-side (`ThreadWorker`) overload — that is plan-91-B. In plan-91-A a
  worker calling `thread::sleep` MUST fail to type-check (proven by a negative
  test), exactly as any not-yet-supported overload does.
- No cancellation interaction. The parent-side sleep is a plain, uninterruptible
  wall-clock sleep; it does not read the cancel flag or touch any queue.
- No change to any existing `thread::` function's signature, semantics, ABI, the
  120-byte thread control block layout (`runtime_helpers.rs:3-50`), or the
  `.ncode` of any program that does not call `thread::sleep`.
- No new user-facing package; `thread::sleep` joins the existing built-in
  `thread` package.

## 2. Current State

- **Descriptor / function table:** `src/builtins/thread.rs` — call-name consts
  (`START` etc., lines 11-40), `THREAD_FUNCTIONS` table built via `tf(name,
  slug, overloads)` (lines 133-146), `const P_*` param arrays (105-131) built
  with `req`/`opt` (84-103). Return types are hand-resolved in `resolve_call`
  (237-309); `expected_arguments` (311-328) and `call_param_names` (208-228) are
  likewise hand-authored. The classifier test `is_thread_call_covers_every_name`
  (line 611) asserts every user name is covered. Parent/worker discriminators:
  `is_parent_thread_type` / `is_worker_thread_type` (386-392).
- **Per-target ABI spec:** `src/target/shared/runtime/thread_specs.rs` — 17
  `RuntimeHelperSpec`s today (`rg -c RuntimeHelperSpec … → 17`). Model:
  `THREAD_CANCEL_SPEC` (lines 23-27), `abi.returns = "Nothing"`.
- **Catalog:** `src/target/shared/runtime/catalog.rs:139-155` — supported-specs
  list; symbol derived as `_mfb_rt_thread_thread_sleep` via `symbol_for_call`
  (`runtime/mod.rs:49-64`).
- **Codegen dispatch:** `src/target/shared/code/mod.rs:2194-2217` routes
  `thread.*` calls to `lower_thread_helper`.
- **Helper bodies:** `src/target/shared/code/runtime_helpers.rs` —
  `lower_thread_helper` dispatch (389-494). Closest no-handle precedent:
  `lower_thread_stdin_subscription_helper` (501-532). External libc calls emitted
  via `emit_thread_external_call` (63-82); on Windows detours to
  `emit_windows_thread_call` (88-218), which already maps threading primitives to
  Win32 (e.g. `pthread_cond_timedwait` → `SleepConditionVariableSRW`, and
  imports `Sleep` for polling backoff).
- **ms→timespec arithmetic precedent:** `emit_thread_deadline`
  (`runtime_helpers_thread.rs:11-61`) converts a timeout-ms integer to a
  `timespec` (`sec = ms/1000`, `nsec = (ms%1000)*1e6`). `nanosleep` needs the
  same split but *relative* (skip the `clock_gettime` add).
- **Per-target imports / advertised calls:** each target enumerates its thread
  calls and libc imports — macOS `macos_aarch64/{mod.rs:140-151, plan.rs:615-651}`,
  Linux `linux_common/{mod.rs:198, plan.rs:423}` + `linux_aarch64/plan.rs:194` +
  `linux_x86_64/plan.rs:246`, Windows `win_x86_64/mod.rs:146-161`.
- **Man pages:** `src/docs/man/builtins/thread/` — 13 `.md` files today
  (`ls …/*.md | wc -l → 13`), one per function plus `package.md`.
- **Tests:** syntax `tests/syntax/threads/` (30 dirs today), runtime behavior
  `tests/rt-behavior/threads/`, runtime errors `tests/rt-error/threads/`,
  byte-identity `tests/byte-identity/thread/`, descriptor unit tests inline in
  `src/builtins/thread.rs:599-1026`.

### Measured populations

| What | Count | Command |
|---|---|---|
| User-facing sleep/delay in builtins | 0 | `rg -l -i 'sleep\|delay\|nanosleep' src/builtins/ → 0` |
| `nanosleep` references in `src/` | 0 (adding the first) | `rg -c nanosleep src/ → 0` |
| Thread man pages | 13 | `ls src/docs/man/builtins/thread/*.md \| wc -l → 13` |
| Thread runtime specs | 17 | `rg -c RuntimeHelperSpec …/thread_specs.rs → 17` |
| Syntax thread test dirs | 30 | `ls -d tests/syntax/threads/*/ \| wc -l → 30` |

### Verified properties

- **A no-handle-dependent thread helper is a proven shape.**
  `lower_thread_stdin_subscription_helper` (`runtime_helpers.rs:501-532`) runs on
  the calling thread and returns an `Ok` result without dereferencing a queue —
  read; it is the template for the sleep helper's frame/return contract.
- **The ms→timespec arithmetic already exists and is correct.**
  `emit_thread_deadline` (`runtime_helpers_thread.rs:39-57`) computes
  `sec = ms/1000` and `nsec = (ms%1000)*1e6` with nsec carry — read; the sleep
  helper reuses this split for the *relative* `nanosleep` timespec.
- **Windows already imports `Sleep`.** `emit_windows_thread_call` uses
  `SleepConditionVariableSRW`; `Sleep` is imported at `win_x86_64/plan.rs:233`
  (per exploration) — so the Windows `nanosleep`→`Sleep(ms)` arm needs no new
  import beyond confirming `Sleep` is in the IAT.

## 3. Design Overview

Three independent, layered pieces:

1. **Front-end (descriptor).** Register the name `thread.sleep` with a single
   overload `(Thread OF Msg TO Out, Integer) → Nothing`, mirroring how
   `thread::poll` declares its parent-handle-first, Integer-second signature
   (`thread.rs` P-array + `tf(...)` + the four hand-authored arms + the coverage
   test). Zero codegen risk; ships behind the descriptor unit tests.

2. **Parent runtime helper (`thread.sleep`).** A new
   `lower_thread_sleep_helper` emitting: load `ms` (arg 1); if `ms < 0` return
   `ErrInvalidArgument`; if `ms == 0` return `Ok`; else split `ms` into a
   *relative* `{sec, nsec}` timespec on the stack and call libc `nanosleep(&req,
   &rem)` in an EINTR-retry loop (on `-1`/EINTR copy `rem`→`req` and re-call so a
   signal cannot truncate the sleep); then return `Ok`. For handle-state
   consistency with `poll`, load the handle's state word
   (`THREAD_OFFSET_STATE = 0`) and return `ErrResourceClosed` (77030004) if it is
   `THREAD_STATE_CLOSED` (`"2"`) — see Open Decision. On Windows, `nanosleep`
   routes through a new `emit_windows_thread_call` arm mapping to `Sleep(ms)`
   (Win32 `Sleep` is millisecond-based, matching the API directly), so no libc
   `nanosleep` import is needed there.

3. **Target wiring + docs.** Register `THREAD_SLEEP_SPEC` (returns `Nothing`),
   add it to the catalog, add the dispatch arm, advertise `thread.sleep` on every
   target and import libc `nanosleep` (macOS/Linux only). Then the man page and
   spec.

**Where correctness risk concentrates:** piece 2 — the emitted `nanosleep`
timespec math and the EINTR loop, and the per-arch libc-import/relocation wiring
(x86-64 SysV vs macOS AArch64 vs Windows). Schedule it as its own phase behind an
rt-behavior test that measures elapsed wall-clock.

**Byte-identity is NOT this plan's core gate.** This plan adds new behavior; the
correctness gate is an rt-behavior test that observes `thread::sleep` actually
consuming ≥ `ms` of wall-clock plus the rt-error test for `ms < 0`. Byte-identity
is used only defensively: the `.ncode` of programs that do *not* call
`thread::sleep` must be unchanged (Phase 3 fixture). A diff there is a bug to
root-cause (objdump one fixture), not a design signal.

**Rejected alternatives:**
- *Build the parent sleep on `pthread_cond_timedwait` like the worker form* —
  rejected: the parent form has no queue/condvar to block on and no cancel
  semantics; `nanosleep` is the direct, dependency-free primitive. (The worker
  form in plan-91-B genuinely needs the condvar; the two are deliberately
  different helpers.)
- *No-handle `thread::sleep(ms)` single overload* — rejected per the confirmed
  design decision to keep the package handle-first (two overloads across
  plan-91-A/B).
- *`busy-wait` / `sched_yield` spin* — rejected: burns a core; `nanosleep` parks.

## 4. Detailed Design — parent helper `lower_thread_sleep_helper`

Emit (model the frame/return contract on
`lower_thread_stdin_subscription_helper`, `runtime_helpers.rs:501-532`):

```
entry:
  ; args: c_arg(0) = handle ptr (Thread), c_arg(1) = ms (Integer)
  ; --- handle-state check (Open Decision — parent poll parity) ---
  load_u64  %v9, [c_arg(0) + THREAD_OFFSET_STATE(0)]
  compare   %v9, THREAD_STATE_CLOSED("2")
  branch_eq  err_closed
  ; --- ms validation ---
  move      %vMS, c_arg(1)
  compare   %vMS, 0
  branch_lt  err_arg           ; ms < 0 → ErrInvalidArgument
  branch_eq  ok                ; ms == 0 → no-op Ok
  ; --- ms → relative timespec {sec, nsec} on stack (reuse emit_thread_deadline math, no clock_gettime add) ---
  sec  = %vMS / 1000
  nsec = (%vMS % 1000) * 1_000_000
  store sec  -> [sp + req+0]
  store nsec -> [sp + req+8]
retry:
  lea  c_arg(0), [sp + req]    ; &req
  lea  c_arg(1), [sp + rem]    ; &rem
  emit_thread_external_call("nanosleep")   ; POSIX; Windows → Sleep(ms) arm
  compare return_register, 0
  branch_eq  ok                ; 0 → completed
  ; -1 with EINTR: copy rem->req and retry (signal must not truncate)
  <copy rem->req>; branch retry
ok:
  move RESULT_TAG_REGISTER, RESULT_OK_TAG
  return
err_arg:
  <set ErrInvalidArgument in RESULT_*>; return
err_closed:
  <set ErrResourceClosed in RESULT_*>; return
```

Finalize with `finalize_vreg_body_with_locals` reserving stack for the two
`timespec` structs (`req`, `rem`, 16 bytes each). Windows: the `nanosleep` arm in
`emit_windows_thread_call` moves `ms` into the `Sleep` DWORD arg and calls
`Sleep`; the EINTR loop is inert there (`Sleep` does not return EINTR), so the
Windows path collapses to a single `Sleep(ms)`.

Error-code sourcing: reuse the same error-table/`RESULT_*` mechanism the sibling
helpers use for `ErrInvalidArgument` (77050002) and `ErrResourceClosed`
(77030004) — grep how `poll`/`simple_thread_handle_helper` set these rather than
hardcoding.

## Compatibility / Format Impact

- **Adds** one user-facing name `thread::sleep` (parent overload only) and one
  runtime symbol `_mfb_rt_thread_thread_sleep`.
- **Adds** a libc `nanosleep` import on macOS/Linux targets and one Win32
  `Sleep` translation arm.
- **Unchanged:** every existing `thread::` signature/semantics, the thread
  control block layout, and the `.ncode` of any program not calling
  `thread::sleep`.

## Phases

> Tick `- [x]` in the same commit as the work; fill each `Commit:` when it lands.

### Phase 1 — Descriptor (parent overload, no codegen)

Register the name and parent signature; prove it type-checks and that a
worker-side call and a bad-arity/negative-arg call are rejected.

- [x] `src/builtins/thread.rs`: add `const SLEEP: &str = "thread.sleep";`; add
      `P_SLEEP` param array `[req("t", Thread-handle), req("ms", "Integer")]`
      modeled on `thread::poll`'s parent-handle-first params; add
      `tf(SLEEP, "sleep", &[ov(P_SLEEP)])` to `THREAD_FUNCTIONS`.
- [x] Add `thread.sleep` arms to `resolve_call` (→ `"Nothing"`, parent handle
      only), `call_param_names`, and `expected_arguments`.
- [x] Update `is_thread_call_covers_every_name` to include the new name.
- [x] Tests: `tests/syntax/threads/func_thread_sleep_valid` (parent handle + ms
      + ms=0), `func_thread_sleep_worker_invalid` (ThreadWorker handle rejected —
      this case flips to valid in plan-91-B), `func_thread_sleep_invalid`
      (missing/extra args + wrong-type-ms). Added inline descriptor unit tests
      (`resolve_sleep_parent_only`, plus SLEEP in the three coverage tests).

Acceptance: `cargo test --bin mfb builtins::thread` green (24 passed) AND the
three new syntax tests pass via `test-accept.sh` (parent-valid resolves to
`Nothing`; worker call and mistyped `ms` are compile errors).
Commit: c3002407b

### Phase 2 — Parent runtime helper + target wiring (largest blast radius)

Emit the `nanosleep`-based helper and wire every target; prove it actually
sleeps and rejects `ms < 0`.

- [x] `src/target/shared/runtime/thread_specs.rs`: add `THREAD_SLEEP_SPEC`
      (`call: "thread.sleep"`, `returns: "Nothing"`), modeled on
      `THREAD_CANCEL_SPEC`.
- [x] `src/target/shared/runtime/catalog.rs`: add `THREAD_SLEEP_SPEC` to the
      supported-specs list (after `THREAD_POLL_SPEC`).
- [x] `src/target/shared/code/mod.rs`: route `"thread.sleep"` to
      `lower_thread_helper` (added to the `thread.*` match).
- [x] `src/target/shared/code/runtime_helpers.rs`: add the `"thread.sleep" =>
      lower_thread_sleep_helper(...)` dispatch arm and implement
      `lower_thread_sleep_helper` per §4 (ms validation, relative timespec, EINTR
      loop, `ErrResourceClosed` on closed handle).
- [x] `emit_windows_thread_call` (`runtime_helpers.rs`): add a
      `"nanosleep" => Sleep(dwMilliseconds)` arm (converts the timespec to ms);
      added `Sleep` to the shared Windows thread import set.
- [x] Advertise `thread.sleep` and import libc `nanosleep` per target: macOS
      `macos_aarch64/{mod.rs,plan.rs}` (dedicated `thread.sleep` arm importing
      only `_nanosleep`), Linux `linux_common/{mod.rs,plan.rs}` (dedicated arm,
      `nanosleep` via `libc_import`), Windows `win_x86_64/mod.rs` (advertise) +
      `plan.rs` (`Sleep`). Added `thread.sleep` to `thread_runtime_return_type`
      (`builder_value_semantics.rs`) → `Nothing` (needed by the eval-call lowering).
      `linux_aarch64/plan.rs` / `linux_x86_64/plan.rs` needed no change (nanosleep
      is libc, not libpthread).
- [x] Tests: `tests/rt-behavior/threads/thread-sleep-parent-rt` — starts a worker
      (`waitForCancelForever`), `thread::sleep(t, 50)`, asserts observed elapsed
      ≥ 40 ms (loose bound; prints only PASS/FAIL) plus a `ms = 0` no-op case.
      `tests/rt-error/threads/thread-sleep-negative-rt` — `thread::sleep(t, -1)`
      aborts with `ErrInvalidArgument` (7-705-0002, exit 255).

Acceptance: the rt-behavior sleep test passes on the native host target (observed
"slept ok", elapsed ≥ 40 ms) AND the rt-error negative-ms test passes
(ErrInvalidArgument) AND full `cargo test` is green (0 failures). Cross-compiled
`-ncode` cleanly for linux-x86_64/linux-aarch64/windows-x86_64 (nanosleep on
Linux, Sleep on Windows).
Commit: 2aacef7eb

### Phase 3 — Docs, spec, byte-identity

- [x] `src/docs/man/builtins/thread/sleep.md`: authored per `.ai/man_template.md`
      (Synopsis shows the parent overload; Errors table lists
      `ErrInvalidArgument`, `ErrResourceClosed`; notes `ms=0` is a no-op and the
      overload is uninterruptible). Omitted the Overloads section (single overload
      in 91-A; 91-B adds the worker form). Added a `thread::sleep` paragraph +
      error-row mentions to `package.md` (the package page is prose, not a list).
- [x] Spec: `src/docs/spec/threading/06_thread-runtime-helpers.md` (helper symbol
      + parent-only note + a `thread::sleep` section) and
      `src/docs/spec/language/16_threads.md` (signature + ms-convention prose);
      also `language/18_builtin-functions.md`'s thread call list.
- [x] Byte-identity: added `thread::sleep(t1, 0)` to the existing
      `tests/byte-identity/thread/` coverage fixture (which drives every overload)
      and regenerated all 4 `.ncodesum` targets + `.ir`/`.ast`/build.log. Non-sleep
      programs stay byte-identical (the nanosleep import is gated to `thread.sleep`);
      the scoped `artifact-gate.sh … thread` reports 0 diffs.

Acceptance: `mfb man thread sleep` renders; man-coverage (261) + spec (42) +
citations (2) tests green; scoped artifact-gate for `thread` = 0 diffs; full
`cargo test` green (0 failures).
Commit: 45df734df

## Validation Plan

- Tests: syntax (parent-valid, worker-invalid, bad-ms), rt-behavior (elapsed ≥
  ms; ms=0 no-op), rt-error (ms<0 → ErrInvalidArgument), byte-identity
  (unrelated programs unchanged; sleep program pinned), inline descriptor unit
  tests.
- Coverage check: the rt-behavior test must exercise the emitted helper on the
  host target — confirm the new test dir is picked up by the runtime suite (it
  runs the compiled program), so a green gate means the sleep code actually ran.
- Runtime proof: a standalone program `IMPORT thread` … `thread::sleep(t, 200)`
  wrapped between two `time`-observations shows ≥ ~200 ms wall-clock on
  macOS/Linux, and (manually, if a Windows runner is available) `Sleep`-based
  timing on Windows.
- Doc sync: `sleep.md` + `package.md` + spec threading/16_threads updated;
  man-coverage and spec-sync gates green.
- Acceptance: full `cargo test` (unit + syntax + rt-behavior + rt-error +
  byte-identity + acceptance goldens); rustfmt/clippy per `.ai/build-tooling.md`.

## Open Decisions

> RESOLVED during execution: (1) the handle-state check is DONE — the helper
> returns `ErrResourceClosed` on a closed handle (poll parity); (2) `sleep.md`
> documents ONLY the parent overload (plan-91-B revises it for the worker form).

- **Parent-side handle-state check.** Recommended: parent `thread::sleep` returns
  `ErrResourceClosed` on an already-closed `Thread` handle, matching `poll`'s
  documented behavior (§4). Alternative: ignore handle state entirely (the sleep
  needs nothing from the handle) — simpler helper, but inconsistent with the
  package's parent-handle conventions. Lean: do the check.
- **Man page for the worker overload.** Recommended: `sleep.md` in plan-91-A
  documents only the parent overload (matches shipped behavior); plan-91-B
  revises it to add the worker form + cancellation section. Alternative:
  pre-document both in A — rejected as documenting unshipped behavior.

## Corrections

- **Phase 2 needed an extra wiring site the plan's Current State did not list.**
  `builder_value_semantics.rs::thread_runtime_return_type` hand-resolves each
  thread call's return type for the *code layer* (separate from the front-end
  `resolve_call`). Without a `thread.sleep => Nothing` arm there, codegen failed
  with `native runtime call 'thread.sleep' has no return type while lowering eval
  call thread.sleep` (evidence: `mfb build tests/rt-behavior/threads/thread-sleep-parent-rt`).
  Added `thread.sleep` to that function's `Nothing`-returning arm. This is the
  code-layer twin of the front-end return-type table, and any future thread call
  must update both.
- **`_nanosleep` import gated to `thread.sleep` only.** The plan said "add
  `_nanosleep` to the libSystem import block" (the shared thread arm). Adding it
  there would import nanosleep for *every* thread program and churn thread
  byte-identity. Instead each target got a dedicated `thread.sleep` import arm
  (macOS `_nanosleep`, Linux `nanosleep` via `libc_import`), so a program that
  never calls `thread::sleep` is unaffected — matching §Non-goals' "no `.ncode`
  change for programs not calling thread::sleep".
- **`linux_aarch64/plan.rs` / `linux_x86_64/plan.rs` needed no change.** The plan
  listed them as import sites; nanosleep lives in libc (not libpthread), so the
  shared `linux_common` `libc_import` covers both arches. Those files' only
  `thread.*` references are unit tests over `thread.start`.

## Summary

The engineering risk is entirely in Phase 2: the emitted `nanosleep` timespec
arithmetic, the EINTR-retry loop, and the per-arch libc-import/relocation wiring
(macOS AArch64 libSystem, Linux SysV, Windows `Sleep`). Phase 1 is a
zero-codegen descriptor change behind unit tests; Phase 3 is docs + a defensive
byte-identity pin. Untouched: the thread control block layout, every existing
`thread::` function, and the worker-side/cancellation machinery (owned by
plan-91-B).
