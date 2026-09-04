# bug-494: `/release-state` and `/signing` authorize on the ident name prefix, not the package owner — a former owner keeps yank + attestation rights after a transfer

Last updated: 2026-09-03
Effort: medium (3h–1d)
Severity: HIGH
Class: security (authorization bypass — stale ownership)

Status: FIXED (found in audit-3, Surface 8 REPO-03; reproduced live by the lead; re-reproduced and fixed 2026-09-03)

Regression Test: `repository/src/server.rs` — `a_transferred_package_is_governed_by_its_current_owner` (after a real two-sided transfer the former owner is refused at `/release-state` and `/signing`, the current owner succeeds at both, and first-publish `/signing` for a package with no row still follows the ident prefix). `repository/src/store.rs` — `set_release_state_rejects_unpublished_and_updates_published` (the `owner_id` predicate refuses a foreign account at the store, logging nothing).

## Summary

`/release-state` (yank/unyank) and `/signing` (registry attestation) authorize by
comparing the ident's own `owner#` prefix to the session owner. That prefix never
moves when ownership does. So after a correctly-guarded package transfer, the
**former** owner can still yank the package (a permanent, un-revertable denial of
a package they handed over) and still obtain a registry attestation for it, while
the **new** owner is locked out of both. The public read surface
(`/index/<ident>`) also still names the former owner as the signed trust anchor.

## Mechanism

```rust
// repository/src/server.rs:2046 — release_state, the only package-level check
let Some((ident_owner, package_part)) = request.ident.split_once('#') else { … };
if package_part.is_empty()
    || crate::validation::fold_owner(ident_owner)
        != crate::validation::fold_owner(&request.owner)
{ return Err(bad_request("ident owner does not match session owner")); }
```

```rust
// repository/src/store.rs:2544 — resolves the row by ident alone, no owner predicate
"SELECT pv.id FROM package_versions pv
 JOIN packages p ON p.id = pv.package_id
 WHERE p.ident = ?1 AND pv.version = ?2"
```

`packages.owner_id` — which `accept_transfer` (`store.rs:2374`) rewrites and
which `publish_package_version` (`store.rs:1703`) *does* check — is never
consulted. `/signing` has the identical prefix-only test at `server.rs:2630`. The
result is symmetric: the new owner also fails the prefix test.

## Reproduction (lead-run, live, 2026-09-03)

`spikes/audit-3/repository-authz/` — `cargo run --bin transfer -- t2b`
(uses the store's own transfer path to move `alicet2b#widget` to `bobt2b`):

```
package owner after transfer = Some("bobt2b")
former-owner yank status = 200 OK                    # alicet2b yanks a package she gave away
former-owner /signing status = 200 OK                # ...and still gets a signed attestation
current-owner unyank status = 400 Bad Request        # bobt2b, the real owner, is locked out
```

Expected: 403/400 for the former owner, success for the current one.

## Best fix

Authorize on the resource. In `release_state` and `signing`, resolve
`store.package_owner(&request.ident)` and require
`package_owner.id == claims.owner_id`, falling back to the ident-prefix check
only when no `packages` row exists yet (the first-publish case `/signing` needs).
Add the same `owner_id` predicate to `set_release_state`'s `WHERE` so the store is
safe independently of the handler, mirroring `publish_package_version`. The read
surface (`package_index`/`package_detail`) should derive `owner`/`identKey` from
`packages.owner_id` too, so the signed name binding follows the transfer.

## Non-goals

- Do not change the `<owner>#<package>` ident format or make an ident mutable —
  already-published artifacts verify against the ident string they were signed
  with.
- Do not break first-publish, where no `packages` row exists yet.

## Prior art

None for the authorization gap. `bugs/completed/bug-274` (cited in
`store.rs:2287,2352`) hardened the *transfer handshake* against stale/racing
offers; it did not touch what a former owner may still do afterwards. Searched
`set_release_state`, `package_owner`, `accept_transfer`, "transfer", "yank"
across `bugs/`, `bugs/completed/`, `bugs/skipped/`, `audit-1-repository.md`,
`audit-2-repository.md`.

## STATUS: FIXED

Landed on `worktree-B-492` (shared with bug-492/493), merged to `main`.

**Mechanism confirmed live before the fix** (`spikes/audit-3/repository-authz`,
`transfer`, against the pre-fix `mfb-repo`):

```
package owner after transfer = Some("bobpre1")
former-owner yank status = 200 OK
former-owner /signing status = 200 OK
current-owner unyank status = 400 Bad Request   {"error":"ident owner does not match session owner"}
```

**What changed:**

- `require_package_owner` (`server.rs`) authorizes on `packages.owner_id`: when a
  row exists its owner must be the session owner (`"package is not owned by the
  session owner"`); only when no row exists yet does the ident's `owner#` prefix
  decide, which keeps first publish working and the error shape for a foreign or
  malformed ident unchanged. `release_state` and `signing` both call it.
- `set_release_state(owner_id, ident, version, state)` resolves the version
  under `AND p.owner_id = ?3`, mirroring `publish_package_version`, so the store
  refuses a former owner even if a handler forgot to; the message is
  `"... is not published by this account"` and nothing is logged.
- `publish_aborts_the_staged_blob_when_the_ident_moved_owner` (bug-347 coverage
  of the blob-abort path) obtained its `/signing` attestation *after* the
  transfer — exactly this bug. Only that setup line moved (attest before the
  transfer; the stale-attestation shape); the protected behavior — a store-refused
  publish aborts the staged blob — is asserted unchanged. Evidence: written in
  d0e3962b8 for coverage, no other dependents, disproved by the live repro above.

**Not changed, deliberately:** the read surface (`/index/<ident>`,
`package_detail`) still derives `owner`/`identKey` from the ident prefix. An
already-published version verifies against the ident key it was signed with, so
re-pointing `identKey` at the new owner would break verification of every
pre-transfer version; following the transfer on the read side needs per-version
identity binding, which is a separate change, not an authorization fix.

**After the fix** (same harness, post-fix binary):

```
former-owner yank status = 400 Bad Request      {"error":"package is not owned by the session owner"}
former-owner /signing status = 400 Bad Request  {"error":"package is not owned by the session owner"}
current-owner unyank status = 200 OK            {"ident":"alicepost4#widget","version":"1.0.0","state":"available",...}
```

**Wire contract:** no request or response shape changed; a legitimate transfer
(`ownership_transfer_is_two_sided_and_rebinds_the_package`) and first-publish
`/signing` are asserted.

**Gates:** `cargo test --no-fail-fast` in `repository/` green (321 lib + 21 bin);
`cargo check --all-targets` 0 warnings; root `tests/cli_repo_*` acceptance suite
from a detached worktree at the landed commit — see the landing report.
