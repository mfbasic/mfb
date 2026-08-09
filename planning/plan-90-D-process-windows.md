# plan-90-D: `process` package — Windows backend

Last updated: 2026-08-08
Effort: large (3h–1d)
Depends on: [[plan-90-A-process-core-spawn]], [[plan-90-B-process-io]],
[[plan-90-C-process-signals-detach]] — if any of A/B/C is not complete, this
sub-plan cannot start, full stop. This sub-plan implements the SAME `process`
surface A/B/C defined, on Windows; it defines no new builtins. (Prerequisites in
sub-plan A.)

This sub-plan makes the `process` package work on `win_x86_64` via
`CreateProcess`, anonymous pipes, `WaitForSingleObject`/`GetExitCodeProcess`, and
`TerminateProcess`. A correct implementation lets the SAME `process` programs
from A/B/C run on Windows: spawn, exchange bytes, wait for the exit code, signal,
and drop-reap — verified by **execution on the Windows box**, not by byte
comparison.

References:

- `src/target/shared/code/audio/windows.rs` — the Windows native-backend
  precedent in this codebase (WASAPI); same `platform`-dispatch shape
  (`audio/mod.rs:118`).
- `src/target/win_x86_64/` — Win64 codegen backend (import registration site).
- memory `windows-byte-identity-is-a-nongoal` — Windows is verified by
  EXECUTION on box 2230, never by byte-identity; and `win64-shadow-space-and-entry-abi`.

## 1. Goal

- The `process` package (all of A's lifecycle, B's I/O, C's signals/detach)
  works on `win_x86_64` with equivalent observable behavior:
  - `spawn`/`shell` — `CreateProcess` (`shell` runs `powershell -Command` /
    `cmd /c` per the platform default shell) with redirected stdin/stdout/stderr
    anonymous pipes; spawn failure TRAPs `ErrSpawnFailed`.
  - `send`/`sendBytes`/`receive`/`receiveBytes`/`poll` — `WriteFile`/`ReadFile`
    over the pipe handles; `poll` via `PeekNamedPipe` / `WaitForSingleObject`
    with a timeout; same `\n`-framing and drain-before-close semantics as B.
  - `pid`/`isRunning`/`waitFor` — `GetProcessId` / `GetExitCodeProcess`
    (`STILL_ACTIVE`) / `WaitForSingleObject`+`GetExitCodeProcess`.
  - `signal` — `Kill`/`Error`→`TerminateProcess`; `Terminate`→
    `GenerateConsoleCtrlEvent(CTRL_C_EVENT)` when a console is attached, else
    `TerminateProcess`; `None`→no-op.
  - `didSignal` — map the exit code: an NTSTATUS-style exception exit code
    (`0xC0000005` access violation, `0xC0000094` int/0, `0x80000003`, etc.)
    → `Error`; otherwise `None`. **Ctrl-C / forced-terminate are not
    retroactively observable and map to `None`** (documented best-effort; see §4).
  - `detach` — `CloseHandle` on the process/pipe handles without terminating;
    the child keeps running (Windows has no zombies, so no reaper needed).
  - Drop policy: a live `Process` dropped out of scope → `TerminateProcess` +
    `CloseHandle`.

### Non-goals (explicit constraints)

- **No byte-identity on Windows** (memory `windows-byte-identity-is-a-nongoal`) —
  do not chase, gate, or report Win64 `.ncode` identity. Verify by execution.
- No new builtins, enums, or error codes beyond A/B/C.
- No change to the Unix backends' behavior.

## 2. Current State

- After A/B/C the `process` surface, `Process` resource, both enums, and all
  runtime-helper specs exist; only the **Windows arm of the native backend** is
  missing. A created `src/target/shared/code/process/{mod,unix}.rs`; the `mod.rs`
  dispatch reserves a `windows` arm (as `audio/mod.rs:118` does).
- **Windows native-backend precedent** is `audio/windows.rs` (WASAPI via the
  `platform` object). Win64 ABI hazards are known: 32-byte shadow space above
  `rsp`, entry `sp%16==8`, no negative immediates (memory
  `win64-shadow-space-and-entry-abi`).
- **Import registration** for Win32 calls happens in `src/target/win_x86_64/`
  (`code.rs`/`plan.rs`), the same site the other packages register their PE
  imports.
- **Windows has no `SIGCHLD`/zombie concept** — `detach` is just `CloseHandle`;
  drop is `TerminateProcess`+`CloseHandle`. The POSIX zombie machinery from C
  (D2) does not apply.

### Measured populations

| What | Count | Command |
|---|---|---|
| `process` builtins to implement on Windows | all 13 | the full A+B+C surface |
| Win32 APIs needed | ~10 | CreateProcess, CreatePipe, WriteFile, ReadFile, PeekNamedPipe, WaitForSingleObject, GetExitCodeProcess, TerminateProcess, GenerateConsoleCtrlEvent, CloseHandle |
| Windows codegen backends | 1 | `win_x86_64` |

### Verified properties

- **A's `process/mod.rs` has a reachable `windows` dispatch arm** — UNVERIFIED
  until A lands; confirm the dispatch shape matches `audio/mod.rs:118` before
  Phase 1 (if A left it absent, adding it is Phase 1's first task).
- **Exception exit codes are observable via `GetExitCodeProcess`** — VERIFIED by
  design (Windows surfaces `EXCEPTION_*` status as the process exit code); the
  `didSignal→Error` mapping keys off the high-bit NTSTATUS range.

## 3. Design Overview

Three pieces mirroring A→B→C, all in a new `src/target/shared/code/process/windows.rs`:

1. **Lifecycle** (A's surface): `CreateProcess` + 3 anonymous pipes, record
   alloc/stamp (same tag 10, same 96-byte envelope), `WaitForSingleObject`/
   `GetExitCodeProcess` caching, `TerminateProcess`+`CloseHandle` drop.
2. **I/O** (B's surface): `WriteFile`/`ReadFile` + `PeekNamedPipe` poll, reusing
   B's staging-buffer/line-framing/drain logic (platform-independent above the
   read/write primitive).
3. **Signals & detach** (C's surface): the `TerminateProcess`/`CTRL_C_EVENT`
   send map and the exit-code→bucket `didSignal` map; `detach` = `CloseHandle`.

**Where risk concentrates:** the `CreateProcess` handle-inheritance dance
(marking only the child ends of the pipes inheritable, closing the parent's copy
of the child ends) and the Win64 ABI hazards (shadow space, entry alignment).
Lifecycle lands first behind an on-box execution test.

**Byte-identity is explicitly NOT a gate here** (memory
`windows-byte-identity-is-a-nongoal`). Acceptance is: the A/B/C runtime programs,
recompiled for Windows, produce the same observable results when run on box 2230.

**Rejected alternatives:**

- *Promise POSIX-signal fidelity on `didSignal`.* Rejected: Ctrl-C /
  forced-terminate as a child's cause-of-death is not observable from the parent
  on Windows; only exception exit codes are. Map those to `Error`, everything
  else to `None`, and document the limit rather than fake it.

## 4. Detailed Design

### 4.1 Lifecycle (`CreateProcess` + pipes)

`CreatePipe` ×3 with a `SECURITY_ATTRIBUTES{bInheritHandle=TRUE}`; mark only the
child ends inheritable (`SetHandleInformation`), pass them in
`STARTUPINFO.hStd*`, `CreateProcess(..., bInheritHandles=TRUE, ...)`, then
`CloseHandle` the parent's copy of the child ends. Store {stdin-w, stdout-r,
stderr-r handles, process handle, pid, cached exit-state} in the record tail
(same offsets A defined; handles are 64-bit, they fit). `CreateProcess` failure →
`ErrSpawnFailed` (TRAP). `waitFor` = `WaitForSingleObject(INFINITE)` +
`GetExitCodeProcess`; `isRunning` = `GetExitCodeProcess`==`STILL_ACTIVE`; cache
the code.

### 4.2 I/O

`send`/`sendBytes` → `WriteFile` (with a `timeoutMs` via overlapped I/O or a
bounded `WaitForSingleObject`); `receive`/`receiveBytes` → `ReadFile` into B's
staging buffer, reusing B's line-framing + drain-on-`ERROR_BROKEN_PIPE` (EOF)
logic; `poll` → `PeekNamedPipe` for buffered bytes, or `WaitForSingleObject` on
the pipe up to `ms`, returning `true` at EOF.

### 4.3 Signals & detach

`signal`: `Kill`/`Error`→`TerminateProcess(1)`; `Terminate`→
`GenerateConsoleCtrlEvent(CTRL_C_EVENT, pid)` if the child shares a console, else
`TerminateProcess`; `None`→no-op. `didSignal`: read cached exit code; if it is in
the NTSTATUS exception range (high nibble `0xC`/`0x8`, e.g. `0xC0000005`) →
`Error`; else `None`. `detach`: `CloseHandle` process + pipe handles, mark the
resource closed; child keeps running.

### 4.4 Import registration

Register the ~10 Win32 imports in `src/target/win_x86_64/` `code.rs`/`plan.rs`,
alongside the existing package imports, respecting the shadow-space/entry-ABI
constraints.

## Compatibility / Format Impact

- Windows-only: adds the `process/windows.rs` backend and Win32 imports. Same tag
  10, same envelope, same package surface. No change to Unix behavior or to any
  existing golden.

## Phases

> **Prerequisites (A/B/C) MET.** Sub-plans A, B, C are complete and runtime-
> verified on macOS-aarch64 + Linux x86_64/aarch64/riscv64 (the whole `process`
> surface, ~30 rt-behavior fixtures). Box 2230 (Win11 x86_64) is reachable.
> **Platform-dispatch scaffold landed** (`019542919`): `process/mod.rs`'s
> `process_dispatch!` routes each helper to `windows::` on
> `PlatformFamily::Windows`, `unix::` elsewhere; `process/windows.rs` holds an
> unreachable "not yet emitted" arm per op (gated by the `win_x86_64` capability
> list, which advertises no `process.*` yet). The Win32 emission below replaces
> those arms and adds the capability entries + imports.

- [x] `process/windows.rs`: `CreateProcessA` + 3 anonymous pipes + record
  alloc/stamp; `WaitForSingleObject`/`GetExitCodeProcess` cache;
  `TerminateProcess`+`CloseHandle` drop; `ErrSpawnFailed` TRAP. **Emitted +
  runtime-verified on box 2230 (spawn/pid/waitFor/isRunning/close/drop).** Design: build a
  command line from the argv `List OF String` (space-joined, quote args with
  spaces); `CreatePipe` ×3 with `SECURITY_ATTRIBUTES{nLength=12, lpSD=0,
  bInheritHandle=1}`; `SetHandleInformation(parent-end, HANDLE_FLAG_INHERIT, 0)`;
  `STARTUPINFOA{cb=104, dwFlags=STARTF_USESTDHANDLES(0x100), hStdInput/Output/Error}`;
  `CreateProcessA(NULL, cmdline, NULL, NULL, TRUE, 0, NULL, NULL, &si, &pi)` — a
  10-arg call via `emit_external_int_call(platform, "CreateProcessA", from, 10, …)`
  (args 4–9 staged with `outgoing_stack_arg_store`, 32-byte shadow handled by the
  Win64 call emitter); `CloseHandle` the child ends + `pi.hThread`; store
  `pi.hProcess`@8 and the parent pipe handles in the tail (handles are 64-bit).
  `waitFor` = `WaitForSingleObject(hProcess, INFINITE)` + `GetExitCodeProcess`;
  `isRunning` = `GetExitCodeProcess`==`STILL_ACTIVE(259)`; `close` = `CloseHandle`
  the stdin write handle; `__drop` = `TerminateProcess(hProcess,1)` +
  `CloseHandle`. `pid` = `pi.dwProcessId` (cache in a tail slot at spawn).
- [x] Register the lifecycle Win32 imports (kernel32: `CreateProcessA`,
  `CreatePipe`, `SetHandleInformation`, `WaitForSingleObject`,
  `GetExitCodeProcess`, `TerminateProcess`, `CloseHandle`, `GetLastError` — plus
  `WriteFile`/`ReadFile`/`PeekNamedPipe` pre-registered for Phase 2) in
  `src/target/win_x86_64/plan.rs` + added spawn/pid/isRunning/waitFor/close/__drop
  to the `win_x86_64` capability list.
- [x] Tests: `cli_process_windows_build.rs` (emit-inspection on macOS: compiles
  the lifecycle program for `windows-x86_64`, asserts the Win32 imports; POSIX
  build must NOT import them) + on-box execution on 2230 of the lifecycle program
  (`["cmd","/c","exit 3"]` → `pid>0`=TRUE, `waitFor`=3, `isRunning`=FALSE; bogus
  path → TRAP `ErrSpawnFailed`).

Acceptance (execution on box 2230): the spawn/waitFor program, compiled for
Windows, prints the child's exit code and leaves no orphaned handle; a bogus path
TRAPs `ErrSpawnFailed`. **MET** — box 2230 prints `TRUE`/`3`/`FALSE`; a bogus
path TRAPped `ErrSpawnFailed` during bring-up.
Commit: 019542919 (scaffold) + a6ce3b4b1 (Win32 lifecycle emission)

### Phase 2 — Windows I/O (send/receive/poll)

- [x] `process/windows.rs`: `WriteFile` (send/sendBytes + trailing `\n`),
  `ReadFile` (receive byte-at-a-time line framing → validated String;
  receiveBytes one chunk → List OF Byte), `PeekNamedPipe` on a `GetTickCount64`
  deadline (poll); registered the I/O imports + advertised
  send/sendTimeout/sendBytes/sendBytesTimeout/receive/receiveFrom/receiveBytes/
  receiveBytesFrom/poll/pollFrom in the capability list.
- [x] Tests: `cli_process_windows_build.rs` compiles the I/O surface for
  `windows-x86_64` + on-box execution on 2230 of a `sort` round-trip
  (send→close→receive) and an `echo` poll+drain program.

Acceptance (execution): B's send→receive round-trip and drain-on-exit program
produce the same output on Windows as on Unix. **MET** — box 2230: a `sort`
round-trip printed `apple`/`banana`/`0`; a `cmd /c echo hello` poll+drain printed
`TRUE`/`7`/`0`.
Commit: c1361e4d5

### Phase 3 — Windows signals & detach

- [x] `process/windows.rs`: `signal` map (every terminating bucket →
  `TerminateProcess(128+signo)`, None = no-op — plan D2's best-effort, no
  `GenerateConsoleCtrlEvent` needed), exit-code→bucket `didSignal` (NTSTATUS
  severity-3 exit code → Error, else None), `detach` = `CloseHandle` the pipes +
  process handle + set the closed bit. Reuses the already-registered
  `TerminateProcess`/`CloseHandle` imports.
- [x] Tests: `cli_process_windows_build.rs` compiles the signal/didSignal/detach
  surface + on-box execution on 2230 (signal Kill; didSignal None/Error; detach).

Acceptance (execution): a faulting child reports `didSignal()==Error`; a normal
exit reports `None`; a detached child survives handle close. `signal(Kill)`
terminates the child. **MET** — box 2230 printed `137` (signal Kill exit),
`none` (normal exit), `error` (0xC0000005 exception exit), `detached`.
Commit: <this commit>

## Validation Plan

- Tests: `cli_process_windows_*_build.rs` compile-gates on macOS (emit-inspection
  where useful, memory `windows-codegen-emit-inspection-test`) + on-box execution
  of the A/B/C runtime programs.
- Coverage check: the on-box runs actually spawn/exchange/signal — not just
  compile.
- Runtime proof: execution on box 2230 for each phase (memory
  `windows-byte-identity-is-a-nongoal` — execution is the ONLY Windows gate).
- Doc sync: add Windows mapping tables + the `didSignal` best-effort note to
  `src/docs/man/builtins/process/{signal,didSignal}.md`.
- Acceptance: full artifact-gate in sub-plan E; Windows verified by execution
  here.

## Open Decisions

- **D1 — `shell` on Windows.** Recommend `cmd /c` (ubiquitous, fast startup) vs.
  `powershell -Command` (the brief's example). Recommend `cmd /c` as the default
  shell unless the box's default is PowerShell; document whichever.
- **D2 — `Terminate` when no console.** Recommend falling back to
  `TerminateProcess` when `GenerateConsoleCtrlEvent` can't apply (no shared
  console) vs. raising. Recommend silent fallback (best-effort, matches the
  4-bucket abstraction).

## Corrections

- **Win64 helper frame discipline (Phase 1).** The plan's `emit_external_int_call`
  / `outgoing_stack_arg_store` sketch for the 10-arg `CreateProcessA` did not
  survive contact with `finalize_frame`: a shared-code helper that mixes numeric
  `Vregs` (spilled by `finalize`) with hand-placed `sp+0x20..` outgoing stack args
  gets those args SHIFTED off the real slot the callee reads (garbage `lpSI`/`lpPI`
  → crash), and `emit_libc_call` does not reserve the 32-byte shadow. Fix: the
  Windows spawn/waitFor/isRunning/close/drop helpers are written fully-explicit —
  one `subtract_stack(FRAME)`/`add_stack(FRAME)` bracket (depth-1, so nothing is
  shifted), NO abstract vregs (so nothing spills), all state in `sp`-relative slots,
  `mfb_arg(0..3)` as transient scratch. This mirrors the proven fs
  `emit_build_argv_utf8` pattern. Recorded in memory `win64-helper-frame-and-zero-reg`.
- **`move_register(_, ZERO)` does not zero on x86-64 (Phase 1).** There is no
  hardware zero register; `ZERO` maps to a GPR holding garbage. CreateProcessA's
  NULL args came out as loop-leftover pointers → returned FALSE (`ErrSpawnFailed`).
  Fix: zero a register arg with `move_immediate(reg, "Integer", "0")` (only
  `store_*` special-cases `ZERO` to an immediate). Same memory note.
- **`process_synth` gate (Phase 1).** The Unix overload force-emit in
  `lower_module_for_platform` (spawnEnv / *Timeout / *From synthesized helpers)
  would emit the Windows *stub* bodies for any Windows program calling `spawn`,
  failing the build. Gated the force-emit blocks on
  `platform.family() != PlatformFamily::Windows`; the Windows backend emits its
  own overloads. (The overload symbols the NIR names directly — via
  `builder_values`' `runtime_target` — are still emitted; only the *force*-emit of
  un-called overloads is gated off.)
- **Man-page Windows notes deferred to E (Phase 3).** The Validation Plan's doc
  sync targets `src/docs/man/builtins/process/{signal,didSignal}.md`, but the
  `process` man directory does not exist yet — the man pages are authored in
  plan-90-E (finalization). The Windows mapping (`signal` → `TerminateProcess`,
  `didSignal` best-effort exception-only) and the `sendTimeout` best-effort note
  must be included when E writes those pages. plan-90-E already schedules the
  POSIX/Windows `signal`/`didSignal` mapping tables (its Phase 1 `types.md` task);
  the `sendTimeout` Windows note should ride along there.
- **`sendTimeout` on Windows is best-effort (Phase 2).** Unix `send(_, _, ms)`
  polls `POLLOUT` before each write; Windows anonymous pipes have no write-
  readiness object, so the Windows `sendTimeout`/`sendBytesTimeout` do a blocking
  `WriteFile`. For a draining reader (the tested case) it returns immediately; it
  does not preempt a genuinely full pipe with no reader. A documented Windows
  limit alongside `didSignal`.

## Summary

Windows re-implements the A/B/C surface over `CreateProcess`/pipes/Win32 wait
APIs; the risk is the pipe-handle inheritance dance and the Win64 ABI. Two
honesty limits are baked in: `didSignal` only recovers exception exit codes
(everything else `None`), and there is no byte-identity gate — Windows is proven
by execution on box 2230.
