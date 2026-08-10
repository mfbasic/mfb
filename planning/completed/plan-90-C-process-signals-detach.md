# plan-90-C: `process` package — signals & detach

Last updated: 2026-08-08
Effort: medium (1h–2h)
Depends on: [[plan-90-A-process-core-spawn]] — if A is not complete, this
sub-plan cannot start, full stop. A provides the `Process` resource, the cached
exit/signal state (`waitFor` already decodes `WIFSIGNALED`), and the `__drop`
reap path this sub-plan reuses. B is NOT required. (Prerequisites in sub-plan A.)

This sub-plan adds signal delivery and inspection plus `detach`. A correct
implementation lets a program send a `Signal` to a child, read back which
`Signal` bucket a terminated child died on, and `detach` a child so it keeps
running after the resource is released — **without leaving a zombie**.

References:

- `src/builtins/app_package.mfb:24` — `EXPORT ENUM` pattern for `Signal`.
- sub-plan A §4.3 — the cached exit/raw-status decode `didSignal` reads.
- The `didSignal`/`signal` mapping tables in the feature brief (POSIX rows).

## 1. Goal

- `process::Signal = { None, Kill, Terminate, Error }` (`EXPORT ENUM`).
- Working on all four Unix backends:
  - `process::signal(p, sig AS Signal) AS Nothing` — deliver a signal:
    `Kill`→`SIGKILL`, `Terminate`→`SIGTERM`, `Error`→`SIGABRT`, `None`→no-op.
  - `process::didSignal(p AS Process) AS Signal` — for a **terminated** child,
    the bucket its terminating signal maps to (POSIX table below); `None` if it
    did not die on a signal (exited normally) **or** has not terminated yet.
  - `process::detach(p AS Process) AS Nothing` — relinquish ownership: the child
    keeps running, the resource is closed/unavailable, and no zombie is left
    (POSIX reaping handled per §4.3).
- **`waitFor` returns `-1` on signal** (already from A); `didSignal` is how the
  caller learns which bucket.

### POSIX `didSignal` bucket mapping (observe)

| Terminating signal | Signal bucket |
|---|---|
| SIGKILL | `Kill` |
| SIGHUP, SIGINT, SIGQUIT, SIGPIPE, SIGALRM, SIGTERM, SIGUSR1, SIGUSR2, SIGVTALRM, SIGPROF | `Terminate` |
| SIGILL, SIGABRT, SIGFPE, SIGSEGV, SIGBUS, SIGXCPU, SIGXFSZ, SIGSYS | `Error` |
| (no signal / exited normally / still running) | `None` |

### POSIX `signal` mapping (send)

`None`→no-op · `Kill`→`SIGKILL` · `Terminate`→`SIGTERM` · `Error`→`SIGABRT`.

### Non-goals (explicit constraints)

- **The 4-bucket `Signal` is the whole sendable/observable vocabulary.** The
  feature brief's alternate note ("only supports SIGINT/SIGHUP/SIGQUIT/…") is
  **dropped** — the enum cannot express those distinctly and `Terminate`→SIGTERM
  is the single "ask it to stop" action. This resolves the brief's
  self-contradiction; see Open Decisions D1.
- No Windows (`didSignal`/`signal` Windows mapping) — sub-plan D.
- No I/O — sub-plan B.
- No layout/ABI/existing-golden change.

## 2. Current State

- A's `waitFor`/`isRunning` already `waitpid` and cache the **raw status** plus a
  decoded exit code (`-1` on `WIFSIGNALED`) into the record tail (A §4.3).
  `didSignal` reads that cached raw status and maps `WTERMSIG` → a bucket; it
  requires no new syscall, only the mapping.
- A's `__drop` already does SIGKILL+waitpid. `detach` is the *opposite* policy:
  do **not** kill, but still avoid a zombie.
- **Zombie avoidance for a detached child** is the one genuinely new mechanism.
  On POSIX a child whose parent neither `waitpid`s nor ignores SIGCHLD becomes a
  zombie. Options: (a) `signal(SIGCHLD, SIG_IGN)` process-wide before/at spawn so
  the kernel auto-reaps any un-waited child; (b) a double-fork at spawn so
  detached children are reparented to init. See D2.
- `Signal` enum declaration follows the source-companion `EXPORT ENUM` pattern
  (`app_package.mfb:24`).
- `resource.rs` `BUILTIN_RESOURCES` `sendable` flag: `detach` closing the
  resource interacts with scope-drop; confirm the `sendable`/close-function
  choice A recorded still holds once `detach` exists.

### Measured populations

| What | Count | Command |
|---|---|---|
| Functions added by this sub-plan | 3 | signal, didSignal, detach |
| Unix backends to wire | 4 | as A |
| POSIX signals in the observe mapping | 19 | rows in the feature brief's POSIX (didSignal) table |

### Verified properties

- **`didSignal` needs no new syscall** — VERIFIED against A §4.3 (raw status is
  cached); re-confirm the exact cached-status offset in A's landed
  `process/unix.rs` before Phase 1.
- **Which zombie-avoidance mechanism A's spawn already permits** — UNVERIFIED;
  decide D2 by reading A's landed spawn path (does it already set SIGCHLD
  disposition?). This is the sub-plan's one real design fork.

## 3. Design Overview

Three pieces, cheapest first:

1. **`Signal` enum** — source companion, no codegen.
2. **`signal` + `didSignal`** — `signal` maps the bucket to a `kill(pid, …)`;
   `didSignal` maps the cached `WTERMSIG` to a bucket. Both are thin.
3. **`detach`** — relinquish without kill + zombie avoidance (the one new
   mechanism); mark the resource closed so later ops raise `ErrResourceClosed`
   and scope-drop does nothing.

**Where risk concentrates:** `detach`'s zombie avoidance (D2) — a detached child
must not become a zombie, and the chosen mechanism must not break A's `__drop`
reap for *non*-detached children. Lands last, behind a test that spawns, detaches,
and confirms via `waitpid`/`ps` that the child survives and no zombie accrues.

**Byte-identity is NOT the gate** — new runtime behavior; validation is runtime
(deliver a signal and observe the bucket; detach and observe survival+no-zombie).

**Rejected alternatives:**

- *Expose raw POSIX signal numbers.* Rejected: contradicts the 4-bucket product
  design and the brief's cross-platform intent; Windows can't honor arbitrary
  POSIX signals anyway (sub-plan D).
- *`detach` = leave the child fully unmanaged (accept zombies).* Rejected:
  violates the "drop must reap" spirit; detach must still avoid zombies.

## 4. Detailed Design

### 4.1 `Signal` enum

`EXPORT ENUM Signal / None / Kill / Terminate / Error / END ENUM` in
`process_package.mfb` with a `DOC/ENUM/PROP` header. Ordinals: None=0, Kill=1,
Terminate=2, Error=3.

### 4.2 `signal` + `didSignal`

- Frontend metadata in `process.rs`: `signal` (arity 2, `Nothing`), `didSignal`
  (arity 1, returns `Signal`).
- `process/unix.rs`: `signal` switches on the enum ordinal → `kill(pid, SIGKILL
  | SIGTERM | SIGABRT)`, `None`→return. Operating on a dropped/detached process →
  `ErrResourceClosed`.
- `didSignal`: if the cached exit-state shows the child hasn't terminated or
  exited normally → `None`; else map cached `WTERMSIG` via the §1 table to the
  bucket ordinal, return it. No syscall.

### 4.3 `detach`

- Frontend metadata: `detach` (arity 1, `Nothing`).
- `process/unix.rs`: close the record's `closed` bit so subsequent ops raise
  `ErrResourceClosed` and scope-drop's `__drop` becomes a no-op (do NOT kill).
  Close the retained pipe fds (parent side) so no fd leaks. Zombie avoidance per
  D2: recommended (a) — ensure `SIGCHLD` is set to `SIG_IGN` (or `SA_NOCLDWAIT`)
  once at first spawn so the kernel auto-reaps ANY child the runtime never
  `waitpid`s, including detached ones. Confirm this does not defeat A's explicit
  `waitpid` in `waitFor`/`__drop` (with `SIG_IGN`, `waitpid` may return `-1
  ECHILD` after auto-reap — A's `waitFor` must treat `ECHILD` as "already
  reaped, return cached/last-known code", not an error).

## Compatibility / Format Impact

- Adds `Signal` to the package surface. If D2 chooses the `SIG_IGN` approach, the
  runtime sets a process-wide SIGCHLD disposition at first spawn — a runtime
  behavior, not a user-visible layout change. `waitFor`'s `ECHILD` handling must
  be updated in A's code (a Correction to A, cross-referenced here). No existing
  golden change.

## Phases

### Phase 1 — `Signal` enum + `signal` + `didSignal`

Delivers signal delivery and inspection; safe alone (no detach/zombie change).

- [x] `Signal` enum in `process_package.mfb` (`None`/`Kill`/`Terminate`/`Error`)
  + doc header.
- [x] Frontend metadata for `signal`/`didSignal` (+ `detach`, landed together).
- [x] `process/unix.rs`: `signal` maps the bucket ordinal → `kill(pid, 9/15/6)`
  (`None`→no-op); `didSignal` maps the cached `WTERMSIG` (status@64) → a bucket
  (`None` if not-reaped/exited-normally; `Kill`=9; `Error`=4/6/8/10/11;
  `Terminate`=else). `runtime/process_specs.rs` helpers; `kill` reused, `signal`
  imported across the Unix backends.
- [x] Tests: `rt-behavior/process/signal` (sleeper + `signal(Signal.Terminate)`
  → `waitFor`==-1, `didSignal`==`Terminate`; `true` → 0/`None`). Enum values are
  compared, not `toString`'d (`toString` has no enum overload). New-layout, NOT
  `tests/rt_*.rs`.
  — FIX (Correction): `ir/lower.rs` lacked `process::augmented_project`, so
  companion enum values (`Signal.*`, `Stream.*`) failed at native codegen ("NIR
  local reference does not resolve"). Added; re-synced all process `.ir`/`.ast`.

Acceptance (runtime): VERIFIED on macOS + Linux x86_64 — `signal(Terminate)` →
`didSignal()==Terminate`, `waitFor()==-1`; a normal exit → `None`. `cargo test`
green (3797).
Commit: a40e645bf

### Phase 2 — `detach` + zombie avoidance

The one new mechanism; lands last behind a survival+no-zombie test.

- [x] D2 decided: **(a) `SIG_IGN` for SIGCHLD**, set inside `detach` (NOT at
  spawn — so non-detached children keep normal `waitpid` status retrieval). The
  kernel then auto-reaps the un-waited detached child, no zombie. A already
  handles `ECHILD` (`waitFor`/`isRunning`/`__drop` branch `<0` → treat as reaped),
  so no A change was needed. SIGCHLD is platform-specific (macOS 20 / Linux 17),
  picked via `platform.family()`.
- [x] `process/unix.rs`: `detach` closes the retained parent-side fds, `SIG_IGN`s
  SIGCHLD, and sets the record `closed` bit (scope-drop `__drop` no-ops; later ops
  trap `ErrResourceClosed`). No kill.
- [x] Tests: `rt-behavior/process/detach` (spawn `sleep 30`, detach → program
  exits in ~0.15s, child SURVIVES, no zombie) + `detach-then-use` (any op after
  detach → `ErrResourceClosed`, exit 255).

Acceptance (runtime): VERIFIED on macOS + Linux x86_64 — a detached child
survives the drop, leaves no zombie, and a subsequent `pid` TRAPs
`ErrResourceClosed`; non-detached children still reap (drop-reap fixture green).
`cargo test` green (3797).
Commit: a40e645bf

## Validation Plan

- Tests: the `rt_process_signal_*`/`detach*` tests above + invalid fixtures.
- Coverage check: the `rt_` binaries actually deliver signals / detach and
  inspect the outcome.
- Runtime proof: signal→didSignal round-trip; detach survival + no-zombie.
- Doc sync: man pages `src/docs/man/builtins/process/{signal,didSignal,detach}.md`
  + the `Signal` enum on the types page, including the POSIX bucket table;
  `cargo test man_citations_resolve`.
- Acceptance: `scripts/test-accept.sh … 'process*'`; full artifact-gate in E.

## Open Decisions

- **D1 — `Signal.None` as a sendable value.** Recommend keeping it a documented
  no-op (uniform enum for send + observe) vs. rejecting `signal(p, None)` as an
  arg error. Recommend no-op (simplest; `None` is meaningful for `didSignal`).
- **D2 — detached-child zombie avoidance.** Recommend **(a) process-wide
  `SIG_IGN`/`SA_NOCLDWAIT` for SIGCHLD set once at first spawn** (kernel
  auto-reaps; simplest) with A's `waitFor` treating `ECHILD` as already-reaped,
  vs. **(b) double-fork at spawn** (reparent to init; more code, keeps explicit
  `waitpid` clean). Recommend (a); decide by reading A's landed spawn path.

## Corrections

<filled during execution — note especially any change pushed back into
plan-90-A's `waitFor` for `ECHILD` handling under D2(a).>

## Summary

`signal`/`didSignal` are thin maps over machinery A already built; the real work
is `detach`'s zombie avoidance (D2), which must coexist with A's explicit reap.
The brief's send/observe `Signal` contradiction is resolved to a single 4-bucket
enum. No layout/ABI/existing-golden change.
