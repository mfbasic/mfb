# bug-274: package-transfer offer authorization is non-atomic (TOCTOU: dispossessed owner can re-bind a stale offer)

Last updated: 2026-07-18
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Security (TOCTOU) / Correctness

Status: Fixed 2026-07-18
Regression Test: `repository/src/store.rs` —
`store::tests::a_stale_offer_cannot_rebind_a_package_after_ownership_moved`

## Resolution

Two changes, defense in depth:

1. `create_transfer_offer` re-reads the package row *inside* the writing
   transaction and aborts unless `owner_id` still equals the account it authorized
   against. The separate pre-transaction `SELECT id FROM packages` is gone —
   package id and current owner now come from the same in-transaction read.
2. `accept_transfer`'s offer lookup gained `AND o.from_owner_id = p.owner_id`, so
   an offer is only acceptable while the account that made it still owns the
   package. This neutralizes a resurrected row even if one were somehow written.

Testing note: racing two threads and hoping to hit the window makes a flaky test.
The regression test instead writes the exact row state the interleave produces — a
pending offer whose `from_owner_id` is no longer the owner — and asserts it cannot
be accepted. Deterministic, and it exercises the outcome the race could reach.
Verified failing without the guard: carol successfully re-binds a package alice no
longer owns.

### Correction to this report's audit

It states rotate/revoke "run their check and write in the same transaction;
unaffected". That is **not accurate for `rotate_ident`**: it resolves the current
key via `owner_with_ident_key` under its own lock acquisition, then inside the
transaction updates by that captured `old_key.id` without re-verifying it is still
current — the same check-then-write shape. There is no `revoke_ident`.

Not fixed here, to keep this bug scoped. It is bug-276's R1 item (the `rotate_ident`
sibling of bug-272's ordering bug) and is addressed there.

`create_transfer_offer` performs its ownership authorization
(`self.package_owner(ident)` then `fold_owner(...) != fold_owner(from_owner)`)
under one mutex acquisition, releases the lock, and later performs the mutating
UPSERT in a *separate* transaction under a fresh acquisition — four separate
`self.conn()` acquisitions total, and the write transaction never re-verifies
ownership. Its `ON CONFLICT(package_id) DO UPDATE ... accepted_at = NULL`
overwrites any offer row, including one already accepted. Because axum handlers
run concurrently over `Arc<Mutex<Connection>>` in WAL mode, an offer sequence and
an accept sequence can interleave: A offers to B; B accepts (owner→B); A's
still-in-flight second transaction commits, resetting `accepted_at = NULL` and
re-listing the offer as pending under A's now-stale authority. The offer's
`from_owner_id` was captured before the accept, so a stale-but-authorized offer
resurrects after ownership has already moved.

The single correct behavior a fix produces: ownership is re-checked inside the
same transaction that writes the offer, and accept only binds an offer whose
`from_owner` still matches the current owner — so no interleaving can resurrect a
stale offer or move a package the offerer no longer owns.

References:

- `planning/old-plans/audit-2-repository.md` (traced these routes as a
  single-threaded logical check; did not consider cross-transaction interleave).
- Found during goal-06 review of `repository/src/store.rs`.

## Failing Reproduction

```
# Concurrent, same ident:
#   Thread 1: POST /packages/transfer/offer  (owner A → B)
#   Thread 2: POST /packages/transfer/accept (recipient B)
# with the offer's authorization snapshot taken before the accept commits.
```

- Observed: after both complete, the offer row is pending again with
  `from_owner_id = A` even though ownership moved to B; A can drive a transfer it
  no longer has authority for.
- Expected: once ownership moves to B, A's in-flight offer transaction aborts
  (ownership re-check fails) and cannot reset the accepted offer.

This is a race; reproduction requires concurrent requests (or an injected delay
between the check and the write to widen the window).

## Root Cause

`repository/src/store.rs:1567-1623` (`create_transfer_offer`) checks ownership
under one lock, then commits the UPSERT under another with no re-check;
`store.rs:1628-1673` (`accept_transfer`) updates without guarding on the expected
`from_owner_id`. The check and the mutation are in different transactions over an
interleavable connection.

## Goal

- The ownership re-check (`SELECT owner_id FROM packages WHERE id=?`) executes
  inside the same transaction as the offer UPSERT and aborts if it no longer
  equals `from_owner`.
- `accept_transfer`'s UPDATE guards `WHERE from_owner_id = <expected>` so a stale
  offer cannot re-bind.

### Non-goals (must NOT change)

- The transfer wire protocol / endpoint shapes.
- The happy-path single-threaded offer→accept flow.
- WAL mode / the `Arc<Mutex<Connection>>` model (fix is transactional, not a
  locking-model rewrite).

## Blast Radius

- `store.rs:create_transfer_offer` — fixed by this bug.
- `store.rs:accept_transfer` — guarded by this bug.
- Other check-then-write pairs in store.rs (rotate/revoke) — audited: those run
  their check and write in the same transaction; unaffected. Confirm during the
  fix.

## Fix Design

Move the ownership check into the offer's write transaction and re-select the
current owner immediately before the UPSERT; abort the transaction if it changed.
Add a `from_owner_id` guard to `accept_transfer`'s UPDATE. Rejected alternative:
a coarse global "transfer lock" — unnecessary; a same-transaction re-check is
sufficient and doesn't serialize unrelated packages.

## Phases

### Phase 1 — failing test + audit
- [ ] Test with an injected delay between check and write proving resurrection;
      audit rotate/revoke for the same pattern.
### Phase 2 — the fix
- [ ] Same-transaction re-check + `accept_transfer` guard.
### Phase 3 — validation
- [ ] Full `repository/` suite green; concurrent offer/accept test passes.

## Validation Plan

- Regression test: interleaved offer/accept cannot resurrect a stale offer.
- Runtime proof: ownership-moved offer transaction aborts.
- Doc sync: none.

## Summary

Authorization is checked against DB state that can change before the write
commits. Folding the re-check into the write transaction and guarding accept on
the expected owner closes the window; risk is in getting the transaction
boundaries right without deadlocking the single connection.
