# bug-310: `net::setReadTimeout`/`setWriteTimeout` misreads a failed `setsockopt` as success (missing sign-extension)

Last updated: 2026-07-17
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: tests/rt-error (new) — a failing setsockopt in set-timeout returns an error

After `emit_libc_call("setsockopt", …)`, `lower_net_set_timeout_helper` does
`compare_immediate(return_register(), "0")` then `branch_lt(&set_fail)` with **no**
`abi::sign_extend_word` first. `setsockopt` returns a C `int`; per AAPCS/SysV the
upper 32 bits of `x0`/`rax` are unspecified. A `-1` (error) return with clear upper
bits reads as `+4294967295`, so `branch_lt` is not taken and the failure falls
through to the success path — the caller believes the timeout is armed when it is
not, and a later blocking read/write never times out. Every other int-returning
libc call in the net layer sign-extends before its signed compare; this one site
(the bug-170 class) was missed.

The single correct behavior a fix produces: a failed `setsockopt` in the set-timeout
helper returns the error, exactly like the other net int-return checks.

References:

- `bugs/completed-bugs/bug-170-net-fs-libc-int-return-not-sign-extended.md` (fixed
  the class but did not include this `setsockopt` site).
- Found during goal-06 review of `src/target/shared/code/net/poll.rs`.

## Failing Reproduction

`net::setReadTimeout(sock, ms)` / `net::setWriteTimeout(sock, ms)` where `setsockopt`
fails (e.g. ENOTSOCK/EINVAL) on a backend that leaves `x0[63:32]` clear.

- Observed: returns Ok; a subsequent blocking read/write never times out.
- Expected: returns the error so the caller knows the timeout was not set.

(Confirmed by code inspection; the wrong-branch mechanism is deterministic when the
upper bits are clear.)

## Root Cause

`src/target/shared/code/net/poll.rs:212-221` (`lower_net_set_timeout_helper`): the
`setsockopt` return is compared as a 64-bit value without
`abi::sign_extend_word(return_register(), return_register())` first, unlike every
other net int-return (accept/socket/bind/listen/connect/getsockopt/both
polls/getpeername/getsockname).

## Goal

- Insert `abi::sign_extend_word` after the `setsockopt` call and before the
  `compare_immediate`.

### Non-goals (must NOT change)

- The other (already-correct) net int-return checks.
- The timeout value semantics.

## Blast Radius

- `net/poll.rs:lower_net_set_timeout_helper` — fixed here.
- All other net int-returns already sign-extend (verified) — unaffected.

## Fix Design

Add the single `sign_extend_word` before the compare, matching the sibling sites.
No alternative considered — this is the established pattern.

## Phases

### Phase 1 — failing test
- [ ] rt-error test forcing a setsockopt failure (e.g. on a non-socket fd) and
      asserting the error surfaces; confirm it returns Ok today where upper bits are
      clear.
### Phase 2 — the fix
- [ ] Insert the sign-extension.
### Phase 3 — validation
- [ ] Full suite green on all backends.

## Validation Plan

- Regression: the setsockopt-failure test.
- Doc sync: none.

## Summary

A single missed sign-extension in the set-timeout helper lets a failed setsockopt
read as success; adding it matches every sibling net int-return. Minimal, well-scoped.
