# bug-492: a scoped publish token self-escalates to a permanent unscoped auth key via machine pairing, surviving revocation

Last updated: 2026-09-03
Effort: medium (3h–1d)
Severity: HIGH
Class: security (authorization bypass / privilege escalation)

Status: Open (found in audit-3, Surface 8 REPO-01; reproduced live by the lead)

Regression Test: none yet — add a registry test asserting a token-backed session cannot `link_start`, and that `/signing` enforces scope for every auth key of the account.

## Summary

A publish token is the credential the registry hands to CI, and the design says
it is *narrower* than the account (scoped to one package, short TTL, revocable).
It is not: a token holder can mint a **new, unscoped, permanent** auth key for
the whole account through the machine-pairing relay, and that key keeps working
after the token is revoked. From there it obtains registry attestations for any
package the account owns and clears `/publish` for the whole namespace.

## Mechanism

`/signing` is the only route that consults a token's scope/expiry, and it looks
the token up by the *session's key id*:

```rust
// repository/src/server.rs:2609
if let Some((scope, expires_at, revoked_at)) = state.store.publish_token_for_key(key.id) {
    ... scope_permits / expiry / revoked checks ...
}
```

```rust
// repository/src/store.rs:2177
"SELECT scope, expires_at, revoked_at FROM publish_tokens WHERE key_id = ?1"
```

A *different* auth key on the same account has no `publish_tokens` row, so
`publish_token_for_key` returns `None` and the entire scope gate is skipped — the
only remaining package check is the ident-prefix `fold_owner` comparison, which
every package of the account passes.

`link_start` requires nothing but a session (which the token has after
`/auth/login`) and `claims.sub == owner` (`server.rs:2352-2375`); `link_fetch`
takes no account credential — only a proof-of-possession over the *attacker's
own* new key, plus the `lookup`/`salt` the same attacker chose in `link_start`
(`server.rs:2402-2437`) — and calls `store.add_auth_key`, creating an auth key
with no `publish_tokens` row. A token session thus supplies both halves of a
"pairing" with itself. This breaks the invariant stated in `issue_token`'s own
doc (`server.rs:1766-1769`): "The token ... never bypasses the ident-proof
requirement."

## Reproduction (lead-run, live, 2026-09-03)

Harness at `spikes/audit-3/repository-authz/` (`cargo run --bin tokenesc -- orgB1`
against a local `mfb-repo`). Observed:

```
token /signing orgB1#ci-only  -> 200 OK
token /signing orgB1#flagship -> 400 Bad Request     # token scope enforced
new UNSCOPED auth key fp = ed564f2e…                 # via /machines/link self-pairing
CI token revoked = true                              # owner revokes the token
escalated key /signing orgB1#flagship -> 200 OK      # signs flagship anyway, post-revoke
body: {"attestation":"{…\"ident\":\"orgB1#flagship\",\"version\":\"6.6.6\"…}", …}
```

Expected: refused, and refused permanently once the token is revoked.

## Best fix

Two independent gates:

1. Make scope/expiry a property of the **session**, not of the key looked up at
   one route: resolve `publish_token_for_key` from the session's key id inside
   `verify_session_token`'s callers (or mint a `scope`/`token_expires_at` claim
   at `/auth/login`), and enforce it in `link_start`, `put_blob`, `validate`,
   `publish` and `signing`. A delegated credential must never widen itself.
2. Refuse `link_start` for a token-backed session outright — enrolling a new
   machine is account authority and belongs behind the same ident-signature
   requirement `/tokens` and `/release-state` use.

Closing REPO-15's separate hole (proving code knowledge on `link_fetch`) does
not fix this: here the attacker legitimately knows the code it just set.

## Non-goals

- A normal two-machine link (an ident-key holder enrolling a machine) must still
  work.
- Do not weaken `/signing`'s existing scope check.
- No registry HTTP contract change to the link request/response shapes.

## Prior art

audit-2 REPO-15 / `bugs/completed/bug-271` §REPO-15 is the same *route* but was
DEFERRED on "the rogue key cannot publish/rotate and is revocable → bounded
impact." This repro contradicts that reasoning: the rogue key *can* obtain
signing attestations (→ publish), and it is *not* revoked with the token that
created it. Searched `link_fetch`, `add_auth_key`, `publish_token_for_key`,
`pairing`, REPO-15 across `bugs/`, `bugs/completed/`, `bugs/skipped/`,
`planning/completed/audit-*`.
