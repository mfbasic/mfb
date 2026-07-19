# bug-314: runtime-helper robustness LOW cluster (io::pollInput EINTR, net accept-after-poll race, term::sync short-write desync, stdin EINTR errno)

Last updated: 2026-07-17
Effort: small (<1h across items)
Severity: LOW
Class: Correctness

Status: Partially fixed — H1/H3/H4 landed; H2 attempted twice and reverted (see below)
Regression Test: per-item

LOW-severity runtime-helper robustness residuals found during goal-06, all around
signal/short-write handling on interruptible or non-seekable handles. Distinct root
causes, one document per the repo's low-cluster convention. Each is latent for the
common case and bites only under signal delivery or a racing peer.

References:

- Found during goal-06 review of `src/target/shared/code/{io_helpers,net/io,term_grid,stdin_broadcast}.rs`.
- `bugs/completed-bugs/bug-62` (read/write/seek EINTR), `bug-115` (net poll EINTR),
  `bug-185` (net accept timeout), `bug-208` (stdout drain persistence).

## Items

### H1 — `io::pollInput` does not retry `EINTR` → a signal surfaces as a spurious `ErrInput`
- `src/target/shared/code/io_helpers.rs:730-738` (`lower_io_poll_input_helper`).
- After `emit_poll_input` the code does `compare 0 / branch_lt poll_error` with no
  EINTR distinction, and the backend `emit_poll_input` is a bare `bl _poll` with no
  internal retry. Every read/write/seek loop got a uniform EINTR retry (bug-62) and
  net poll got one (bug-115), but `io::pollInput`'s fd-0 poll was left unwrapped — a
  handled signal (SIGWINCH in a TUI, SIGCHLD, the console SIGINT/SIGTERM handler where
  the program continues) interrupting a blocked/timed `io::pollInput()` returns
  `ErrInput` instead of ready/not-ready.
- Fix: on the negative return, branch back to the poll on `EINTR` (reuse
  `emit_eintr_retry_or_error`); a strict re-poll should recompute the remaining
  timeout.

### H2 — bounded `net::accept` can block past its deadline after poll signals readiness
- `src/target/shared/code/net/io.rs:92, 109-120` (`lower_net_accept_helper`).
- The bug-185 bounded wait polls `POLLIN` then, on `poll > 0`, issues a *blocking*
  `accept(fd, NULL, NULL)` (the listener fd is never set non-blocking). If the single
  pending connection is aborted (RST/`ECONNABORTED`) or consumed by another thread in
  the window between poll and accept, `accept` blocks until the next connection,
  ignoring `timeoutMs`.
- Fix: set the listener non-blocking around the `accept`, and on `EAGAIN/EWOULDBLOCK`
  re-enter the poll loop against the remaining deadline; restore blocking mode after
  (as the connect path already does).

### H3 — `term::sync` ignores the present `write` result after already syncing the front buffer → permanent desync on short/interrupted write
- `src/target/shared/code/term_grid.rs:1035-1038` (back→front copy) + `:1077-1082`
  (write result discarded).
- The diff loop copies each emitted cell back→front *before* the single
  `write(1, outbuf, len)`, whose result is discarded (no short-write/EINTR retry). If
  that write is short or EINTR after a partial transfer (a large repaint interrupted by
  a signal), the terminal shows only part of the frame, but `front == back`, so the
  next `term::sync` diffs to nothing and never repairs the missing cells — permanent
  corruption, not a dropped frame.
- Fix: loop the present write until all bytes flush (retry EINTR, advance on short
  counts), or copy back→front only after a fully-successful write.

### H4 — stdin next-byte EINTR classification reads errno after an intervening `pthread_mutex_lock`
- `src/target/shared/code/stdin_broadcast.rs:407-431` (read → relock → classify) +
  `:487-496`.
- On a `read` returning -1 the result is saved, then `pthread_mutex_lock` runs and
  reader-busy is cleared before `read_neg` fetches errno via `__errno_location`. errno
  is only meaningful if the lock preserved it; glibc/musl (raw futex) and macOS
  preserve it in practice, so it is correct today, but the code relies on that unstated
  guarantee — any lock path that touched errno would misclassify a read error as EINTR
  (silent infinite retry) or vice-versa. Latent.
- Fix: capture errno immediately after `read` (before re-locking) into a stack
  slot/vreg and classify from that saved value.

## Goal

- Signal/short-write handling is robust: pollInput retries EINTR, accept honors its
  deadline, term::sync repairs partial writes, stdin classifies EINTR from a
  guaranteed-fresh errno.

### Non-goals (must NOT change)

- The common-case fast paths.
- The existing EINTR loops (bug-62/115) that are already correct.

## Blast Radius

Each item is a single cited helper site; land per item.

## Fix Design / Phases

- [ ] Phase 1: tests where constructible (H1 via a signal during pollInput; H2 via a
      reset-before-accept fixture); H3/H4 are reasoned/latent.
- [ ] Phase 2: apply per-item fixes.
- [ ] Phase 3: full suite green; TUI/stdin behavior unaffected.

## Validation Plan

- Regression: signal-during-pollInput and accept-timeout tests where the harness
  supports them.
- Doc sync: none.

## Summary

Four runtime-helper robustness residuals around signals and short writes; each is a
small EINTR/short-write guard. Latent for the common case; value is TUI/network
robustness under signals before MVP.

## Resolution — three of four; H2 deliberately not landed

**H1 — `io::pollInput` now retries EINTR.** A negative poll return goes through
`emit_eintr_retry_or_error` and re-enters at `os_poll`, which re-arms the pollfd from
scratch. This is the same treatment read/write/seek got in bug-62 and net poll in
bug-115; fd-0 poll was simply the one left unwrapped. Like those, the retry re-polls
with the original timeout rather than the remaining one.

**H3 — `term::sync` loops the present write to completion.** This was the most
damaging of the four, and worth stating precisely: the diff copies each emitted cell
back→front *before* the write, so a short or interrupted write left the terminal
showing a partial frame while `front == back` asserted it was fully painted. The next
`term::sync` then diffed to nothing and never repaired it — permanent corruption, not
a dropped frame. The write now advances on short counts and stops on a non-positive
return (the bug-62 lesson: a 0-byte return for a nonzero-length write must not spin).
Looping was chosen over "copy back→front only after success" because the streaming
diff does not retain the emitted-cell list to replay.

**H4 — errno is captured immediately after the `read`,** before the intervening
`pthread_mutex_lock`, and the EINTR classification reads that saved value. The old
code was correct in practice — glibc, musl and macOS all preserve errno across the
lock — but it rested on an unstated guarantee about someone else's implementation,
and the failure mode (misclassifying a real read error as EINTR) is a silent infinite
retry.

### H2 — attempted twice, reverted both times, and NOT shipped

The fix is what the report describes: put the listener in non-blocking mode around
the bounded accept, re-enter the poll on EAGAIN, restore the flags afterwards. Both
attempts produced a **strictly worse defect than the one being fixed**, on the very
first test — a bounded accept with no client, which must report `ErrTimeout`:

1. **Attempt one returned success for a timed-out accept.** All four exits converge
   on `done`, which is what makes a single flag-restore attractive — but the result
   and tag registers are already set by then, and the restoring `fcntl` is a call
   that destroys them. The timeout's error tag was overwritten with fcntl's return
   value.
2. **Attempt two segfaulted.** Spilling and reloading the result registers around
   that call, and widening the frame to hold them, crashed instead.

The bug being fixed is LOW severity and requires a connection to be aborted or stolen
in the window between poll and accept. Trading that for "a bounded accept sometimes
reports success when nothing connected" — or for a crash — is not a trade worth
making, and a third unverified attempt would be reckless. Reverted; `net::accept`
verified back to reporting `77050008` on a bounded wait with no client.

What the next attempt should know:

- The listener fd needs its own slot: the success path overwrites `FD_OFFSET` with
  the *accepted* socket's fd before `done` is reached.
- The restore must not clobber `RESULT_TAG_REGISTER` / `RESULT_VALUE_REGISTER`, and
  spilling them around the call was not sufficient on its own.
- Restoring per-exit (before each `emit_fail` and before the success return) avoids
  the register-preservation problem entirely, at the cost of four call sites — that
  is probably the shape to try next.
- `net.accept` must gain `fcntl` in `plan::net_libc_symbols`, or the helper fails to
  link with `runtime helper requires _fcntl import`.

Full `cargo test` green; artifact gate 0 diffs; acceptance 1013/1013.

## Resolution — H1, H3, H4 landed; H2 reverted and re-filed

**H1 — `io::pollInput` retries EINTR.** A negative poll return goes through
`emit_eintr_retry_or_error` and re-enters at `os_poll`, which re-arms the pollfd.
Same treatment read/write/seek got in bug-62 and net poll in bug-115; fd-0 poll was
the one left unwrapped. Like those, it re-polls with the original timeout, not the
remaining one.

**H3 — `term::sync` loops the present write to completion.** The most damaging of the
four: the diff copies each emitted cell back->front *before* the write, so a short or
interrupted write left the terminal partially painted while `front == back` claimed
otherwise. The next sync then diffed to nothing and never repaired it -- permanent
corruption, not a dropped frame. The write now advances on short counts and stops on
a non-positive return (bug-62's lesson: a 0-byte return for a nonzero write must not
spin). Looping beats "copy back->front only after success" because the streaming diff
does not retain the emitted-cell list to replay.

**H4 — errno is captured immediately after the `read`,** before the intervening
`pthread_mutex_lock`, and the EINTR check reads that saved value. The old code was
correct in practice (glibc/musl/macOS all preserve errno across the lock) but rested
on an unstated guarantee about someone else's implementation, and the failure mode --
misclassifying a real read error as EINTR -- is a silent infinite retry.

**H2 is NOT fixed.** Two attempts each produced a strictly worse defect than the race
they fix, caught by the first test run (a bounded accept with no client, which must
report `ErrTimeout`): the first returned *success* for a timed-out accept, the second
segfaulted. Reverted; `net::accept` verified back to `77050008`. Re-filed as
**bug-359** with the full diagnosis so the next attempt starts informed.

Full `cargo test` green; artifact gate 0 diffs.
