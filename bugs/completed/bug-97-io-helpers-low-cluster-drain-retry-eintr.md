# bug-97 — io_helpers LOW cluster: stdout drain re-sends written prefix on retry; continuation-byte reads treat EINTR as failure

**Status:** OPEN. Filed 2026-07-10 (goal-02 review, G4). Two independent LOW
findings in `src/target/shared/code/io_helpers.rs`, batched per goal-02.

## 1. stdout drain re-sends the already-written prefix after a mid-drain failure

`io_helpers.rs:42-82` (`lower_stdout_drain`) — the drain loop advances a local
cursor (`%v2`/`%v1`) per partial write but only zeroes `ARENA_OUT_FILLED` after
the *whole* buffer lands; `ARENA_OUT_PTR`/`OUT_FILLED` are never advanced to
reflect partial progress, and the `err` path stores nothing. After a failure
following a successful partial write, the buffer state still claims the full
original contents. The header comment even documents "buffer left intact … so
a later flush can retry" without accounting for partial progress.

Trigger: buffered stdout (`io::setBuffered(TRUE)`) on a nearly-full
filesystem: first `write` lands N bytes, next returns ENOSPC → `io::flush`
traps `ErrOutput`; program frees space, calls `io::flush` again → the first N
bytes appear **twice** in the output.

Fix: on the error path, store the advanced pointer/remaining count back into
`ARENA_OUT_PTR`/`ARENA_OUT_FILLED` (memmove-down or offset-tracking) before
returning the error.

Prior art: bug-51/bug-62 fixed short-write looping and EINTR *within* one
drain call, not cross-call retry state.

## 2. UTF-8 continuation-byte reads treat EINTR as input failure

`io_helpers.rs:1160-1161` et al. (`lower_io_read_char_helper`,
`lower_io_read_line_helper`) — bug-62's `emit_single_op_eintr_guard` wraps only
the *lead* blocking 1-byte read; every follow-up read for bytes 2–4 of a
multi-byte sequence branches straight to `input_error` on any negative return
(`branch_lt(&input_error)`). A signal delivered while blocked mid-sequence
surfaces as a spurious `ErrInput`; in readLine it also discards the partial
line.

Trigger: `io::readChar()` in raw mode while a multi-byte character arrives
split over a slow connection; a handled signal (e.g. SIGINT with the runtime's
console handler, program continuing) interrupts the blocked continuation read
→ -1/EINTR → "input failure" instead of resuming.

Fix: apply the same EINTR guard to every continuation-byte read.

Prior art: adjacent to bug-62 (closed) but explicitly outside its landed
scope.
