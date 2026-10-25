//! `/validate` and `/publish` (`server/publish.rs`) on the refusals the other
//! publish suites leave out: the per-owner version quota, the validate budget,
//! a package whose ident has no owner, a session whose claims disagree with its
//! key, a publisher with no current ident key, a malformed header identKey, a
//! payload or signature tampered after signing, a publish whose version row
//! cannot be written over an already-stored blob, and the typosquat warning.

use super::*;
use crate::store::tests::{refuse_writes, tamper};

const PAYLOAD: &[u8] = b"MFPCtestpayload";

/// The wire request describing `artifact`, as the CLI builds it.
fn request_for(artifact: &[u8], token: &str) -> PackageArtifactRequest {
    let parsed = package::parse_mfp_package(artifact).expect("crafted package parses");
    PackageArtifactRequest {
        ident: parsed.ident.clone(),
        version: parsed.version.clone(),
        artifact: crypto::encode_bytes(artifact),
        content_hash: parsed.content_hash_hex(),
        ident_fingerprint: parsed.ident_fingerprint().unwrap_or_default(),
        signing_fingerprint: parsed.signing_fingerprint().unwrap_or_default(),
        session_token: token.to_string(),
    }
}

async fn validate(h: &Harness, request: PackageArtifactRequest) -> ValidatePackageResponse {
    validate_package(State(h.state.clone()), Json(request))
        .await
        .expect("validate answers with a report")
        .0
}

fn alice(h: &Harness) -> (TestOwnerKeys, String) {
    let keys = register_owner_with_all_keys(&h.store, "alice");
    let token = open_session(&h.store, "alice", &keys.auth_private);
    (keys, token)
}

#[tokio::test]
async fn an_owner_at_the_version_quota_cannot_publish_a_new_version() {
    let h = harness();
    let (keys, token) = alice(&h);
    publish_valid_package(&h.state, &keys, &token, "1.0.0").await;
    // Fill alice's quota with rows of another package she owns.
    tamper(
        &h.store,
        &format!(
            "INSERT INTO packages (ident, owner_id, created_at)
                 SELECT 'alice#filler', owner_id, 0 FROM packages WHERE ident = 'alice#toolbox';
             WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < {})
             INSERT INTO package_versions (package_id, version, hash, state, created_at)
                 SELECT (SELECT id FROM packages WHERE ident = 'alice#filler'),
                        'v' || i, 'h' || i, 'available', 0 FROM n;",
            MAX_VERSIONS_PER_OWNER - 1
        ),
    );
    assert_eq!(
        h.store
            .owner_version_count(h.store.owner_with_ident_key("alice").unwrap().unwrap().0.id)
            .unwrap(),
        MAX_VERSIONS_PER_OWNER
    );
    let (_artifact, request) =
        signed_request(&h.state, &keys, &token, "2.0.0", PAYLOAD.to_vec()).await;
    let (status, message) = err_of(publish_package(State(h.state.clone()), Json(request)).await);
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        message,
        format!("per-owner version quota of {MAX_VERSIONS_PER_OWNER} reached")
    );
    // Refused before anything is staged.
    assert!(!h.packages_dir.read_dir().unwrap().any(|entry| entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .contains("tmp")));
}

#[tokio::test]
async fn validate_refuses_an_owner_past_its_budget() {
    let h = harness();
    let (keys, token) = alice(&h);
    let (_artifact, request) =
        signed_request(&h.state, &keys, &token, "1.0.0", PAYLOAD.to_vec()).await;
    for _ in 0..VALIDATE_PER_OWNER_MAX {
        assert!(h
            .state
            .rate_limiter
            .allow("validate:alice", VALIDATE_PER_OWNER_MAX, 60));
    }
    let (status, _message) = err_of(validate_package(State(h.state.clone()), Json(request)).await);
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn a_package_ident_with_no_owner_part_is_an_invalid_report() {
    let h = harness();
    let (keys, token) = alice(&h);
    let (signing_public, signing_private) = crypto::generate_keypair();
    let artifact = package::test_support::serialize(
        &package::test_support::TestPackage {
            name: "toolbox".to_string(),
            ident: "toolbox".to_string(),
            version: "1.0.0".to_string(),
            author: "alice".to_string(),
            url: String::new(),
            payload: PAYLOAD.to_vec(),
            ident_key: format!("ed25519:{}", crypto::encode_bytes(&keys.ident_public)),
            signing_key: format!("ed25519:{}", crypto::encode_bytes(&signing_public)),
            proof: "{}".to_string(),
            proof_sig: vec![0; 64],
            attestation: "{}".to_string(),
            attestation_sig: vec![0; 64],
        },
        &signing_private,
    );
    let report = validate(&h, request_for(&artifact, &token)).await;
    assert!(!report.valid);
    assert_eq!(
        report.diagnostics,
        vec!["package ident must use <owner>#<package>".to_string()]
    );
}

#[tokio::test]
async fn a_session_whose_claims_name_another_account_than_its_key_is_refused() {
    let h = harness();
    let (keys, token) = alice(&h);
    publish_valid_package(&h.state, &keys, &token, "1.0.0").await;
    register_owner_with_all_keys(&h.store, "bob");
    let (alice_owner, alice_key) = h.store.owner_with_auth_key("alice").unwrap().unwrap();
    let bob_id = h.store.owner_with_ident_key("bob").unwrap().unwrap().0.id;
    // Signed by the server and backed by a live session row, but its owner id
    // is bob's while its subject and key are alice's.
    let now = crate::store::now_unix();
    let forged = session_with_claims(
        &h.store,
        alice_owner.id,
        alice_key.id,
        SessionClaims {
            sub: "alice".to_string(),
            owner_id: bob_id,
            auth_fingerprint: alice_key.fingerprint.clone(),
            iat: now,
            exp: now + 600,
            jti: Uuid::new_v4().to_string(),
            iss: SESSION_TOKEN_ISSUER.to_string(),
            aud: SESSION_TOKEN_AUDIENCE.to_string(),
        },
    );
    let (_artifact, mut request) =
        signed_request(&h.state, &keys, &token, "2.0.0", PAYLOAD.to_vec()).await;
    request.session_token = forged;
    let report = validate(&h, request).await;
    assert!(!report.valid);
    assert!(
        report
            .diagnostics
            .contains(&"session key does not match current owner key".to_string()),
        "{:?}",
        report.diagnostics
    );
}

#[tokio::test]
async fn a_publisher_with_no_current_ident_key_is_refused() {
    let h = harness();
    let (keys, token) = alice(&h);
    publish_valid_package(&h.state, &keys, &token, "1.0.0").await;
    let (_artifact, request) =
        signed_request(&h.state, &keys, &token, "2.0.0", PAYLOAD.to_vec()).await;
    tamper(
        &h.store,
        "UPDATE keys SET status = 'past' WHERE role = 'ident'",
    );
    let report = validate(&h, request).await;
    assert!(!report.valid);
    assert!(
        report
            .diagnostics
            .contains(&"owner has no current ident key".to_string()),
        "{:?}",
        report.diagnostics
    );
}

#[tokio::test]
async fn a_header_ident_key_that_does_not_decode_is_refused() {
    let h = harness();
    let (_keys, token) = alice(&h);
    let (signing_public, signing_private) = crypto::generate_keypair();
    let artifact = package::test_support::serialize(
        &package::test_support::TestPackage {
            name: "toolbox".to_string(),
            ident: "alice#toolbox".to_string(),
            version: "1.0.0".to_string(),
            author: "alice".to_string(),
            url: String::new(),
            payload: PAYLOAD.to_vec(),
            ident_key: "ed25519:not base64!".to_string(),
            signing_key: format!("ed25519:{}", crypto::encode_bytes(&signing_public)),
            proof: "{}".to_string(),
            proof_sig: vec![0; 64],
            attestation: "{}".to_string(),
            attestation_sig: vec![0; 64],
        },
        &signing_private,
    );
    let report = validate(&h, request_for(&artifact, &token)).await;
    assert!(!report.valid);
    let expected = package::decode_metadata_key("ed25519:not base64!", "identKey").unwrap_err();
    assert!(
        report.diagnostics.contains(&expected),
        "{expected}: {:?}",
        report.diagnostics
    );
}

#[tokio::test]
async fn a_payload_or_signature_changed_after_signing_is_refused() {
    let h = harness();
    let (keys, token) = alice(&h);
    let (artifact, _request) =
        signed_request(&h.state, &keys, &token, "1.0.0", PAYLOAD.to_vec()).await;

    // A payload byte flipped: the header's payload hash no longer welds it.
    let mut payload_changed = artifact.clone();
    *payload_changed.last_mut().unwrap() ^= 0x01;
    let report = validate(&h, request_for(&payload_changed, &token)).await;
    assert!(!report.valid);
    let expected =
        package::verify_payload_hash(&package::parse_mfp_package(&payload_changed).unwrap())
            .unwrap_err();
    assert!(
        report.diagnostics.contains(&expected),
        "{expected}: {:?}",
        report.diagnostics
    );

    // A signature byte flipped: the header no longer verifies.
    let mut signature_changed = artifact.clone();
    let signature_at = artifact.len() - PAYLOAD.len() - 64;
    signature_changed[signature_at] ^= 0x01;
    let report = validate(&h, request_for(&signature_changed, &token)).await;
    assert!(!report.valid);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.starts_with("package signature verification failed: ")),
        "{:?}",
        report.diagnostics
    );
}

#[tokio::test]
async fn a_publish_whose_version_row_fails_over_a_stored_blob_keeps_the_blob() {
    let h = harness();
    let (keys, token) = alice(&h);
    let (artifact, request) =
        signed_request(&h.state, &keys, &token, "1.0.0", PAYLOAD.to_vec()).await;
    // The same bytes are already stored (an earlier, interrupted publish).
    let blob = h.packages_dir.join(format!("{}.mfp", request.content_hash));
    std::fs::write(&blob, &artifact).unwrap();
    refuse_writes(&h.store, "package_versions", "INSERT");
    let (status, message) = err_of(publish_package(State(h.state.clone()), Json(request)).await);
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        message,
        "failed to publish package version: injected INSERT failure"
    );
    // Nothing was staged, so nothing was aborted: the stored bytes remain.
    assert_eq!(std::fs::read(&blob).unwrap(), artifact);
}

#[tokio::test]
async fn a_publish_one_edit_from_an_existing_ident_carries_a_typosquat_warning() {
    let h = harness();
    let (keys, token) = alice(&h);
    let alice_id = h.store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
    h.store
        .publish_package_version(
            alice_id,
            "alice#toolbax",
            "1.0.0",
            "hash-toolbax",
            "data/toolbax.mfp",
            "{}",
            &[],
            &crate::store::PublishMetadata::default(),
        )
        .unwrap();
    let (_artifact, request) =
        signed_request(&h.state, &keys, &token, "1.0.0", PAYLOAD.to_vec()).await;
    let response = publish_package(State(h.state.clone()), Json(request))
        .await
        .expect("a near-duplicate ident still publishes")
        .0;
    assert_eq!(
        response.warnings,
        vec![
            "published `alice#toolbox` is one edit away from existing `alice#toolbax`".to_string()
        ]
    );
}
