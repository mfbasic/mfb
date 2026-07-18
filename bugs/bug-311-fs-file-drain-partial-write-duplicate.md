# bug-311: buffered `fs::File` drain does not persist partial-write progress on error → a retried flush duplicates the already-written prefix

Last updated: 2026-07-17
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (data duplication)

Status: Open
Regression Test: tests/rt-behavior (new) — a short-write then hard-error on a buffered File, then retry, does not duplicate bytes

`lower_fs_file_drain` advances only local cursor/remaining vregs per partial write
and zeroes `BUF_FILLED` only after the whole buffer lands. The error path returns 1
without writing back any progress — `BUF_PTR` (fixed base) and `BUF_FILLED` (full
original count) are left unchanged. So after a partial write lands `k > 0` bytes and
the next `write` hard-errors, the File record still claims the full buffer starting
at the base. A later `fs::flush`/overflow-drain re-issues `write` from byte 0,
duplicating the `k` already-written bytes. This is the cross-call re-send bug-97
item-1 described and bug-208 fixed for the stdout twin (`lower_stdout_drain`), but
the file drain never received that fix. (Unlike bug-208 there is no OOB — `BUF_PTR`
is never advanced — so the failure mode is pure data duplication, not a heap
overrun.)

The single correct behavior a fix produces: after a partial write followed by a hard
error, a retried drain resumes from the unflushed tail — the already-written prefix
is never re-sent.

References:

- `bugs/completed-bugs/bug-208-*` (fixed the stdout drain's err-path progress
  persistence), `bug-97` item-1 (named the stdout drain), `bug-62` (drain 0-spin +
  EINTR, no cross-call persist).
- Found during goal-06 review of `src/target/shared/code/fs_helpers_io.rs`.

## Failing Reproduction

```
' fs::setBuffered(f, TRUE) on a File whose fd short-writes (pipe/FIFO by path, or a
' filling disk); drain (flush/close/overflow); first write lands N>0, next write
' hard-errors (ENOSPC); free space; retry fs::flush/writeAll.
```

- Observed: the first N bytes appear twice in the file.
- Expected: the file contains each byte once; the retry resumes from the tail.

(Confirmed by code inspection; the twin path bug-208 fixed had the same mechanism.)

## Root Cause

`src/target/shared/code/fs_helpers_io.rs:253-266` (`lower_fs_file_drain`; header
claim at `:197-198`): the err path (`:263-266`) returns 1 without sliding the
unflushed tail to `BUF_PTR` or updating `BUF_FILLED`, so the record still describes
the full original buffer.

## Goal

- On the genuine-error exit, slide the unflushed tail down to `BUF_PTR` and store
  `BUF_FILLED = remaining` (keeping `BUF_PTR` as the fixed base) before returning 1,
  mirroring bug-208's stdout fix.

### Non-goals (must NOT change)

- The success-path behavior (whole-buffer drain).
- The EINTR/0-spin handling (bug-62).

## Blast Radius

- `lower_fs_file_drain` err path — fixed here.
- `emit_append_to_file_buffer` (which appends at `BUF_PTR+BUF_FILLED` and re-drains)
  — benefits; verify it reads the updated `BUF_FILLED`.
- The stdout twin already has the fix (bug-208) — unaffected.

## Fix Design

Copy bug-208's stdout err-path pattern: on the error exit, memmove the
`%v2`-remaining tail to `BUF_PTR` and store the new `BUF_FILLED`. Rejected
alternative: advancing `BUF_PTR` per write — introduces the OOB class bug-208
avoided; keep `BUF_PTR` as a fixed base.

## Phases

### Phase 1 — failing test
- [ ] rt-behavior test with a short-writing then failing fd, asserting no duplicate
      bytes after retry.
### Phase 2 — the fix
- [ ] Persist the tail on the err path.
### Phase 3 — validation
- [ ] Full suite green; buffered file writes to normal files unaffected.

## Validation Plan

- Regression: the short-write-then-error-then-retry test (a pipe/FIFO fixture).
- Runtime proof: no byte duplication.
- Doc sync: none.

## Summary

The file drain's error path leaves the buffer describing already-written bytes, so a
retry re-sends them; mirroring the stdout drain's bug-208 tail-slide fixes it. Risk
is matching the stdout persistence logic exactly.
