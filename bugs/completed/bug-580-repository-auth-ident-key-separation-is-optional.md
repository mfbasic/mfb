# bug-580: repository permits an auth key to double as the ident key

Last updated: 2026-09-12
Effort: medium (1h–2h)
Severity: LOW
Class: Security / footgun

Status: **CLOSED** (`e18cbb6fa`) — but **not as filed.** The reported
vulnerability does not exist; the separation was already enforced, incidentally,
by a global UNIQUE index. What landed makes it an explicit, account-scoped
invariant so that loosening that index (which this bug's own non-goals ask for)
cannot silently remove it.
Regression Test: `store::tests::key_role_separation_is_enforced_at_every_creation_path`,
`store::tests::distinct_keys_still_register_link_tokenize_and_rotate`, and
`store::tests::key_insertion_has_exactly_one_writer`.

The repository verifies role-separated proof messages but permits the same
Ed25519 public key in both the machine-auth and account-ident roles.  This
silently defeats the system's intended credential separation: theft of a
machine auth private key is also theft of the identity-signing private key.

Correct behavior: an account's current ident key and every current auth key
must have distinct public-key fingerprints; requests attempting to reuse one
are rejected before state changes.

References:

- Security review, 2026-09-11 (repository-only scope)
- `repository/src/crypto.rs:registration_message`

## Failing Reproduction

Add a store test that generates one keypair, makes valid `auth` and `ident`
role-specific proofs with that same private key, and calls `register_owner`:

```
cargo test -p mfb_repository store::tests::registration_rejects_an_auth_key_equal_to_the_ident_key
```

- Observed today: `Store::register_owner` succeeds because both role-specific
  proofs verify.
- Expected: it returns an explicit key-role-separation error and persists no
  owner or key rows.

Add the analogous linked-machine test with the current ident public key.

## Root Cause

`repository/src/store.rs:Store::register_owner` verifies two domain-separated
proofs but never compares `auth_key` and `ident_key`.  `Store::add_auth_key`
likewise verifies the auth proof and inserts the key without checking the
account's current ident key.  Domain separation prevents proof replay; it does
not prevent intentional reuse of one private key across both roles.

## Goal

- Reject auth/ident equality at registration and machine-link insertion.
- Preserve independent role-proof validation and support for multiple distinct
  machine auth keys.

### Non-goals (must NOT change)

- Do not alter Ed25519, fingerprint, or registration-message wire formats.
- Do not invalidate an existing account automatically without an explicit
  migration/recovery policy.
- Do not prohibit two different accounts from independently choosing the same
  public key unless a separate account-identity policy requires it.

## Blast Radius

- `store.rs:register_owner` — fixed in this bug.
- `store.rs:add_auth_key` — fixed in this bug.
- `server.rs:register` and `link_fetch` — unaffected directly because they
  funnel into the store methods, but require endpoint-level regression tests.
- `store.rs:rotate_ident` — audit for collision with existing auth keys; fixed
  here if the invariant is declared account-wide.

## Fix Design

Compare fingerprints/public-key bytes at every insertion or rotation boundary
inside the transaction.  Decide and document how already-created colliding
accounts are handled before enforcing an account-wide invariant; fail closed
for new credentials without silently revoking live access.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [x] Add registration, linking, and rotation-collision tests.
- [x] Inventory all key insertion paths and existing-data migration needs.

**Acceptance NOT met, because it is false.** "Equal-key attempts succeed on HEAD"
is the one thing that had to be true for this bug to be real, and it is not.
Measured on the pre-fix store, every path already refused:

    register auth==ident         refused: UNIQUE constraint failed: keys.fingerprint
    link machine = ident key     refused: UNIQUE constraint failed: keys.fingerprint
    token = ident key            refused: UNIQUE constraint failed: keys.fingerprint
    rotate ident -> auth key     refused: UNIQUE constraint failed: keys.fingerprint
    reanchor ident -> auth key   refused: UNIQUE constraint failed: keys.fingerprint
    acct1 shared auth key        ACCEPTED
    acct2 SAME auth key          refused: UNIQUE constraint failed: keys.fingerprint

`repository/src/store.rs:464` declares `fingerprint TEXT NOT NULL UNIQUE` — a
GLOBAL uniqueness constraint over the whole `keys` table. No two rows anywhere
may share a public key, so auth/ident collision was structurally impossible. The
report reasoned from `register_owner`'s body (which indeed never compares the two
keys) without checking the schema the insert lands in.

**The inventory was also wrong, and in the direction that matters.** The report
names two insertion paths plus one to audit. There are FIVE:
`register_owner` (x2), `add_auth_key`, `rotate_ident`, `reanchor_ident`, and
`issue_publish_token` (`grep -n "INSERT INTO keys"`). The two it missed include
the highest-risk one: a publish token is a **delegated, exportable** credential
handed to CI, so a token equal to the ident key would hand over the account
identity. Any per-site fix would have been written from the report's list.

No existing-data migration is needed: no colliding pair can exist, because the
index has always forbidden it.
Commit: — (tests landed with the fix, `e18cbb6fa`)

### Phase 2 — the fix

- [x] Enforce key-role distinctness transactionally at every audited boundary.
- [x] Add a documented policy for legacy collisions (there are none — see above).

Given the property already holds, what landed is aimed at the **latent** defect
rather than an exploit: the guarantee was incidental, and this bug's own
non-goals ask for the index it depends on to be loosened.

> "Do not prohibit two different accounts from independently choosing the same
> public key unless a separate account-identity policy requires it."

The global UNIQUE currently *does* prohibit exactly that (row 7 of the
measurement). So the non-goal is already violated, and anyone who fixes it by
relaxing the index removes auth/ident role separation as a side effect, with
nothing failing. Enforcing the account-scoped invariant directly lets the two
properties move independently.

Every `keys` row is now created by one writer, `insert_key_tx`, which checks for
a current key of the OPPOSITE role with the same fingerprint inside the caller's
transaction — so a concurrent rotation cannot slip a collision between the read
and the insert. `register_owner` needs no special case: it writes the auth row
first, so the ident insert's query sees it.

Acceptance, restated honestly: equal-key attempts fail atomically **with an
explicit message instead of a database constraint error**, and distinct
multi-machine keys still work.
Commit: `e18cbb6fa`

### Phase 3 — full validation

- [x] Run `cargo test -p mfb_repository`.
- [x] Verify registration and pairing workflows with distinct keys end-to-end.

`cargo test -p mfb_repository --no-fail-fast` -> **366 + 24 passed, 0 failed,
exit 0**, in an isolated `git worktree add --detach`.

One pre-existing test went red and was RIGHT to:
`losing_the_key_tables_errors_revocation_and_chain_reads` (bug-264 / REPO-09)
pins that a dropped `keys` table degrades to an error naming the OPERATION
("failed to register token key: …"). Adding a read ahead of the insert re-worded
five operator-facing failures. The contract must not depend on which statement
happens to touch the broken table first, so the separation query maps its SQL
error to the caller's own message. **The test is unmodified.**

**Instrument.** The artifact gate is structurally blind to this crate (no IR, no
goldens). The `mfb_repository` unit suite is the instrument.

Acceptance: **met.**
Commit: `e18cbb6fa`

## Outcome

Closed by `e18cbb6fa`. **Not a vulnerability** — see Phase 1.

### The lesson

**A security property can be true by accident, and an accident is not a
guarantee.** Role separation held here because of a `UNIQUE` index on
`keys.fingerprint` that exists for a different reason. Nothing named the
property, no test asserted it, and the one document that discussed it — this
bug report — got its direction exactly backwards on both counts: it said the
collision was permitted (it was not) and that cross-account key sharing was
permitted (it is not).

The dangerous shape is the combination: an incidental guarantee whose support
is something an open item wants to remove. Loosening the index to satisfy this
bug's own non-goal would have deleted the separation silently. When you find a
property holding for a reason unrelated to its purpose, that is worth a test and
an explicit check even though nothing is broken today.

### Still open, deliberately

Two different accounts cannot currently choose the same public key. That
diverges from this bug's stated non-goal. It is left alone: letting one key
authenticate as two accounts is a policy decision with its own security
argument, not something to settle while fixing an unrelated invariant. The
current behaviour is now asserted in
`distinct_keys_still_register_link_tokenize_and_rotate`, so changing it is a
deliberate edit to that line rather than a silent drift.
