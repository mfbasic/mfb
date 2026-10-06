# bug-955: ident-authorized mutations (release-state, org role, token revoke) sign no nonce, so a captured signature can be replayed with any live session

Last updated: 2026-10-05
Effort: medium (1h–2h)
Severity: LOW
Class: Security

Status: Open
Coordinator note: measured by the review agent against a local `mfb-repo` on 127.0.0.1 only; the coordinator did not re-run it.
Regression Test: (proposed) repository/src/server.rs `a_release_state_signature_cannot_be_replayed`

**Owner decision (2026-10-05):** every ident-authorized mutation (release state/yank, org role, token revoke, and any
other signed mutation without a nonce) signs a one-time server-issued challenge, as auth already
does. The server consumes the challenge on use, so a captured signature cannot be replayed. The
repository protocol spec documents the challenge field per request.

`/release-state`, `/orgs/members`, `/tokens/revoke` (and `/tokens`) are "ident-authorized":
the request carries a live session (liveness) plus an ident signature (authority). The signed
messages are `release_state_message(ident, version, state)`,
`org_role_message(org, member, role)` and `token_revoke_message(owner, fingerprint)`. None of
them contains a challenge, nonce, timestamp or sequence. Ed25519 is deterministic, so an
earlier signature for the same tuple is byte-identical to a fresh one. Anyone who has once
obtained a signature for a state, for example `yanked` (from a client log, a proxy, or a TLS-terminating
CDN), together with *any* live auth session of that owner can re-apply it forever (until the ident
rotates). They can re-yank a version the owner restored, or (by the same message construction, read from code and not exercised) re-grant an org role the owner later removed.
That contradicts "an auth session alone can never change a release state". Only `/machines/revoke` is
challenge-bound ("a signature can neither be replayed nor redirected").

UNCLEAR (owner decision): either bind these messages to a server challenge or a monotonic per-account
sequence (as `/machines/revoke` and the v2 transfer terms do), or document that replays of a
previously authorized tuple are accepted by design.

References:

- src/docs/spec/package-manager/01_repository-protocol.md "Release States": "Authority is the **ident key**: an auth session alone can never change a release state."; "Accounts — Orgs, Publish Tokens, Transfers": "every account mutation is ident-authorized and logged".
- 03_signing.md signing-domain table (`mfb-repo-release-state-v1`, `mfb-repo-org-role-v1`, `mfb-repo-token-revoke-v1`).

## Failing Reproduction

Python harness /tmp/hardening/B16/py/t_rs.py (alice's local ident key + a session):

```
y = sign(ident, "mfb-repo-release-state-v1\0alice#toolbox\x000.7.0\0yanked")   # captured once
POST /release-state state=available (fresh signature)       -> 200, index 29
POST /release-state state=yanked identSignature=y (replay)   -> 200, index 30
POST /release-state state=yanked identSignature=y (again)    -> 200, index 31 (same leaf hash as 30)
```

- Observed: every replay is accepted and logged.
- Expected (if replay protection is intended): the replayed signature is refused.

## Root Cause

`repository/src/crypto.rs:release_state_message`, `org_role_message`, `token_revoke_message` carry
no freshness. `server.rs:release_state`/`org_members`/`revoke_token` verify only the signature.

## Goal

- Decide on and document the replay model. If protection is wanted, add a challenge or sequence term.

## Blast Radius

- release-state, org-role grant/remove, token issue/revoke. Transfers (v2 sequence + digest) and machine revoke (challenge) are already bound.
