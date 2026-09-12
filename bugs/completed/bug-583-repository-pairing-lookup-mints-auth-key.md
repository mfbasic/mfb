# bug-583: a machine-pairing lookup is sufficient to mint an account auth key

Last updated: 2026-09-12
Effort: large (3h–1d)
Severity: MEDIUM
Class: Security / credential lifecycle

Status: Fixed
Regression Test: `server::tests::pairing_lookup_without_code_cannot_enrol_an_auth_key`
(relay adversary + the honest link that still completes),
`crypto::tests::pairing_approval_is_derived_from_the_code_and_binds_the_auth_key`,
`store::tests::pairing_blob_is_single_use_and_expires` (an unapproved fetch
consumes nothing), and the two client pins
`client::tests::link_start_relays_only_a_blob_sealed_under_the_pairing_code` /
`client::tests::link_fetch_installs_the_relayed_ident_keypair`.

The machine-link server sees the code-derived `lookup` when the old machine
starts pairing.  Any party with that lookup—including the registry relay or a
database reader—can call `/machines/link/fetch` with its own auth key and a
valid proof.  The handler consumes the pairing row and inserts that key as a
current account auth key before returning the encrypted ident blob.  The
attacker cannot decrypt the ident blob, but can complete login challenges and
obtain a live account session.

Correct behavior: possession of the pairing code, not merely the server-visible
lookup, authorizes enrollment of the new auth key; a relay that cannot decrypt
the ident blob must not be able to impersonate the account at the auth layer.

References:

- Security review, 2026-09-11 (machine-link authority trace)
- `repository/src/crypto.rs:pairing_lookup`

## Failing Reproduction

In the server test harness, start a legitimate link and retain only its lookup.
Generate an attacker auth key, sign its ordinary auth registration proof, and
POST it to `/machines/link/fetch` without the pairing code:

```
cargo test -p mfb_repository server::tests::pairing_lookup_without_code_cannot_enrol_an_auth_key
```

- Observed today: the request succeeds; the attacker can then obtain a
  challenge for its fingerprint and log in.
- Expected: enrollment is refused and the legitimate pairing remains usable.

## Measured (2026-09-12)

What the relay actually sees when the old machine parks a blob
(`store_pairing_blob` row): `owner_id`, `lookup`, `blob`, `salt`, timestamps.
What it can replay: the whole `link_fetch` request — the only authorization
material was the `lookup` plus a proof-of-possession over a key of the
caller's own choosing. Nothing in that request was bound to the pairing code.

Reproduction, run against the pre-fix code: the RED test
`server::tests::pairing_lookup_without_code_cannot_enrol_an_auth_key` posted
`/machines/link/fetch` with the lookup and a relay-minted auth key and
`expect`ed a refusal. It failed with `expected an error response` at
`server.rs:err_of` (`cargo test -p mfb_repository --no-fail-fast
pairing_lookup_without_code` → exit 101): the enrollment SUCCEEDED. The bug
doc's stated root cause held up exactly.

## Root Cause

`repository/src/server.rs:link_start` stores the lookup, blob, and salt.  In
`link_fetch`, the only authorization material checked is possession of the new
auth private key; `Store::take_pairing_blob(owner, lookup)` treats the lookup as
the bearer capability and `Store::add_auth_key` persists it.  The code is never
presented or proven in this flow.  The current encryption protects the ident
private key, not the authority to add an auth key.

## Goal

- Bind the new auth key to an approval that a lookup-only relay cannot forge.
- Preserve server blindness to the transferred ident private key.
- Prevent an attacker from consuming a legitimate pairing attempt.

### Non-goals (must NOT change)

- Do not send the raw pairing code to the relay, which would let it decrypt the
  stored blob.
- Do not weaken the 125-bit code or single-use/TTL requirements.
- Do not grant a publish token the ability to enroll a permanent machine key.

## Blast Radius

- `server.rs:link_fetch` and `store.rs:take_pairing_blob` — fixed in this bug.
- `client.rs:link_start` / `link_fetch` — require the revised approval flow.
- `crypto.rs:{pairing_lookup,seal_pairing_blob,open_pairing_blob}` — audit for
  a PAKE or a pre-bound-key redesign; do not retrofit an unverifiable HMAC.

## Fix Design (as landed)

A protocol change, and no PAKE is needed: the old machine already holds the
code when it parks the blob, so it can publish a **verifier** for it.

- `crypto::pairing_approval_keypair(code)` derives an Ed25519 keypair whose
  seed is `argon2id("mfb-pairing-approval-v1\0" || code, salt = lookup)` —
  domain-separated from the blob key (different password prefix, different
  salt), so it can never open the blob.
- `link_start` carries the PUBLIC half (`approvalKey`); the store keeps it on
  the pairing row. Publishing it leaks nothing: forging under it needs the
  125-bit code through argon2id.
- `link_fetch` carries `approval`, a signature over
  `"mfb-pairing-approve-v1\0" || lookup || "\0" || authKey`. Binding the auth
  key defeats relay key substitution; binding the lookup defeats replay onto
  another pairing.
- `Store::take_pairing_blob` now takes an `approve` closure evaluated INSIDE
  the transaction and returns `PairingFetch::{Relayed,Missing,Unapproved}`. An
  unapproved fetch rolls back, so a lookup-only caller can neither read the
  blob nor spend the honest machine's single-use pairing.

The change only ADDS a binding: every pre-existing check (session-authenticated
`link_start`, the bug-492 publish-token refusal, the role-separated auth proof,
single-use + 600s TTL, owner folding) is untouched, and the new check runs
after them.

### Contract

plan-23 §3.2 step 1 — the new machine's auth key is registered "authenticated
by an existing session / **pairing approval**" — read with plan-23 §2: "the
server holds no user private keys... a full server compromise yields zero user
keys", and §3.2 step 2's "a blob the server cannot read". A server-visible
lookup is not a pairing approval; treating it as one handed the relay account
authority at the auth layer while the plan says a compromised server "cannot
sign proofs" and gets nothing. The fix makes the approval the one thing the
relay provably does not have: the code.

### Docs vs code

They agreed, and both were wrong against plan-23.
`src/docs/spec/package-manager/01_repository-protocol.md` stated "Presenting
the correct code-derived lookup **is** the pairing approval" — an accurate
description of the defect, not a contract that justified it. Evidence that the
spec is the side to change: plan-23 §2/§3.2 (above) is the design of record
the code cites, and the spec page itself already claims in the next paragraph
that the relay "can neither read the blob nor derive its key" — a property it
then makes irrelevant by granting the relay auth authority anyway. The spec
page is updated with the fix.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [x] Add the lookup-only relay attack and a legitimate-link contrast test.
- [x] Document the desired server-blind pairing threat model.

Acceptance: the attack currently creates a usable auth key; the contrast works.
Measured RED before the fix: `expected an error response`, exit 101.
Commit: 01d3529d1 (landed with Phase 2 — a knowingly-red test is never
pushed to main on its own).

### Phase 2 — the fix

- [x] Implement a reviewed key-approval protocol that resists relay key
  substitution and lookup replay.
- [x] Make consumption atomic with successful authorization.

Acceptance: lookup-only attack fails; only the approved key can enroll.
Commit: 01d3529d1

### Phase 3 — full validation

- [x] Run `cargo test -p mfb_repository`.
- [x] Exercise success, wrong-code, lookup replay, relay substitution, and
  interrupted-pairing recovery.

Acceptance: full suite green and no relay-visible value confers auth authority.
`cargo test -p mfb_repository --no-fail-fast` → 345 + 21 passed, 0 failed,
exit 0. Covered: honest end-to-end link (blob opens under the code, new key
opens a session), a forged approval, an all-zero approval, an honest approval
re-pointed at the relay's key, lookup replay onto another pairing
(`pairing_approval_is_derived_from_the_code_and_binds_the_auth_key`), and
recovery — every refusal above leaves the pairing usable, proven by the honest
fetch that follows them in the same test.
Commit: 01d3529d1
