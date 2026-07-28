# bug-272: `mfb repo register` overwrites-then-deletes an owner's existing local keys on any failure (account lockout)

Last updated: 2026-07-18
Effort: small (<1h)
Severity: HIGH
Class: Correctness (data-loss footgun)

Status: Fixed 2026-07-18
Regression Test: `repository/src/client.rs` —
`client::tests::register_refuses_and_preserves_keys_when_owner_keys_already_exist`

## Resolution

`register` now checks for an existing auth/ident private key for `owner` before it
generates or writes anything, and returns an actionable error pointing at
`mfb repo auth` / `mfb repo link`. The guard sits ahead of `ensure_server_key`, so
the refusal costs no network round trip and cannot reach the truncating writers or
the `remove_owner_keys` error path.

Cleanup after a failed registration for a genuinely *new* owner is unchanged — the
rule enforced is only that `register` never deletes keys it did not create.

Verified both directions: with the guard removed, the new test fails with the key
files deleted from disk (the reported lockout); with the guard in place it passes,
and the full `repository/` suite is green (131 + 8 tests).

`register` in `repository/src/client.rs` generates fresh auth+ident keypairs and
writes them to the local key store **before** the network request, using
overwrite-unconditional writers. If `POST /accounts/register` then fails — the
*guaranteed* outcome when the owner already exists ("owner already exists"), or
any transient connect/timeout error — the error path calls
`local::remove_owner_keys`, which deletes all four key files. The owner's
original ident private key is destroyed twice over (overwritten, then the
replacement deleted). Per plan-23 the ident key is the account authority (rotate,
revoke, publish, org/token ops all require it); if this machine is the only
holder, the account becomes permanently unrecoverable.

The single correct behavior a fix produces: running `register` for an owner whose
keys already exist locally must not touch those keys — it must refuse up front and
direct the user to `auth`/`link`. A failed registration for a *new* owner may
still clean up the keys it just created (that is correct), but it must never
delete pre-existing keys it did not create this call.

References:

- `planning/old-plans/plan-23-key-trust-model.md` (ident key = account authority)
- Found during goal-06 full source review (repository/src/client.rs).

## Failing Reproduction

```
# Machine already holds keys for `alice` (prior register or `mfb repo link`).
mfb repo register alice        # against a registry where alice already exists
```

- Observed: command errors with "owner already exists"; `~/.mfb`/keys for `alice`
  (auth.pub/prv, ident.pub/prv) are gone afterward.
- Expected: command refuses before generating/writing anything; existing `alice`
  keys remain intact.

Contrast case that is correct: registering a genuinely new owner that fails
mid-flight — deleting the just-created keys is fine because they were never valid.

## Root Cause

`repository/src/client.rs:148-157` (`register`): writes keypairs via
`local::write_auth_keypair` / `write_ident_keypair` (both call
`write_private_file`, which truncates/creates unconditionally — `local.rs:159-193`)
*before* `post_json`, and on any `Err` calls `local::remove_owner_keys`
(`local.rs:195-200`), which `fs::remove_file`s all four paths regardless of who
created them. There is no pre-check that keys already exist and no staging of the
new keys under temp names. `src/cli/repo.rs` calls straight through with no guard.

## Goal

- `register` returns an error without mutating the key store when
  `paths.ident_private_key_path(owner)` (or the auth private key) already exists.
- On failure for a new owner, only keys created by this call are removed.

### Non-goals (must NOT change)

- The success path and on-disk key file format/permissions.
- The legitimate cleanup of keys created by a failed *new-owner* registration.
- Do not "fix" this by making `remove_owner_keys` silent about missing files —
  that is already its behavior and is not the bug.

## Blast Radius

- `client.rs:register` — fixed by this bug.
- `client.rs:rotate_ident` / other key-writing flows — separate durability
  ordering issue, tracked in bug-276 (LOW cluster); not this bug.
- `local::write_*_keypair` / `remove_owner_keys` — shared writers; leave behavior
  as-is, gate at the caller.

## Fix Design

In `register`, before generating keys, check whether the auth or ident private
key path for `owner` exists; if so, return an actionable error ("keys for
'{owner}' already exist locally; use `mfb repo auth` or `mfb repo link`, or remove
them first"). Alternative (heavier): stage new keys under `*.next` temp names and
promote only after the server accepts — rejected as unnecessary for the common
case, but noted for the rotate-ordering sibling in bug-276.

## Phases

### Phase 1 — failing test + audit
- [ ] Add a test that pre-populates `alice` keys, runs `register` against a stub
      that returns an error, and asserts the four key files are unchanged.
- [ ] Confirm the current code deletes them (test fails today).

### Phase 2 — the fix
- [ ] Add the existence guard at the top of `register`.

### Phase 3 — validation
- [ ] Full `repository/` test suite green; manual repro no longer loses keys.

## Validation Plan

- Regression test: register-over-existing-keys in `repository/tests`.
- Runtime proof: the repro above leaves keys intact and prints an actionable error.
- Doc sync: none expected (behavior becomes safer, matches plan-23 intent).

## Summary

One-line guard fixes a HIGH-severity, easily-triggered account-lockout footgun;
the risk is confined to `register`'s pre-request key writes.
