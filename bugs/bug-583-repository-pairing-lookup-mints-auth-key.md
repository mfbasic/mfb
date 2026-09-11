# bug-583: a machine-pairing lookup is sufficient to mint an account auth key

Last updated: 2026-09-11
Effort: large (3h–1d)
Severity: MEDIUM
Class: Security / credential lifecycle

Status: Open
Regression Test: `repository/src/server.rs` machine-link tests to be extended
with a relay adversary that knows the stored lookup but not the pairing code.

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

## Fix Design

This needs a protocol change, not a lookup-format tweak.  Prefer binding the
new machine's public key into an approval established over an authenticated
out-of-band step, or adopt a reviewed PAKE/OPAQUE-style exchange that proves
the code without disclosing it to the relay.  Explicitly prove that the relay
cannot substitute an attacker key or spend the pairing record.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add the lookup-only relay attack and a legitimate-link contrast test.
- [ ] Document the desired server-blind pairing threat model.

Acceptance: the attack currently creates a usable auth key; the contrast works.
Commit: —

### Phase 2 — the fix

- [ ] Implement a reviewed key-approval protocol that resists relay key
  substitution and lookup replay.
- [ ] Make consumption atomic with successful authorization.

Acceptance: lookup-only attack fails; only the approved key can enroll.
Commit: —

### Phase 3 — full validation

- [ ] Run `cargo test -p mfb_repository`.
- [ ] Exercise success, wrong-code, lookup replay, relay substitution, and
  interrupted-pairing recovery.

Acceptance: full suite green and no relay-visible value confers auth authority.
Commit: —
