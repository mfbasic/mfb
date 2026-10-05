# bug-792: A transferred package cannot receive a new release from its recipient

Last updated: 2026-10-04
Effort: large (3h–1d)
Severity: HIGH
Class: Correctness
Status: Fixed (plan-168 A–D, 2026-10-04)
Regression Test: `repository/src/server.rs:tests::spec_probe_transfer_recipient_can_publish` (the reproduction below, with the transfer routed through the real signed handlers); end to end `tests/cli/cli_repo_governance.rs:repo_transfer_recipient_publishes_and_consumers_verify_both_owners`

A successful transfer must let the recipient build, attest, publish, and install a
new release under the unchanged package ident while preserving verification of
pre-transfer releases. Today ownership and attestation move to the recipient,
but build, publish validation, proof verification, and index identity still use
the original ident prefix. The transfer therefore strands further publication.

Found while checking the package-manager spec against the implementation.
The existing transfer authorization fix covers signing and release-state only;
its completed record explicitly leaves read-side identity binding unchanged.

References:

- `bugs/completed/bug-494-registry-release-state-signing-prefix-authz.md` — related earlier work; this reproduction covers the remaining path.
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
  --lib spec_probe_transfer_recipient_can_publish -- --nocapture
```

```rust
    #[tokio::test]
    async fn spec_probe_transfer_recipient_can_publish() {
        let h = harness();
        let alice = register_owner_with_all_keys(&h.store, "alice");
        let bob = register_owner_with_all_keys(&h.store, "bob");
        let alice_token = open_session(&h.store, "alice", &alice.auth_private);
        let bob_token = open_session(&h.store, "bob", &bob.auth_private);
        publish_valid_package(&h.state, &alice, &alice_token, "1.0.0").await;
        h.store.create_transfer_offer("alice#toolbox", "alice", "bob").unwrap();
        h.store.accept_transfer("alice#toolbox", "bob").unwrap();
        assert_eq!(h.store.package_owner("alice#toolbox").unwrap().unwrap().owner_display, "bob");
        let (pubkey, private) = crypto::generate_keypair();
        let idfp = crypto::fingerprint(&bob.ident_public);
        let signfp = crypto::fingerprint(&pubkey);
        let attestation = signing(State(h.state.clone()), peer("127.0.0.1"), Json(SigningRequest {
            owner: "bob".into(), ident: "alice#toolbox".into(), version: "2.0.0".into(),
            signing_fingerprint: signfp.clone(), session_token: bob_token.clone(),
        })).await.unwrap().0;
        let proof = serde_json::json!({"owner":"bob", "ident":"alice#toolbox", "version":"2.0.0",
            "identFingerprint":idfp, "signingFingerprint":signfp, "issued":now_unix()}).to_string();
        let proof_sig = crypto::sign(&bob.ident_private, &crypto::proof_signing_input(proof.as_bytes())).unwrap();
        let artifact = package::test_support::serialize(&package::test_support::TestPackage {
            name:"toolbox".into(), ident:"alice#toolbox".into(), version:"2.0.0".into(), author:"bob".into(),
            url:String::new(), payload:b"MFPCtestpayload".to_vec(),
            ident_key:format!("ed25519:{}",crypto::encode_bytes(&bob.ident_public)),
            signing_key:format!("ed25519:{}",crypto::encode_bytes(&pubkey)), proof, proof_sig,
            attestation:attestation.attestation,
            attestation_sig:crypto::decode_bytes(&attestation.attestation_signature,"sig").unwrap(),
        }, &private);
        let parsed = package::parse_mfp_package(&artifact).unwrap();
        let report = validate_package(State(h.state.clone()), Json(PackageArtifactRequest {
            ident:parsed.ident.clone(), version:parsed.version.clone(), artifact:crypto::encode_bytes(&artifact),
            content_hash:parsed.content_hash_hex(), ident_fingerprint:idfp, signing_fingerprint:signfp,
            session_token:bob_token,
        })).await.unwrap().0;
        assert!(report.valid, "recipient validation failed: {:?}",report.diagnostics);
    }
```

Observed at HEAD `b285741f3` in a detached worktree on macOS, using
`cargo test --lib spec_probe_ -- --nocapture` (manifest path and target directory
pointed at `/tmp/mfb-package-spec-head` and `/tmp/mfb-package-spec-target`):

```text
recipient validation failed: ["session owner does not match package ident owner", "session key does not match current owner key"]
```

Expected: the assertion passes. Other platforms have not been measured.

## Root Cause

- `repository/src/server.rs:validate_package_request` compares the package ident
  prefix with the session and looks up that prefix owner's auth/ident keys.
- `src/cli/build/signing.rs:signing_ident` rejects a manifest ident whose prefix
  differs from the signing owner, before a recipient can use the CLI build flow.
- `repository/src/package.rs:verify_proof` and `verify_attestation` require the
  signed `owner` to equal the ident prefix, even for a transferred package.
- `repository/src/server.rs:package_index` resolves the prefix owner's ident key
  rather than a per-version publisher identity.
- `repository/src/server.rs:require_package_owner` and
  `repository/src/store.rs:accept_transfer` already use stored ownership. Their
  success is the contrast case: the recipient can get an attestation and change
  release state, but cannot validate the attested release.

## Goal

A successful transfer must let the recipient build, attest, publish, and install a
new release under the unchanged package ident while preserving verification of
pre-transfer releases. Today ownership and attestation move to the recipient,
but build, publish validation, proof verification, and index identity still use
the original ident prefix. The transfer therefore strands further publication.

### Non-goals

Do not weaken existing tests, package signatures, identity checks, or ownership
checks. Do not silently regenerate goldens. This document files the code defect;
the accompanying spec review changes documentation only.

## Blast Radius

- Fix in this work: `signing_ident`, `validate_package_request`, package proof and
  attestation verification, index identity binding, and their compiler-side
  consumers (`src/cli/pkg.rs`, `src/cli/resolve.rs`, build package classification).
- Preserve: `require_package_owner` rejects former-owner signing/release-state
  mutations; `publish_package_version` rejects publication by a non-owner.
- Preserve: unchanged-ident first publication, owner case folding, old artifact
  bytes, and offline verification of pre-transfer releases.
- Audit: `src/cli/info.rs:verify_chain` consumes the index identity for executable
  inspection too; establish its behavior before changing index response meaning.

## Fix Design

Introduce explicit, authenticated publisher identity per release and bind it to
transfer authority. Reuse stored package ownership for new publication while
keeping historical identity verifiable. Do not merely replace the index's key
with the recipient's key: that breaks old releases. Do not rename the package
or weaken signature/owner checks to make the test pass. The precise wire/pinning
change requires design before implementation.

## Phases

### Phase 1 — failing tests and audit

- [x] Land the reproduction as a regression test and add contrast cases from the
      blast-radius audit.
- [x] Confirm the tests fail for the documented mechanism and audit all listed
      consumers before selecting the implementation.

Acceptance: reproducible RED tests and completed consumer audit.
  Measured: `cargo test -p mfb_repository --lib -- spec_probe_transfer` failed
  with exactly `recipient validation failed: ["session owner does not match
  package ident owner", "session key does not match current owner key"]`
  (plan-168-B Phase 1). The consumer audit is plan-168-A §2 / plan-168-C §2; it
  also found a pre-existing rotation defect (a registry add of a pre-rotation
  release refused to install), landed RED as
  `repo_registry_add_of_a_pre_rotation_release_installs` and fixed by plan-168-C.
Commit: fab5bbea3 (probe + contrast), 27cff5fe2 (rotation RED)

### Phase 2 — implementation

- [x] Implement the design above and update the owning embedded specs.
- [x] Verify the reproduction and contrast cases without weakening assertions.

Acceptance: the target behavior holds and existing valid flows remain valid.
  Measured: the probe and its contrast cases pass (plan-168-B Phase 2, `11
  passed`); three existing tests changed one line each, with the AGENTS.md proof
  in their commits (`publish_aborts_the_staged_blob_when_the_ident_moved_owner`,
  `signing_ident_rejects_bad_idents`,
  `build_project_sign_rejects_a_foreign_manifest_ident` → `_malformed_`), and the
  v1-transfer tests were migrated to v2 with every assertion kept.
Commit: 0b3c5db49, fe5242d24 (A); ed3f34ab2, ffd19bbc8 (B); aa5b5b79f,
144cadc99, 352c4c63a (C); bb166a318 (D)

### Phase 3 — full verification

- [~] Run the full repository crate suite and root project suite. The
      repository crate: `cargo test -p mfb_repository --no-fail-fast` → `EXIT=0`
      (450 lib tests). The root project suite is run once for the batch by the
      coordinator's gate, not by this change.
- [x] Re-run CLI integration scenarios and the original reproduction.
      `cargo test --test cli_repo_install --test cli_repo_publish --test
      cli_repo_governance` → all pass (8 + 10 + 9 after plan-168-D), including
      the end-to-end transfer scenario, which fails at the build step with
      plan-168-D Phase 1 reverted.
- [x] Review any generated-output differences against the intended contract.
      No golden or generated artifact is touched by plan-168 (no codegen,
      runtime or `.mfp` byte change).

Acceptance: complete suites pass and the goal is demonstrated end to end.
Commit: ab3cc833c (end to end), 1ba768a08 (close-out)

## Resolution

A transfer is now a signed, append-only record that clients verify, and every
step that tied identity to the ident prefix follows it:

- **Record (plan-168-A).** Both halves sign v2 terms `(ident, sequence,
  fromOwner, fromIdentFingerprint, toOwner, releaseDigest)`, verified by the
  store inside its writing transaction; accepted links go to an append-only
  `transfer_history` (triggers abort UPDATE/DELETE); legacy transfers are
  replayed from the transparency log as unsigned links; each release records
  publisher, publisher key and epoch; `/index` serves `transfers` (with a
  server-signed `toBinding`) and `versions[].publisher` / `.epoch`; the CLI
  offers as the package's current owner.
- **Authorization (plan-168-B).** `/validate` and `/publish` authorize the
  stored owner (prefix only for a first publish, origin owner only past a legacy
  link); proof/attestation `owner` binds to the release author; token scopes
  follow the stored owner.
- **Trust (plan-168-C).** `repository/src/trust.rs` verifies a release's key for
  its publisher from the pin (or first use) through rotation chains and the
  signed links; `pkg add`, the resolver/lock, indirect dependencies,
  `pkg verify` pin-follow and `mfb info` use it, and pins move with a notice.
- **Build (plan-168-D).** `signing_ident` accepts any well-formed ident; the
  registry's `/signing` decides.

Owner follow-ups: the hosted registry's legacy-transfer count and the
`migrate` replay check against a copy of its database were not available to
this change (plan-168-A Corrections); the A-D2 ratification flow depends on
that count.

## Validation Plan

The failing reproduction is the minimum regression. Extend it to exercise both
successful and failed operations, persisted state, and subsequent CLI behavior.
Verify the existing registry command integration tests and signed-package
verification flows before considering the fix complete.
