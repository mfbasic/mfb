# bug-359: bounded `net::accept` can block past its deadline when the pending connection vanishes

Last updated: 2026-07-19
Effort: medium (1h–2h)
Severity: LOW
Class: Correctness (deadline not honored)

Status: Open
Regression Test: tests/ (new) — a bounded `net::accept` returns `ErrTimeout` within its deadline even when a connection is aborted between poll and accept

Split out of bug-314 (item H2), which fixed its three siblings. This one was
attempted twice and reverted both times; the diagnosis below is the value of this
document.

The bug-185 bounded wait polls `POLLIN` on the listener and then issues a **blocking**
`accept(fd, NULL, NULL)` — the listener fd is never set non-blocking. If the single
pending connection is aborted (RST/`ECONNABORTED`) or consumed by another thread in
the window between the poll returning ready and the accept running, that accept blocks
until the *next* connection arrives, ignoring `timeoutMs` entirely.

The single correct behavior a fix produces: a bounded `net::accept` returns within its
deadline regardless of what happens to the pending connection after poll reports it.

References:

- `src/target/shared/code/net/io.rs` (`lower_net_accept_helper`).
- `bugs/completed-bugs/bug-185-*` (introduced the bounded wait), `bug-115` (accept
  EINTR retry).
- `bugs/completed-bugs/bug-314-*` (the parent cluster; H1/H3/H4 landed there).

## Root Cause

The listener stays in blocking mode, so `accept` has no way to report "the connection
I was told about is gone" — it just waits for another.

## Fix Design — and two failed attempts to learn from

The shape is: set the listener non-blocking for the bounded path, re-enter the poll on
`EAGAIN`, restore the original flags on exit. Both attempts broke a *basic* case on
the very first run — a bounded accept with no client at all, which must report
`ErrTimeout` (77050008):

1. **Attempt one returned success for a timed-out accept.** All four exits (success,
   accept failure, timeout, closed, allocation failure) converge on `done`, which
   makes a single flag-restore there look attractive. But the result and tag registers
   are already set by that point, and the restoring `fcntl` is a *call* that destroys
   them — the timeout's error tag was overwritten with fcntl's return value.
2. **Attempt two segfaulted.** Spilling `RESULT_TAG_REGISTER`/`RESULT_VALUE_REGISTER`
   to new frame slots around that call, and widening `FRAME_SIZE` from 64 to 80 to
   hold them, crashed instead.

What the next attempt should know:

- **Restore per-exit, not at `done`.** Emitting the restore before each `emit_fail`
  and before the success return avoids the register-preservation problem entirely, at
  the cost of four call sites. That is the shape to try.
- **The listener fd needs its own frame slot.** The success path overwrites
  `FD_OFFSET` with the *accepted* socket's fd before any convergence point, so a
  restore keyed on `FD_OFFSET` would set flags on the wrong descriptor.
- **`net.accept` must gain `fcntl`** in `plan::net_libc_symbols`, or the helper fails
  to link with `runtime helper requires _fcntl import`.
- `platform.o_nonblock()` and `platform.eagain()` already exist; the `connectTcp` path
  in `net/mod.rs` is a working model of the get-flags/set-nonblocking/restore sequence.
- Guard the restore so the unbounded path (`timeoutMs <= 0`, the deliberate
  block-forever overload) never goes non-blocking and never restores.

### Non-goals (must NOT change)

- The unbounded `net::accept(listener)` overload, which must keep blocking.
- The EINTR retry (bug-115).

## Blast Radius

`lower_net_accept_helper` only, plus the `net.accept` import list. Leaving a listener
non-blocking after the call would be a worse and far more visible bug than the race —
that is the failure mode to test for.

## Validation Plan

- A bounded accept with **no** client must report `ErrTimeout` within its deadline
  (this is what both failed attempts broke — run it first).
- A bounded accept with a client connecting must still succeed.
- An unbounded accept must still block.
- After any of the above, a second accept on the same listener must behave normally
  (proving the flags were restored).

## Summary

A real deadline violation, but one needing a racing peer, and two attempts have each
traded it for something worse. Worth fixing carefully with the per-exit restore, not
worth shipping unverified.
