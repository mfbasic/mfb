# bug-493: `/machines/revoke` accepts an auth-key challenge — any auth key can revoke every auth key on the account

Last updated: 2026-09-03
Effort: medium (3h–1d)
Severity: HIGH
Class: security (authorization bypass / denial of service — account lockout)

Status: FIXED (found in audit-3, Surface 8 REPO-02; reproduced live by the lead; re-reproduced and fixed 2026-09-03)

Regression Test: `repository/src/server.rs` — `machine_revocation_is_ident_authorized_and_kills_the_session` (an `/auth/challenge`-minted challenge, revocation message signed by that auth key, is refused at `/machines/revoke` and not burned; an ident challenge is refused at `/auth/login`; revoking the last machine key is refused with a valid ident signature). `repository/src/store.rs` — `revocation_challenge_requires_the_ident_key` (purpose binding both ways at the store) and `linked_machine_key_works_and_revocation_kills_sessions` (last-key guard; a publish token does not count as a machine and may itself still be revoked).

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

## STATUS: FIXED

Landed on `worktree-B-492` (shared with bug-492/494), merged to `main`.

**Mechanism confirmed live before the fix** (`spikes/audit-3/repository-authz`,
`revoke`, against the pre-fix `mfb-repo`):

```
ident private key is NEVER used below
revoke status = 200 OK
revoke body   = {"owner":"victimpre1","authFingerprint":"b0f12c82…","revoked":true}
primary key still usable for login? false            # a secondary auth key killed the primary
```

**What changed:**

- `auth_challenges.purpose` (`'login'` | `'revoke'`, `store.rs`), set by
  `create_challenge_for_key` from its caller — `/auth/challenge` mints `login`,
  `/machines/revoke/challenge` mints `revoke` — and added to legacy databases by
  `add_column_if_missing` with the `'login'` default (every pre-existing row was
  minted by `/auth/challenge`).
- `complete_challenge_with` takes the expected purpose **and** key role and
  checks both — before the signature is verified and before the row is burned —
  so `/machines/revoke` refuses a login challenge (`"challenge was not issued for
  revocation"`) and `/auth/login` refuses an ident challenge (`"...for login"`),
  and a challenge presented to the wrong completer stays usable for its own.
- `revoke_auth_key` refuses, inside its transaction, to revoke the account's
  last current *machine* key (`"cannot revoke the account's last auth key; link
  another machine first"`). Publish tokens are `auth` rows but do not count as
  machines (they cannot enrol one after bug-492), so an account left with only a
  token is treated as locked out too; a token itself may still be revoked here.
  `/machines/revoke` reports that refusal as a 400, not a 500.

**After the fix** (same harness, post-fix binary):

```
revoke status = 400 Bad Request
revoke body   = {"error":"challenge was not issued for revocation"}
primary key still usable for login? true
```

**Wire contract:** no request or response shape changed on `/auth/challenge`,
`/machines/revoke/challenge`, or `/machines/revoke`; nonce, single-use, and
expiry checks are untouched (the purpose/role checks precede them). The
pre-existing happy path — the ident key revoking a linked machine and killing its
session — is still asserted, now with a second machine present so the revoked
key is not the last.

**Gates:** `cargo test --no-fail-fast` in `repository/` green (320 lib + 21 bin);
`cargo check --all-targets` 0 warnings; root `tests/cli_repo_*` acceptance suite
from a detached worktree at the landed commit — see the landing report.
