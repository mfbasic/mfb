# bug-493: `/machines/revoke` accepts an auth-key challenge — any auth key can revoke every auth key on the account

Last updated: 2026-09-03
Effort: medium (3h–1d)
Severity: HIGH
Class: security (authorization bypass / denial of service — account lockout)

Status: Open (found in audit-3, Surface 8 REPO-02; reproduced live by the lead)

Regression Test: none yet — add a registry test asserting `/machines/revoke` rejects a challenge minted for an auth key (only an ident-key challenge suffices), and that revocation cannot remove the last current auth key.

## Summary

Revocation authority is supposed to be the **ident key alone** (plan-23 §3.6);
the handler doc says so verbatim. But the challenge loader never constrains the
key role, so a challenge minted by `/auth/challenge` for an ordinary **auth** key
satisfies the revocation path. Any single auth private key of an account — a
linked machine, or the deliberately-limited publish token from bug-492 — can
therefore revoke every *other* auth key on the account. Walked to completion this
locks the account out permanently: there is no ident-authorized "add auth key"
route, so an account with zero current auth keys is unrecoverable without the
operator `reanchor` ceremony.

## Mechanism

```rust
// repository/src/store.rs:1175
pub fn complete_revocation_challenge(&self, challenge_id, signature, ...) {
    self.complete_challenge_with(challenge_id, signature, |id, nonce| {
        crypto::revocation_message(...)          // domain-separated message
    })
}
```

```rust
// repository/src/store.rs:1196 — the loader for BOTH login and revocation
"SELECT c.id, c.owner_id, c.key_id, c.nonce, c.expires_at, c.used_at,
        o.owner_display, k.public_key, k.fingerprint
 FROM auth_challenges c JOIN owners o ON o.id = c.owner_id
 JOIN keys k ON k.id = c.key_id
 WHERE c.id = ?1"                                 # no AND k.role = 'ident'
```

There is no `role`/`purpose` predicate, and `create_ident_challenge` /
`create_auth_challenge` write structurally identical rows. `revoke_challenge`
(`server.rs:2445`) *does* create an ident challenge — but the attacker never uses
it; they submit an `/auth/challenge` id to `/machines/revoke` and sign the
revocation message with their auth key. The message-level domain separation does
not help: the attacker simply signs the revocation message.
`revoke_auth_key` (`store.rs:1115`) has no last-key guard.

## Reproduction (lead-run, live, 2026-09-03)

`spikes/audit-3/repository-authz/` — `cargo run --bin revoke -- victimB1`:

```
ident private key is NEVER used below
revoke status = 200 OK
revoke body   = {"authFingerprint":"46a9f357…","revoked":true}
primary key still usable for login? false           # a secondary auth key killed the primary
```

Expected: 400 — revocation requires the ident key.

## Best fix

Bind a challenge to its purpose at creation. Add a `purpose` column
(`'login'` / `'revoke'`) to `auth_challenges`, set it in
`create_auth_challenge` / `create_ident_challenge`, and thread an expected
purpose into `complete_challenge_with` so the loader filters on it *and* on
`k.role` (`'auth'` for login, `'ident'` for revoke). Separately, refuse a
revocation that would leave the owner with no current auth key.

## Non-goals

- No wire-shape change to `/auth/challenge`, `/machines/revoke/challenge`, or
  `/machines/revoke`.
- Do not weaken the existing nonce / single-use / expiry properties.

## Prior art

None found (searched `complete_challenge_with`, `complete_revocation_challenge`,
`revoke_auth_key`, "revocation authority", `role = 'ident'` across `bugs/`,
`bugs/completed/`, `bugs/skipped/`, `planning/completed/audit-1-repository.md`,
`audit-2-repository.md`, bug-419/271/276).
