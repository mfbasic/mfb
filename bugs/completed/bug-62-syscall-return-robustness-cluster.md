# bug-62: fs/io syscall loops mishandle degenerate returns — `EINTR` is a hard error (no retry), a `write()`-returns-0 makes the drain loops spin forever, and `reconcile` ignores a failed rewind `lseek`

Last updated: 2026-07-09
Effort: small (<1h)

A cluster of LOW-severity robustness gaps in the fs/io runtime-helper syscall loops, all
in the class "the loop tests the raw syscall return without handling a degenerate case".
All are latent for regular files on local filesystems (which don't return `EINTR`, don't
return 0 from a positive-count `write`, and always seek successfully); they bite only
interruptible / non-seekable handles (FIFOs, sockets, ttys opened by path) and signal
delivery. MFBASIC installs SIGINT/SIGTERM handlers and can run threads, so signals mid-syscall
are reachable.

The single correct behavior a fix produces: `EINTR` is retried, a `write()` of 0 with
bytes pending is treated as an error (not a spin), and a failed rewind `lseek` surfaces an
error rather than silently dropping buffered bytes.

References (all under `src/target/shared/code/`):

- **`EINTR` treated as a hard error (no retry)** — every fs/io read/write/seek loop tests
  a negative return as failure without distinguishing `-EINTR`:
  `io_helpers.rs:45-46`, `:276-279` (writes), `:842-844`, `:1006-1008`, `:1407-1409`
  (reads); `fs_helpers_io.rs` write loops `:736`, `:1059`, read loops `:905`, `:1271`,
  seeks throughout; `fs_helpers_atomic.rs` write loop `:473-478`.
- **`write()`-returns-0 spin** — the drain loops advance by the (zero) count and re-test
  `remaining != 0`, looping forever: `io_helpers.rs:lower_stdout_drain` (`:41-48`, uses
  `branch_lt` so a 0 return is "progress"); `fs_helpers_io.rs:lower_fs_file_drain`
  (`:41-48`, same). Contrast: `lower_fs_write_all_helper` uses `branch_le` and cannot spin.
- **`reconcile` ignores the rewind `lseek`** — `io_helpers.rs:emit_reconcile_read_buffer`
  (`:1547-1553`): after `readLine` on a non-seekable handle, `lseek(fd, -(fill-pos),
  SEEK_CUR)` on `ESPIPE` fails but the code unconditionally zeroes READ_POS/READ_FILL/
  READ_AT_EOF, discarding unconsumed read-ahead and leaving the fd unmoved. Contrast:
  `readAll`/`eof` check every seek with `branch_lt(seek_error)`.
- Note: the *short-write* silent-truncation on these same single-`write` sites is filed as
  bug-51 (different failure mode — positive short count vs. EINTR/zero); the fixes are
  adjacent (both want a proper advance-and-loop with a `branch_le`/`EINTR` distinction).
- KNOWN: PTY echo race (`bug-pty-echo-race`). Related: the `EINTR` class is
  defense-in-depth like OS-08.
- Found during the goal-01 compiler source review of `src/target/shared/code/`.

## Failing Reproduction

- `EINTR`: run a program blocked in `io::readLine` on a FIFO and deliver SIGWINCH (or
  SIGCHLD from a worker thread) so the handler returns; the read fails `ErrInput` where a
  retry would have completed.
- Spin: a `write()` to a handle that returns 0 with bytes pending → `lower_stdout_drain` /
  `lower_fs_file_drain` livelock (process hangs, buffer never drains). Degenerate but
  unguarded.
- `reconcile`: `readLine` then `writeAll` on a FIFO (`ESPIPE` on the rewind) → the
  unconsumed read-ahead is silently dropped and the fd position is wrong.

- Observed: spurious `ErrInput`/`ErrOutput` on `EINTR`; infinite loop on `write`-0;
  silent data-position corruption on `reconcile`.
- Expected: retry on `EINTR`; error (not spin) on `write`-0; surface the seek failure.

Contrast: regular local files never hit these; `readAll`/`eof` check seeks;
`lower_fs_write_all_helper` uses `branch_le`.

## Root Cause

The loops compare the raw syscall return with `branch_lt`/`branch_le` but (a) do not
re-read `errno` to distinguish `EINTR`, (b) two drain loops use `branch_lt` so a 0 return
is treated as progress, and (c) `reconcile` performs the buffer-invalidation stores
unconditionally after a seek it never checks.

## Goal

- On a negative return, the loop re-reads `errno` and retries when it is `EINTR`.
- A `write()` returning 0 with bytes pending is an error, not a spin (`branch_le`).
- `reconcile` checks the rewind `lseek` and surfaces `ErrRead`/`ErrOutput` on failure
  instead of dropping buffered bytes.

### Non-goals (must NOT change)

- Regular-file behavior (unaffected today).
- The short-write positive-count handling (bug-51's scope) — coordinate but keep distinct.
- The genuine-error paths.

## Blast Radius

- Every `file:line` in References. Group the fix by mechanism: an `EINTR`-retry wrapper
  applied uniformly to the read/write/seek loops; `branch_le` on the two drain loops; a
  seek-result check in `reconcile`.

## Fix Design

- **`EINTR`:** on a negative syscall return, load `errno`; if `EINTR`, branch back to
  re-issue the same syscall; else take the error path. Apply uniformly.
- **Drain spin:** change the two drain loops' guard to `branch_le(&err)` so a 0 return
  errors.
- **`reconcile`:** add `compare/branch_lt(seek_error)` on the `lseek` result before the
  buffer-invalidation stores.

## Phases

### Phase 1 — audit + tests

- [x] Enumerate all read/write/seek loops; add tests for the FIFO/`EINTR`, `write`-0
      (fault-injected), and `reconcile`-`ESPIPE` cases.

### Phase 2 — the fixes

- [x] `EINTR`-retry the loops; `branch_le` the drains; check `reconcile`'s seek.

### Phase 3 — validation

- [x] Regenerate goldens; `scripts/artifact-gate.sh`, `scripts/test-accept.sh`;
      re-run the FIFO/signal reproductions. (Goldens/acceptance are the orchestrator's
      to regenerate — the fs/io helper `.ncode` shifts on every backend; runtime
      reproductions re-run below via a `read`/`write`/`lseek` fault-injection dylib.)

## Validation Plan

- Regression test(s): FIFO+signal read completes; `write`-0 errors; `reconcile` seek
  failure surfaces.
- Runtime proof: a signal delivered mid-`readLine` does not spuriously fail.
- Doc sync: none expected.
- Full suite: `scripts/artifact-gate.sh`, `scripts/test-accept.sh`.

## Summary

The fs/io syscall loops don't retry `EINTR`, two drain loops spin on a 0-byte `write`, and
`reconcile` ignores a failed rewind seek. All latent for regular files but reachable on
interruptible/non-seekable handles with signals. The fixes are a uniform `EINTR`-retry, a
`branch_le` on the drains, and a seek-result check.

## Resolution

Fixed in `src/target/shared/code/fs_helpers_io.rs` and `src/target/shared/code/io_helpers.rs`
(shared helpers live in `fs_helpers_io.rs`, called from both). Composes with bug-51's
advance-and-retry loops — a write interrupted after a partial transfer resumes, never errors.

**Mechanism (all in the two shared files — no `plan.rs`/backend edits).**

- `emit_eintr_retry_or_error` is the shared negative-return tail. Two errno conventions,
  because only `linux-x86_64`'s `write` is a raw `svc` (all other write/read/seek on every
  backend, and `read`/`lseek` on x86, go through libc):
  - **libc** (`___error` / `__errno_location`, code left in `x9`): re-read `errno`, retry on
    `EINTR == 4`.
  - **raw** (`write_uses_raw_syscall` ⇒ `linux-x86_64` write): the return value *is* `-errno`,
    so `EINTR` is `ret + 4 == 0` — no accessor call, and it works even in a pure-`io::`
    program that never links the accessor.
- `EINTR == 4` on both Linux and macOS/BSD, so a single literal serves every backend.
- `errno_accessor_available` gates the libc path. The accessor is imported by the `fs::` helpers
  (a `File` only comes from `fs::openFile`) and by the `io::` read helpers — the
  `io.readByte`/`io.readChar`/`io.readLine`/`io.input` arms of all four `plan.rs` files now
  co-import it (`___error` on macOS, `__errno_location` on Linux), so io read `EINTR` retry is
  active in a pure-`io::` program that never touches `fs`/`net`. `io.input` already imported it;
  the follow-up added it to the `readLine`/`readChar`/`readByte` arm. The accessor is referenced
  (each read helper emits `bl <accessor>` in its guard) exactly when it is imported, so no backend
  gains a dead import. The only path that links no accessor is an output-drain-only program
  (`io.print`/`io.write`/`io.flush`, no read and no `fs`): there the libc-write negative return is
  a hard error (a drain-only `EINTR` is degenerate) and the `linux-x86_64` raw-`svc` write still
  retries via its `-errno` return.
- **Item 1 (EINTR):** `emit_transfer_loop_tail` wraps every bug-51 write/read loop
  (writeAll/writeAllBytes/readAll/readAllBytes, the buffered-append alloc-failed/big-write
  loops, io_write direct+newline, the readLine prompt, and both drains); `emit_single_op_eintr_guard`
  wraps the lead blocking read of `io::readByte`/`readChar`/`readLine` and the `fs::readLine`
  refill read (retry re-enters the read's setup). Seeks are intentionally **not** wrapped —
  `lseek` never blocks, so it cannot return `EINTR` (wrapping would be dead code).
- **Item 2 (`write()==0` spin):** the loop tail routes a 0-byte return to the error label with
  an equality branch (`branch_gt advance; branch_eq error`), replacing the drains' old
  `branch_lt` that treated 0 as progress and spun.
- **Item 3 (reconcile seek):** `emit_reconcile_read_buffer` gained a `seek_error_label` param
  and now `branch_lt`s on the rewind `lseek` result — `writeAll`/`writeAllBytes` surface
  `ErrOutput`, `readAll`/`readAllBytes` surface `ErrRead` — instead of unconditionally zeroing
  the read buffer and dropping unconsumed read-ahead.

**RISC-V gotcha:** RISC-V has no persistent condition flags — the MIR fuser welds each compare
to the single branch that immediately follows. The single-op read guard's `branch_ge resume`
consumes the caller's `cmp x0, 0`, so the guard re-issues `cmp x0, 0` at `resume` for the
caller's follow-on `branch_eq <eof>` to fuse with. (`x0` is untouched on the `>= 0` path.)

**Validation.** `cargo build` clean on all backends. New regression test
`tests/syscall_return_robustness.rs` locks the codegen structure of all three fixes across
macos-aarch64 / linux-aarch64 / linux-x86_64 / linux-riscv64 (4 tests, pass; verified they
FAIL when the EINTR guard is neutered). Runtime proof on macOS via a `DYLD_INSERT_LIBRARIES`
`read`/`write`/`lseek` interposer:
- Item 1 two-sided: injecting `EINTR` (4) on `read` ⇒ `fs::readLine` resumes, correct output,
  exit 0; injecting `EIO` (5) ⇒ correctly errors (`read failure`, exit 255) — the guard is
  EINTR-specific, not a blind retry.
- Item 1, pure-`io::` (the accessor-import follow-up): a program that only `IMPORT io` and calls
  `io::readLine` (no `fs`/`net`) now imports the accessor on all four backends (`-nplan`) and the
  `readLine`/`readByte`/`readChar` helpers each emit `bl <accessor>` in their guard (`-ncode`
  relocations, none dead). Runtime via the same `read` interposer: 1x/3x `EINTR` ⇒ retries, exit
  0; 1x `EIO` ⇒ `input failure`, exit 255. A print-only hello-world imports no accessor on any
  backend (`-nplan`), so no backend gained a dead import.
- Item 2: forcing `write()==0` ⇒ the drain errors (`output failure`, exit 255) within a 10 s
  watchdog, no spin.
- Item 3: forcing the reconcile `lseek(SEEK_CUR, <0)` to fail ⇒ `fs::writeAll` surfaces
  `output failure` (exit 255) instead of silently corrupting the fd position.

Goldens WILL shift for every program that emits these fs/io helpers (new EINTR/0-return/seek
instructions on all four backends); the orchestrator regenerates them.
