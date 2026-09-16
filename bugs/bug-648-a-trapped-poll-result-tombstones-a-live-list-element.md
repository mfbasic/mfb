# bug-648: a trapped `tcp/udp/tls::poll` result tombstones the live list element it borrowed

Last updated: 2026-09-15
Effort: medium (1h–2h)
Severity: **HIGH** (a live, still-owned handle is flagged `moved|closed`)
Class: Correctness (resource lifetime)

Status: Open
Regression Test: none yet — see Phase 1

`tcp::poll`/`udp::poll`/`tls::poll` over a `List OF RES …` returns a **borrowed** pointer to
an element the list still owns (§15.6). Under an inline `TRAP`, that borrowed result is run
through `copy_resource_to_current_arena`, which unconditionally sets `moved|closed` on its
**source** — so the list's still-live socket is tombstoned while the list still holds it.

**The single correct behavior a fix produces:** polling a list under an inline `TRAP` leaves
every element open and owned by the list; the list's drop closes each exactly once, and no
element reports `ERR_RESOURCE_CLOSED` / `ERR_RESOURCE_MOVED` afterwards.

References:

- Found by bug-643's fix work (subagent report), reading
  `codegen/memory/arena/builder_arena_transfer.rs::copy_resource_to_current_arena`.
- `.ai/resources-packages.md`, "Thread transfer move-flag is success-gated" and "Borrowed-return
  + return-type-overloaded builtin wiring".
- bug-375 / §15.6: a borrowed return registers no close obligation.

## Failing Reproduction

**Not yet written.** `tests/rt-behavior/tcp/tcp-udp-poll-list-trap-rt` has the right SHAPE but
never reaches the defect: its polls always time out, so the success arm that makes the copy is
never taken. A reproduction must make `poll` actually **return an element** — e.g. a loopback
socket with data already pending — and then observe the element afterwards (poll it again,
send on it, or let the list drop and count closes).

Phase 1's first job is to produce that reproduction and confirm the tombstone; until then the
mechanism is read from the source, not measured.

## Root Cause

To confirm. `copy_resource_to_current_arena` flags the source record `moved|closed` after
copying it into the destination arena. For a thread hand-over that is correct — the sender
really has given the handle away. For a **borrowed** `poll` result it is not: the source is a
live element the list still owns and will close at its own drop.

bug-643 deliberately left this path on the copy rather than extending its pointer-carry to it:
carrying the pointer would hand the `TRAP` binding a close obligation on an element the list
also closes — a double free, strictly worse than the tombstone. So the fix is neither "carry
the pointer" nor "keep copying": the borrowed case needs the copy **without** the source flag,
or no copy and no obligation. Deciding which is the work.

Note `copy_resource_to_current_arena`'s source flag is already conditional on the SEND path
(the success-gated `suppress_resource_source_flag`, `.ai/resources-packages.md`), so the
machinery for suppressing it exists.

## Goal

- Polling a list under an inline `TRAP` leaves every element open, owned by the list, and
  closed exactly once at the list's drop.

### Non-goals (must NOT change)

- bug-498's rule: a sender must never allocate in the receiver's arena.
- The thread hand-over path must keep tombstoning its source — that flag is what makes a
  transferred handle's second close a no-op.
- No double free on the borrowed path (the reason bug-643 left it alone).

## Blast Radius

- Every builtin that returns a borrowed resource pointer under an inline `TRAP`:
  `tcp|udp|tls::poll` over a list, `collections::get`/`getOr` of a `List OF RES …`. Audit
  `target_returns_borrowed_resource` (`codegen/engine/value/builder_values.rs`) for the full
  list — bug-643 split it out as the single source of truth.

## Phases

### Phase 1 — failing test + audit

- [ ] A reproduction where `poll` actually returns an element under a `TRAP`; confirm the
      source is tombstoned; confirm whether `collections::get`/`getOr` has the same shape.

Commit: —

### Phase 2 — the fix

Commit: —

### Phase 3 — full validation

Commit: —

## Summary

A thread hand-over's "I gave this away" flag applied to a handle that was only borrowed.
