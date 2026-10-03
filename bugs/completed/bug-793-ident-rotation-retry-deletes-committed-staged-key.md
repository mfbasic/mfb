# bug-793: Retrying an uncertain ident rotation destroys the committed private key

Last updated: 2026-10-03
Effort: large (3h–1d)
Severity: HIGH
Class: Correctness
Status: Fixed
Regression Test: `repository/src/server.rs:tests::rotation_retry_preserves_committed_staged_key` (+ `rotation_retry_finishes_an_interrupted_promotion`, `rotation_retry_replaces_an_uncommitted_staging`, `rotation_retry_keeps_the_staging_when_the_registry_holds_another_key`)

Retrying a rotation whose server commit succeeded but whose response and
reconciliation were lost must preserve and recover the staged authoritative
ident key. Today `rotate_ident` overwrites the pending pair before checking it.
The old live key signs a second rotation, which the server rejects. Reconciliation
then discards the replacement pending pair, leaving no copy of the key the
server actually trusts. A sole-holder account loses its signing authority.

The reproduction constructs the exact persisted state after a committed but
unacknowledged rotation, then invokes the real client against a real local
registry. It does not simulate the retry HTTP routes.

References:

- `bugs/completed/bug-276-repository-low-cluster.md` — related earlier work; this reproduction covers the remaining path.
- `src/docs/spec/package-manager/01_repository-protocol.md` — current observable
  behavior; no ticket references are embedded in the spec.

## Failing Reproduction

Run from the repository root. The probe adds a new test only to a disposable
HEAD worktree; it neither edits nor weakens an existing test.

```sh
git worktree add --detach /tmp/mfb-spec-repro HEAD
# Save the Rust block below as /tmp/mfb-spec-probe.rs.
python3 - <<'PROBE'
from pathlib import Path
p = Path('/tmp/mfb-spec-repro/repository/src/server.rs')
s = p.read_text()
i = s.rfind('}')
p.write_text(s[:i] + Path('/tmp/mfb-spec-probe.rs').read_text() + s[i:])
PROBE
CARGO_TARGET_DIR=/tmp/mfb-spec-repro-target cargo test \
  --manifest-path /tmp/mfb-spec-repro/repository/Cargo.toml \
  --lib spec_probe_rotation_retry_preserves_committed_staged_key -- --nocapture
```

```rust
    #[tokio::test]
    async fn spec_probe_rotation_retry_preserves_committed_staged_key() {
        use crate::local::{self, LocalPaths};
        let h = harness();
        let alice = register_owner_with_all_keys(&h.store, "alice");
        let token = open_session(&h.store, "alice", &alice.auth_private);
        let dir = tempfile::tempdir().unwrap();
        let paths = LocalPaths::new(dir.path().to_owned());
        local::write_ident_keypair(&paths,"alice",&alice.ident_public,&alice.ident_private).unwrap();
        local::write_session(&paths,"alice",&token).unwrap();
        let (new_pub,new_prv)=crypto::generate_keypair();
        local::write_pending_ident_keypair(&paths,"alice",&new_pub,&new_prv).unwrap();
        let chain = crypto::sign(&alice.ident_private,&crypto::ident_rotation_message("alice",&crypto::fingerprint(&alice.ident_public),&new_pub)).unwrap();
        let possession = crypto::sign(&new_prv,&crypto::registration_message(crypto::ROLE_IDENT,"alice",&new_pub)).unwrap();
        h.store.rotate_ident("alice",&new_pub,&chain,&possession).unwrap();
        // Exact persisted state after the server commits but response and reconciliation are lost.
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url=format!("http://{}",listener.local_addr().unwrap());
        let app=build_router(h.state.clone());
        let server=tokio::spawn(async move { axum::serve(listener,app.into_make_service_with_connect_info::<SocketAddr>()).await.unwrap(); });
        let retry_paths=paths.clone();
        let result=tokio::task::spawn_blocking(move || crate::client::rotate_ident(&url,&retry_paths,"alice")).await.unwrap();
        server.abort();
        eprintln!("retry result: {result:?}");
        let live=local::read_ident_private_key(&paths,"alice").ok();
        let staged=std::fs::read_to_string(paths.ident_pending_private_key_path("alice")).ok()
            .and_then(|s|crypto::decode_bytes(s.trim(),"pending").ok());
        assert!(live.as_ref()==Some(&new_prv)||staged.as_ref()==Some(&new_prv),"committed ident private key was lost on retry");
    }
```

Observed at HEAD `b285741f3` in a detached worktree on macOS, using
`cargo test --lib spec_probe_ -- --nocapture` (manifest path and target directory
pointed at `/tmp/mfb-package-spec-head` and `/tmp/mfb-package-spec-target`):

```text
retry result: Err("invalid ident chain signature")
committed ident private key was lost on retry
```

Expected: the assertion passes. Other platforms have not been measured.

## Root Cause

- `repository/src/client.rs:rotate_ident` generates and writes a fresh pending
  pair unconditionally; it never calls `read_pending_ident_public_key` first.
- `repository/src/local.rs:write_pending_ident_keypair` truncates both pending
  paths through `write_private_file`.
- After the old-key-signed retry fails, `reconcile_pending_ident` compares the
  server's current key with the newly generated key, not the overwritten one.
  They differ, so it removes the staging files.
- `client::auth` also does not recover the pending ident. The error text advising
  `mfb repo rotate` names an unavailable command; the actual CLI is `mfb key rotate`.

## Goal

Retrying a rotation whose server commit succeeded but whose response and
reconciliation were lost must preserve and recover the staged authoritative
ident key. Today `rotate_ident` overwrites the pending pair before checking it.
The old live key signs a second rotation, which the server rejects. Reconciliation
then discards the replacement pending pair, leaving no copy of the key the
server actually trusts. A sole-holder account loses its signing authority.

### Non-goals

Do not weaken existing tests, package signatures, identity checks, or ownership
checks. Do not silently regenerate goldens. This document files the code defect;
the accompanying spec review changes documentation only.

## Blast Radius

- Fix: `client::rotate_ident`, `reconcile_pending_ident`, and pending-key read /
  promotion helpers. Reconcile existing staging before generating anything.
- Audit: `promote_pending_ident_keypair` renames the public and private files
  separately; partial promotion must retain recoverable private material.
- Audit: `auth` and CLI rotation error guidance; ensure the advertised recovery
  command reaches the recovery logic.
- Unaffected: server rotation signatures and transaction checks are correct;
  first-time successful rotation should retain the same wire behavior.
- Other unconditional owner-key writers (`register`, `link_fetch`) need a
  collision audit for pending rotation material; no failure there was reproduced.

## Fix Design

Recover an existing pending rotation before reading/signing with a presumed
live pair or writing a new pending pair. If the registry confirms the pending
key, promote it safely; if registry state cannot be established, leave it intact
and return a recoverable error. Distinguish a proven rejected rotation from an
uncertain state. Preserve private material through partial promotion and process
interruption. Do not delete uncertain staging or overwrite it as a retry policy.

## Phases

### Phase 1 — failing tests and audit

- [x] Land the reproduction as a regression test and add contrast cases from the
      blast-radius audit.
- [x] Confirm the tests fail for the documented mechanism and audit all listed
      consumers before selecting the implementation.

Acceptance: reproducible RED tests and completed consumer audit.
Commit: —

### Phase 2 — implementation

- [x] Implement the design above and update the owning embedded specs.
- [x] Verify the reproduction and contrast cases without weakening assertions.

Acceptance: the target behavior holds and existing valid flows remain valid.
Commit: —

### Phase 3 — full verification

- [x] Run the full repository crate suite and root project suite.
- [x] Re-run CLI integration scenarios and the original reproduction.
- [x] Review any generated-output differences against the intended contract.

Acceptance: complete suites pass and the goal is demonstrated end to end.
Commit: —

## Validation Plan

The failing reproduction is the minimum regression. Extend it to exercise both
successful and failed operations, persisted state, and subsequent CLI behavior.
Verify the existing registry command integration tests and signed-package
verification flows before considering the fix complete.

## Resolution (2026-10-03)

- `client::recover_pending_ident` replaces `reconcile_pending_ident` and gives a
  three-way verdict against the registry's current ident: promote if it equals the
  staged key; discard only if it equals this machine's live key; in every other
  state (unreachable registry, or a third key) keep the staging and report it.
- `rotate_ident` calls it **before** reading the live key or writing a new
  staging. A committed-but-unacknowledged rotation is promoted and returned, a
  stale one is cleared and a fresh rotation proceeds, and an unresolved one fails
  without touching anything. The same verdict settles a failed rotation request.
- `auth` settles any staging after a successful login.
- The staged **private** key is the staging marker (`local::read_pending_ident_private_key`).
  `promote_pending_ident_keypair` rewrites the live public key from it, removes the
  pending public file, and moves the private key last, so an interrupted promotion
  is re-run to completion.
- The advertised retry command is now `mfb key rotate <owner>` (was the
  nonexistent `mfb repo rotate`).
- Specs: `mfb spec package-manager repository-protocol` (the deviation note is
  replaced by the contract) and `key-store` (promotion order).

Test correction (four questions):
`local::tests::promoting_without_a_staged_rotation_reports_the_missing_file`
(3ee3342ad, bug-347 coverage) protects "promoting with nothing staged fails,
names both ends of the move, and leaves the live keypair intact". Its two
path assertions pinned the pending *public* file as the rename source. That
order is disproved: the interrupted-promotion case above shows that a promotion
cut off after the public rename leaves no pending public file, so it cannot be
the marker. Only those two lines now name the private-key paths. The error shape
and the intact-keypair assertions are unchanged.

Proof: on main's client, 3 of the 4 new tests fail
(`rotation_retry_replaces_an_uncommitted_staging` is the contrast case and passes
on both). With the fix, `cargo test` in `repository/` passes 407/407, and
`cli_repo_auth` and `cli_repo_governance` pass.
