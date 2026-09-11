# bug-580: repository permits an auth key to double as the ident key

Last updated: 2026-09-11
Effort: medium (1h–2h)
Severity: LOW
Class: Security / footgun

Status: Open
Regression Test: `repository/src/store.rs` and `repository/src/server.rs`
registration/linking tests to be added.

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

- [ ] Add red registration, linking, and rotation-collision tests.
- [ ] Inventory all key insertion paths and existing-data migration needs.

Acceptance: equal-key attempts succeed on HEAD and the tests capture that fact.
Commit: —

### Phase 2 — the fix

- [ ] Enforce key-role distinctness transactionally at every audited boundary.
- [ ] Add a documented policy for legacy collisions.

Acceptance: equal-key attempts fail atomically; distinct multi-machine keys work.
Commit: —

### Phase 3 — full validation

- [ ] Run `cargo test -p mfb_repository`.
- [ ] Verify registration and pairing workflows with distinct keys end-to-end.

Acceptance: full suite green and role isolation holds at every creation path.
Commit: —
