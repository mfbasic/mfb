# plan-90-B: `process` package — streaming I/O

Last updated: 2026-08-08
Effort: large (3h–1d)
Depends on: [[plan-90-A-process-core-spawn]] — if A is not complete, this
sub-plan cannot start, full stop. A creates the `process` package, the `Process`
resource, and the three pipes (stdin-write / stdout-read / stderr-read fds)
stored in the record tail; B only reads/writes those fds. (Prerequisites table
lives in sub-plan A.)

This sub-plan adds line- and chunk-oriented I/O to a spawned `Process`, plus the
`Stream` enum that selects stdout vs. stderr for reads. A correct implementation
lets a program `send` a line to a child, `receive` a line back, `poll` for
readiness with a timeout, and drain the child's final buffered output after the
child has exited (no truncation).

References:

- `src/target/shared/code/net/io.rs` and `net/poll.rs` — the read/write/poll
  emission precedent (EINTR retry, timeout handling) this package mirrors.
- `src/builtins/app_package.mfb:24` — `EXPORT ENUM` declaration pattern for
  `Stream`.
- `./mfb man net accept` — Errors-section conventions for a resource I/O call.

## 1. Goal

- `process::Stream = { StdOut, StdErr }` (an `EXPORT ENUM` in the source
  companion).
- Working on all four Unix backends:
  - `process::send(p, text AS String) AS Nothing` — writes `text` **plus a
    trailing `\n`** to the child's stdin.
  - `process::send(p, text AS String, timeoutMs AS Integer) AS Nothing`
  - `process::sendBytes(p, data AS List OF Byte) AS Nothing` — writes the raw
    bytes, no newline added.
  - `process::sendBytes(p, data AS List OF Byte, timeoutMs AS Integer) AS Nothing`
  - `process::receive(p) AS String` — reads one `\n`-terminated line from stdout.
  - `process::receive(p, from AS Stream) AS String`
  - `process::receiveBytes(p) AS List OF Byte` — reads the next available chunk
    from stdout.
  - `process::receiveBytes(p, from AS Stream) AS List OF Byte`
  - `process::poll(p, ms AS Integer) AS Boolean` — stdout readable within `ms`.
  - `process::poll(p, ms AS Integer, from AS Stream) AS Boolean`
- **Drain-before-close semantics:** `receive`/`receiveBytes` return any bytes
  still buffered from the child even after the child has exited; they only raise
  `ErrResourceClosed` once the relevant pipe is at EOF **and** its buffer is
  drained. `poll` returns `true` at EOF so the caller can do a final draining
  read.

### Non-goals (explicit constraints)

- No new lifecycle/spawn behavior — the pipes already exist from A.
- No signals (`Signal` enum, `signal`/`didSignal`) — sub-plan C.
- No Windows — sub-plan D.
- No merged stdout+stderr stream and no stdio redirection to a file — out of
  scope for the whole `plan-90` feature.
- No layout/ABI/existing-golden change.

## 2. Current State

- After sub-plan A, a `Process` record's type tail holds three fds
  (stdin-write, stdout-read, stderr-read) plus the cached exit-state; A's backend
  is `src/target/shared/code/process/{mod,unix}.rs`.
- **Read/write/poll emission precedent** is `net/io.rs` (blocking read/write with
  `EINTR` retry — `net/mod.rs:134`) and `net/poll.rs` (fd-level readiness with a
  millisecond timeout). `process` I/O is the same shape over the stored fds.
- **A read buffer is needed** for line framing: `receive` must split on `\n`
  across read boundaries, so each read fd needs a small per-Process staging
  buffer (an out-of-line allocation the record tail points at). Precedent for
  per-resource I/O buffers is File's buffered I/O (`resource_uses_io_buffers`,
  `builder_resource_cleanup.rs:12`) — Process must opt into buffer cleanup the
  same way.
- **`Stream` enum**: package enums are declared in the source companion `.mfb`
  as `EXPORT ENUM` (discriminant = ordinal) — `app_package.mfb:24`,
  `datetime_package.mfb:109`.

### Measured populations

| What | Count | Command |
|---|---|---|
| Functions added by this sub-plan | 5 names (10 overloads) | send(×2), sendBytes(×2), receive(×2), receiveBytes(×2), poll(×2) |
| Unix backends to wire | 4 | as A: macos_aarch64, linux_{x86_64,aarch64,riscv64} |
| Read fds per Process needing a line buffer | 2 | stdout, stderr |

### Verified properties

- **The record tail has room for two read-buffer pointers** — VERIFIED against
  A's layout (§4.3 of A leaves 64 bytes of tail; 3 fds + exit-state + 2 buffer
  ptrs fit). Re-confirm the exact tail offsets against A's landed
  `process/unix.rs` before Phase 2.
- **Drain-on-EOF is not automatic** — UNVERIFIED that a naive `read()==0` path
  returns the buffered remainder first; this is the sub-plan's core correctness
  risk and gets a dedicated test.

## 3. Design Overview

Three pieces:

1. **`Stream` enum** in the source companion — the cheapest, no-codegen piece.
2. **Write path** (`send`/`sendBytes` + timeouts) — write to the stdin-write fd
   with an optional `timeoutMs` (via a non-blocking write + `poll` for
   writability, mirroring `net`'s timeout handling). `send` appends `\n`;
   `sendBytes` does not.
3. **Read path** (`receive`/`receiveBytes`/`poll` + `Stream` selection) — the
   risky piece: a per-fd staging buffer, `\n` line framing for `receive`,
   chunk return for `receiveBytes`, and **drain-before-EOF** so late output is
   never truncated.

**Where correctness risk concentrates:** the read path's drain-on-close and line
framing across read boundaries, and the classic **pipe deadlock** (a large
`send` while the child fills its stdout and the caller isn't reading blocks both
sides) — mitigated by the `timeoutMs` overloads and documented in the man pages.
Both land last, behind tests.

**Byte-identity is NOT the gate** (new runtime behavior). Validation is runtime:
round-trip bytes/lines through a child, and a specific late-output-drain test.

**Rejected alternatives:**

- *No staging buffer (read exactly one line per `read`).* Rejected: a pipe read
  returns arbitrary chunk boundaries; line framing requires buffering the
  remainder between calls.
- *`receive` errors immediately on child-exit.* Rejected — that is the
  truncation bug the drain semantics exist to prevent.

## 4. Detailed Design

### 4.1 `Stream` enum

Add to `src/builtins/process_package.mfb`:
`EXPORT ENUM Stream / StdOut / StdErr / END ENUM` with the `DOC/ENUM/PROP` doc
header (copy `app_package.mfb:17`). `StdOut`=0, `StdErr`=1 select the
stdout-read / stderr-read fd in the read builtins.

### 4.2 Write path

- Frontend metadata in `src/builtins/process.rs`: add send/sendBytes overloads
  (arity 2 and 3), return `Nothing`.
- `process/unix.rs`: `send` copies the String bytes then a `\n` to the
  stdin-write fd; `sendBytes` copies the `List OF Byte` raw. The `timeoutMs`
  overload sets the fd non-blocking, `poll`s for writability up to `timeoutMs`,
  and raises `ErrTimeout` (`ERR_TIMEOUT_*`) if not writable; `EPIPE`/closed
  stdin → `ErrResourceClosed`. `runtime/process_specs.rs` gains the helpers.

### 4.3 Read path

- Frontend metadata: receive/receiveBytes (arity 1 and 2, return `String` /
  `List OF Byte`), poll (arity 2 and 3, return `Boolean`); the `from AS Stream`
  overload selects the fd.
- Per-fd staging buffer: allocate a small heap buffer per read fd on first read,
  pointer stored in the record tail; freed in `__drop` (opt Process into
  `resource_uses_io_buffers`, `builder_resource_cleanup.rs:12`).
- `receive`: return bytes up to and including the next `\n` from the buffer,
  refilling via `read()` as needed; on `read()==0` (EOF) return the buffered
  remainder (even without a trailing `\n`); once the buffer is empty **and** EOF,
  raise `ErrResourceClosed`.
- `receiveBytes`: return the next non-empty chunk (buffered remainder first, then
  one `read()`); same EOF/closed rule.
- `poll`: `poll()`/`select()` the selected read fd for readability up to `ms`;
  return `true` if readable **or** at EOF (so a draining `receive` can follow),
  `false` on timeout.

## Compatibility / Format Impact

- Adds `Stream` to the package surface and two per-Process read-buffer
  allocations (runtime state, not a user-visible layout change). No existing
  golden change.

## Phases

### Phase 1 — `Stream` enum + write path (`send`/`sendBytes` + timeouts)

Delivers stdin writing; safe alone (reads not required to write).

- [ ] `Stream` enum in `process_package.mfb` + doc header.
- [ ] Frontend metadata for send/sendBytes overloads.
- [ ] `process/unix.rs` write emission + timeout (non-blocking write + poll) +
  `runtime/process_specs.rs` helpers; register any new libc imports (`write`,
  `fcntl`, `poll`) across the 4 Unix backends.
- [ ] Tests: `tests/rt_process_send_receive.rs` (send a line to `cat`, later
  read it back — the receive half is stubbed until Phase 2; assert the write
  succeeds and the child echoes when its stdin closes), `func_process_send*_invalid`.

Acceptance (runtime): a program spawns `cat`, `send`s a line, `close`s stdin, and
`waitFor` returns 0; a `send` with a 1ms timeout to a full pipe raises
`ErrTimeout`. `cargo test` green.
Commit: —

### Phase 2 — Read path (`receive`/`receiveBytes`/`poll` + `Stream` + drain)

Highest risk (line framing, drain-on-EOF); lands last behind tests.

- [ ] Frontend metadata for receive/receiveBytes/poll overloads.
- [ ] `process/unix.rs`: per-fd staging buffer, `\n` framing, chunk read,
  drain-before-close, poll-with-timeout; buffer cleanup in `__drop`.
- [ ] Tests: `tests/rt_process_receive_line.rs` (spawn `echo -e "a\nb"`, receive
  "a", "b"), `tests/rt_process_receive_drain.rs` (child writes then exits
  immediately → receive still returns the bytes, THEN `ErrResourceClosed`),
  `tests/rt_process_receive_stderr.rs` (`from := Stream.StdErr`),
  `tests/rt_process_poll_timeout.rs`.

Acceptance (runtime): full round-trip through `cat` (send→receive echoes), stderr
selection reads the child's stderr, late output is drained not truncated, poll
returns false on timeout / true at EOF. `cargo test` green on macOS + Linux
x86_64/aarch64; rv64 remote proof folded into sub-plan E.
Commit: —

## Validation Plan

- Tests: the `rt_process_*` I/O tests above + `func_process_*` invalid fixtures.
- Coverage check: the `rt_` binaries actually spawn a child and exchange bytes.
- Runtime proof: send/receive round-trip + drain test as above.
- Doc sync: man pages `src/docs/man/builtins/process/{send,sendBytes,receive,
  receiveBytes,poll}.md` + a `Stream` note on the package/types page; Errors
  sections cite `ErrTimeout`/`ErrResourceClosed`; `cargo test man_citations_resolve`.
- Acceptance: `scripts/test-accept.sh … 'process*'`; full artifact-gate deferred
  to sub-plan E.

## Open Decisions

- **D1 — deadlock mitigation surface.** Recommend documenting the pipe-deadlock
  hazard in the man pages and relying on the `timeoutMs` overloads (no auto
  reader thread) vs. spawning a background drain thread. Recommend
  document-and-timeout (simpler, matches `net`).
- **D2 — `receiveBytes` chunk size.** Recommend returning whatever a single
  `read()` yields (up to the buffer size) vs. a fixed chunk. Recommend
  single-read (lowest latency, caller loops).

## Corrections

<filled during execution>

## Summary

Risk is the read path: line framing across read boundaries and drain-on-EOF
(pinned by `rt_process_receive_drain`), plus the documented pipe-deadlock hazard
mitigated by timeout overloads. The write path and the `Stream` enum are
straightforward. No layout/ABI/existing-golden change.
