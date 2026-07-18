# bug-314: runtime-helper robustness LOW cluster (io::pollInput EINTR, net accept-after-poll race, term::sync short-write desync, stdin EINTR errno)

Last updated: 2026-07-17
Effort: small (<1h across items)
Severity: LOW
Class: Correctness

Status: Open
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
