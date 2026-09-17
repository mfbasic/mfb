# bug-648: a trapped `tcp/udp/tls::poll` result tombstones the live list element it borrowed

Last updated: 2026-09-16
Effort: medium (1h–2h)
Severity: **HIGH** (a live, still-owned handle is flagged `moved|closed`)
Class: Correctness (resource lifetime)

Status: Open
Regression Test: `tests/net/rt_inline_trap_borrowed_resource.rs` (5 cases, RED at 3b94f621e)

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

Measured on the main thread at `3b94f621e` (macos-aarch64, debug `mfb`). Every case is a
case in `tests/net/rt_inline_trap_borrowed_resource.rs`, run under `ulimit -n 64`:

| case | shape | observed |
|---|---|---|
| tcp/udp poll | `FOR i = 1 TO 3 … RES ready = tcp::poll(socks, 5000) TRAP(e) … END TRAP; tcp::read(ready)` | `tcp 1 t`, then `Error: 7-703-0004` (already closed) |
| tls poll | same loop, `openssl s_client` peer | `tls 1 TRUE`, then `tls poll trapped Resource handle is already closed.` |
| get | `RES got = collections::get(socks, 0) TRAP(e)` in a loop, then an untrapped `poll` | `get 1 g`, then `7-703-0004` **and** `Cleanup failure: 7-703-0009` (moved) at the list's drop |
| RECOVER alias | `RES c = udp::bind(badHost, 0) TRAP … RECOVER outer` in a loop | `7-703-0004` on iteration 2: `outer` was closed at the end of iteration 1 |
| mixed | borrowed `poll` success + `RECOVER udp::bind(...)` | `7-703-0004` on the first poll after a borrowed success |

The same `tcp::poll` loop **without** `TRAP` prints all three iterations and exits 0, so the
defect is TRAP-specific. `tests/rt-behavior/tcp/tcp-udp-poll-list-trap-rt` never saw it for the
reason this doc gave (its polls time out) and a second one: its list and its trap binding share
one function scope, so a close at scope exit is indistinguishable from the list's own.

## Root Cause

**The doc's mechanism is right for `get` and wrong for `poll`.** `mfb build -ncode` of the
tcp poll loop contains no `thread_copy_resource*` label at all: the list-`poll` result never
reaches `copy_resource_to_current_arena`. The single root cause that covers every row is in
ownership, not in the copy:

1. **The `$trap_valN` temp owns whatever it is assigned** (`builder_control.rs`, `NirOp::Bind`).
   The inline-TRAP desugar (`ir/lower.rs`) binds `MUT $trap_valN : T` with **no initializer**,
   assigns it `ResultValue($trap_resN)` on Ok and the `RECOVER` value on Err, then binds the
   user's name to `Local($trap_valN)`. The user bind is an alias (`value_aliases_live_resource`
   says `Local` → no cleanup). `$trap_valN` has no value to classify, so `owns_resource_slot`
   is true and it registers `ActiveCleanup::Resource` — with `frees_record` set for the net
   handles. At its scope exit it closes the list's element **and frees its 96-byte record**.
   Every borrowed Ok value and every aliasing `RECOVER` value (`RECOVER outer`, a field, a
   borrowed call) is wrongly owned.
2. **The Ok-path `Result` wrap deep-copies a borrowed SENDABLE element** (`get` only).
   `lower_inline_builtin_raw`/`lower_inline_infallible_raw` pass `RawSuccessBlock::OwnedElsewhere`,
   so `materialize_current_result` runs the element through `copy_value_to_current_arena`, whose
   sendable-resource arm is `copy_resource_to_current_arena`: a second record, and the element
   tombstoned `moved|closed` on the spot — the doc's original mechanism. The list-`poll` result
   is spelled with the bare type, so it takes the non-sendable pointer-carry arm instead and is
   never copied (which is why `tls::poll`, non-sendable by type, fails identically to `tcp::poll`).

Ownership of `$trap_valN` is a property of **each assignment**, not of the temp: a borrowed
success can recover an owned value (`RECOVER udp::bind(…)`), and an owned success can recover an
alias (`RECOVER outer`). A static "own" leaks nothing but closes live handles; a static "don't
own" closes nothing but leaks every recovered/produced handle. The mixed shapes need a run-time
answer.

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

- [x] A reproduction where `poll` actually returns an element under a `TRAP`; confirm the
      source is tombstoned; confirm whether `collections::get`/`getOr` has the same shape.
      (Measured: `poll` is closed, not tombstoned; `get` is tombstoned AND closed; an aliasing
      `RECOVER` value is closed too — see Root Cause.)
- [x] RED tests: `tests/net/rt_inline_trap_borrowed_resource.rs`, 5/5 failing on the mechanism.

Commit: —

### Phase 2 — the fix

Commit: —

### Phase 3 — full validation

Commit: —

## Summary

A thread hand-over's "I gave this away" flag applied to a handle that was only borrowed.
