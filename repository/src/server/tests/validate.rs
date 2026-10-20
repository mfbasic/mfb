//! plan-176-I: `server::tests` — the validate tests. The shared harnesses (owners, transfers,\n//! MFPC builders, signed requests) live in `server/tests/mod.rs`.

use super::*;

#[tokio::test]
async fn release_state_requires_ident_signature_and_is_served_and_logged() {
    let temp = tempfile::tempdir().unwrap();
    let opened =
        Store::open_repository(&temp.path().join("meta.db"), &temp.path().join("data")).unwrap();
    let store = opened.store;
    let keys = register_owner_with_all_keys(&store, "alice");
    let token = open_session(&store, "alice", &keys.auth_private);
    let state = AppState {
        store: store.clone(),
        blob_store: BlobStore::local(opened.packages_dir.clone()),
        rate_limiter: RateLimiter::new(),
    };
    publish_valid_package(&state, &keys, &token, "1.0.0").await;
    let log_before = store.log_size().unwrap();

    let sign_state = |new_state: &str| {
        crypto::encode_bytes(
            &crypto::sign(
                &keys.ident_private,
                &crypto::release_state_message("alice#toolbox", "1.0.0", new_state),
            )
            .unwrap(),
        )
    };

    // Auth session but no valid ident signature: refused (the ident key is
    // the authority, an auth session alone must not suffice).
    let refused = release_state(
        State(state.clone()),
        Json(ReleaseStateRequest {
            owner: "alice".to_string(),
            ident: "alice#toolbox".to_string(),
            version: "1.0.0".to_string(),
            state: "deprecated".to_string(),
            session_token: token.clone(),
            ident_signature: crypto::encode_bytes(&[0u8; 64]),
        }),
    )
    .await;
    assert!(refused.is_err());

    // An operator-only state is refused even with a valid ident signature.
    let refused = release_state(
        State(state.clone()),
        Json(ReleaseStateRequest {
            owner: "alice".to_string(),
            ident: "alice#toolbox".to_string(),
            version: "1.0.0".to_string(),
            state: "blocked".to_string(),
            session_token: token.clone(),
            ident_signature: sign_state("blocked"),
        }),
    )
    .await;
    assert!(refused.err().unwrap().0 == StatusCode::BAD_REQUEST);

    // The happy path moves the version to deprecated, appends one log
    // entry, and the index reflects it.
    let response = release_state(
        State(state.clone()),
        Json(ReleaseStateRequest {
            owner: "alice".to_string(),
            ident: "alice#toolbox".to_string(),
            version: "1.0.0".to_string(),
            state: "deprecated".to_string(),
            session_token: token.clone(),
            ident_signature: sign_state("deprecated"),
        }),
    )
    .await
    .expect("state change accepted")
    .0;
    assert_eq!(response.state, "deprecated");
    assert_eq!(store.log_size().unwrap(), log_before + 1);

    let index = package_index(
        State(state.clone()),
        axum::extract::Path("alice#toolbox".to_string()),
    )
    .await
    .expect("index served")
    .0;
    assert_eq!(index.versions[0].state, "deprecated");

    // The transition's log entry has a verifying inclusion proof.
    let leaves = store.log_leaf_hashes(None).unwrap();
    let root = crate::log::root(&leaves);
    let index_n = response.log_entry.index as usize;
    let path = crate::log::inclusion_path(index_n, &leaves);
    crate::log::verify_inclusion(index_n, leaves.len(), &leaves[index_n], &path, &root)
        .expect("release-state entry inclusion verifies");
}

#[tokio::test]
async fn transfer_and_org_endpoints_reject_malformed_and_wrong_signatures() {
    let h = harness();
    let alice = register_owner_with_all_keys(&h.store, "alice");
    let bob = register_owner_with_all_keys(&h.store, "bob");
    let alice_token = open_session(&h.store, "alice", &alice.auth_private);
    let bob_token = open_session(&h.store, "bob", &bob.auth_private);
    // A v2 transfer signs the package's terms (plan-168-A), so the package
    // has to exist for a signature to be judged at all.
    publish_valid_package(&h.state, &alice, &alice_token, "1.0.0").await;

    let mut offer = signed_offer(
        &h.state,
        "alice#toolbox",
        "alice",
        &alice,
        &alice_token,
        "bob",
    )
    .await;
    offer.ident_signature = NOT_BASE64.to_string();
    assert_eq!(
        err_of(transfer_offer(State(h.state.clone()), Json(offer)).await).1,
        "malformed identSignature",
    );
    // A signature over a different recipient does not authorize this one.
    let mut offer = signed_offer(
        &h.state,
        "alice#toolbox",
        "alice",
        &alice,
        &alice_token,
        "carol",
    )
    .await;
    offer.to_owner = "bob".to_string();
    assert_eq!(
        err_of(transfer_offer(State(h.state.clone()), Json(offer)).await).1,
        "invalid transfer offer ident signature",
    );
    // A v1-domain signature (the replayable pre-plan-168 message) is refused.
    let mut v1 = b"mfb-repo-transfer-offer-v1\0".to_vec();
    v1.extend_from_slice(b"alice#toolbox\0alice\0bob");
    let mut offer = signed_offer(
        &h.state,
        "alice#toolbox",
        "alice",
        &alice,
        &alice_token,
        "bob",
    )
    .await;
    offer.ident_signature = crypto::encode_bytes(&crypto::sign(&alice.ident_private, &v1).unwrap());
    assert_eq!(
        err_of(transfer_offer(State(h.state.clone()), Json(offer)).await).1,
        "invalid transfer offer ident signature",
    );

    let mut accept = signed_accept(&h.state, "alice#toolbox", "bob", &bob, &bob_token).await;
    accept.ident_signature = NOT_BASE64.to_string();
    assert_eq!(
        err_of(transfer_accept(State(h.state.clone()), Json(accept)).await).1,
        "malformed identSignature",
    );
    // Alice cannot sign bob's acceptance for him.
    let accept = signed_accept(&h.state, "alice#toolbox", "bob", &alice, &bob_token).await;
    assert_eq!(
        err_of(transfer_accept(State(h.state.clone()), Json(accept)).await).1,
        "invalid transfer accept ident signature",
    );
    // A correctly signed acceptance with no pending offer is still refused.
    let unoffered = signed_accept(&h.state, "alice#toolbox", "bob", &bob, &bob_token).await;
    assert_eq!(
        err_of(transfer_accept(State(h.state.clone()), Json(unoffered)).await).0,
        StatusCode::BAD_REQUEST,
    );

    // The org endpoint decodes and verifies its ident signature the same way.
    let mut member = OrgMemberRequest {
        org: "bob".to_string(),
        grantor: "bob".to_string(),
        member: "alice".to_string(),
        role: "admin".to_string(),
        action: "grant".to_string(),
        session_token: bob_token.clone(),
        ident_signature: NOT_BASE64.to_string(),
    };
    assert_eq!(
        err_of(org_members(State(h.state.clone()), Json(member)).await).1,
        "malformed identSignature",
    );
    member = OrgMemberRequest {
        org: "bob".to_string(),
        grantor: "bob".to_string(),
        member: "alice".to_string(),
        role: "admin".to_string(),
        action: "grant".to_string(),
        session_token: bob_token,
        ident_signature: crypto::encode_bytes(
            &crypto::sign(
                &bob.ident_private,
                // signed for the `owner` role, presented as `admin`
                &crypto::org_role_message("bob", "alice", "owner"),
            )
            .unwrap(),
        ),
    };
    assert_eq!(
        err_of(org_members(State(h.state.clone()), Json(member)).await).1,
        "invalid org role ident signature",
    );
    assert!(h.store.org_member_role("bob", "alice").unwrap().is_none());
}

#[tokio::test]
async fn release_state_rejects_every_malformed_request_shape() {
    let h = harness();
    let keys = register_owner_with_all_keys(&h.store, "alice");
    register_owner_with_all_keys(&h.store, "bob");
    let token = open_session(&h.store, "alice", &keys.auth_private);
    let request = |owner: &str, ident: &str, state: &str, session: &str| ReleaseStateRequest {
        owner: owner.to_string(),
        ident: ident.to_string(),
        version: "1.0.0".to_string(),
        state: state.to_string(),
        session_token: session.to_string(),
        ident_signature: crypto::encode_bytes(
            &crypto::sign(
                &keys.ident_private,
                &crypto::release_state_message(ident, "1.0.0", state),
            )
            .unwrap(),
        ),
    };

    assert_eq!(
        err_of(
            release_state(
                State(h.state.clone()),
                Json(request("alice", "alice#toolbox", "yanked", "bad.token")),
            )
            .await
        )
        .1,
        "expired or malformed session token",
    );
    assert_eq!(
        err_of(
            release_state(
                State(h.state.clone()),
                Json(request("bob", "bob#toolbox", "yanked", &token)),
            )
            .await
        )
        .1,
        "session owner does not match requested owner",
    );
    assert_eq!(
        err_of(
            release_state(
                State(h.state.clone()),
                Json(request("alice", "alice#toolbox", "bogus", &token)),
            )
            .await
        )
        .1,
        "state must be one of available, deprecated, or yanked",
    );
    assert_eq!(
        err_of(
            release_state(
                State(h.state.clone()),
                Json(request("alice", "noseparator", "yanked", &token)),
            )
            .await
        )
        .1,
        "ident must use <owner>#<package>",
    );
    // An empty package half, and another owner's package, are both refused
    // even though the session itself is valid.
    for ident in ["alice#", "bob#toolbox"] {
        assert_eq!(
            err_of(
                release_state(
                    State(h.state.clone()),
                    Json(request("alice", ident, "yanked", &token)),
                )
                .await
            )
            .1,
            "ident owner does not match session owner",
            "{ident}",
        );
    }
    let mut malformed = request("alice", "alice#toolbox", "yanked", &token);
    malformed.ident_signature = NOT_BASE64.to_string();
    assert_eq!(
        err_of(release_state(State(h.state.clone()), Json(malformed)).await).1,
        "malformed identSignature",
    );
    // A correctly signed change for a version that was never published is
    // refused by the store, and nothing is logged.
    let before = h.store.log_size().unwrap();
    assert_eq!(
        err_of(
            release_state(
                State(h.state.clone()),
                Json(request("alice", "alice#toolbox", "yanked", &token)),
            )
            .await
        )
        .0,
        StatusCode::BAD_REQUEST,
    );
    assert_eq!(h.store.log_size().unwrap(), before);
}

#[tokio::test]
async fn put_blob_requires_a_well_formed_bearer_header() {
    let h = harness();
    let body = b"payload".to_vec();
    let hash = hex::encode(crypto::sha256(&body));
    let call = |headers: axum::http::HeaderMap| {
        put_blob(
            State(h.state.clone()),
            axum::extract::Path(hash.clone()),
            headers,
            axum::body::Bytes::from(body.clone()),
        )
    };

    assert_eq!(
        err_of(call(axum::http::HeaderMap::new()).await),
        (
            StatusCode::UNAUTHORIZED,
            "missing Authorization header".to_string()
        ),
    );

    let mut basic = axum::http::HeaderMap::new();
    basic.insert(
        axum::http::header::AUTHORIZATION,
        "Basic dXNlcjpwdw".parse().unwrap(),
    );
    assert_eq!(
        err_of(call(basic).await),
        (
            StatusCode::UNAUTHORIZED,
            "Authorization header must be `Bearer <token>`".to_string()
        ),
    );

    let mut blank = axum::http::HeaderMap::new();
    blank.insert(
        axum::http::header::AUTHORIZATION,
        "Bearer    ".parse().unwrap(),
    );
    assert_eq!(
        err_of(call(blank).await).1,
        "Authorization header must be `Bearer <token>`",
    );

    let mut binary = axum::http::HeaderMap::new();
    binary.insert(
        axum::http::header::AUTHORIZATION,
        axum::http::HeaderValue::from_bytes(b"Bearer \xff\xfe").unwrap(),
    );
    assert_eq!(
        err_of(call(binary).await).1,
        "malformed Authorization header"
    );

    // None of the refused shapes stored anything.
    assert!(!h
        .state
        .blob_store
        .exists(&hash, BlobKind::Native)
        .await
        .unwrap());
}

#[tokio::test]
async fn put_blob_is_throttled_per_owner() {
    let h = harness();
    let keys = register_owner_with_all_keys(&h.store, "alice");
    let token = open_session(&h.store, "alice", &keys.auth_private);
    let headers = bearer_headers(&token);

    for index in 0..BLOB_UPLOAD_PER_OWNER_MAX {
        let body = format!("vendored-blob-{index}").into_bytes();
        let hash = hex::encode(crypto::sha256(&body));
        let status = put_blob(
            State(h.state.clone()),
            axum::extract::Path(hash),
            headers.clone(),
            axum::body::Bytes::from(body),
        )
        .await
        .expect("uploads under the cap succeed");
        assert_eq!(status, StatusCode::CREATED);
    }
    let body = b"one too many".to_vec();
    let hash = hex::encode(crypto::sha256(&body));
    let (status, message) = err_of(
        put_blob(
            State(h.state.clone()),
            axum::extract::Path(hash.clone()),
            headers,
            axum::body::Bytes::from(body),
        )
        .await,
    );
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert!(message.contains("rate limit"), "{message}");
    // A throttled upload writes nothing to the datapath.
    assert!(!h
        .state
        .blob_store
        .exists(&hash, BlobKind::Native)
        .await
        .unwrap());
    assert!(h.store.blob_kind(&hash).unwrap().is_none());
}

fn put_field(dst: &mut Vec<u8>, value: &[u8]) {
    put_u32(dst, value.len() as u32);
    dst.extend_from_slice(value);
}

#[tokio::test]
async fn validate_handler_reports_every_request_field_mismatch() {
    let h = harness();
    let keys = register_owner_with_all_keys(&h.store, "alice");
    let token = open_session(&h.store, "alice", &keys.auth_private);
    let (_artifact, valid) = signed_request(
        &h.state,
        &keys,
        &token,
        "1.0.0",
        b"MFPCtestpayload".to_vec(),
    )
    .await;

    // The baseline validates through the public handler.
    let report = validate_package(State(h.state.clone()), Json(clone_request(&valid)))
        .await
        .expect("validation ran")
        .0;
    assert!(report.valid, "{:?}", report.diagnostics);
    assert_eq!(report.content_hash, valid.content_hash);

    // Every wire field is cross-checked against the signed header, so a
    // lying request cannot make the registry index the wrong thing.
    let lying = PackageArtifactRequest {
        ident: "alice#other".to_string(),
        version: "9.9.9".to_string(),
        artifact: valid.artifact.clone(),
        content_hash: "0".repeat(64),
        ident_fingerprint: "1".repeat(64),
        signing_fingerprint: "2".repeat(64),
        session_token: token.clone(),
    };
    let report = validate_package(State(h.state.clone()), Json(lying))
        .await
        .expect("validation ran")
        .0;
    assert!(!report.valid);
    for expected in [
        "request contentHash does not match package content hash",
        "request ident does not match package ident",
        "request version does not match package version",
        "request identFingerprint does not match package header",
        "request signingFingerprint does not match package header",
    ] {
        assert!(
            report.diagnostics.iter().any(|d| d == expected),
            "missing `{expected}` in {:?}",
            report.diagnostics,
        );
    }
    // The report still names the real content hash, not the claimed one.
    assert_eq!(report.content_hash, valid.content_hash);

    // A non-base64 artifact is a hard 400; base64 garbage is a diagnostic.
    let mut undecodable = clone_request(&valid);
    undecodable.artifact = NOT_BASE64.to_string();
    let (status, message) =
        err_of(validate_package(State(h.state.clone()), Json(undecodable)).await);
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(message, "malformed artifact");

    let mut garbage = clone_request(&valid);
    garbage.artifact = crypto::encode_bytes(b"not an mfp package at all");
    let report = validate_package(State(h.state), Json(garbage))
        .await
        .expect("validation ran")
        .0;
    assert!(!report.valid);
    assert_eq!(report.content_hash, "");
    assert_eq!(report.abi_index, serde_json::json!({}));
    assert_eq!(report.diagnostics.len(), 1);
}

#[tokio::test]
async fn validate_reports_owner_author_and_duplicate_problems() {
    let h = harness();
    let keys = register_owner_with_all_keys(&h.store, "alice");
    let token = open_session(&h.store, "alice", &keys.auth_private);
    let (owner, key) = h.store.owner_with_auth_key("alice").unwrap().unwrap();

    // A package whose ident owner is not registered at all.
    let (unregistered_public, unregistered_private) = crypto::generate_keypair();
    let (sign_public, sign_private) = crypto::generate_keypair();
    let stranger = package::test_support::serialize(
        &package::test_support::TestPackage {
            name: "toolbox".to_string(),
            ident: "mallory#toolbox".to_string(),
            version: "1.0.0".to_string(),
            author: "mallory".to_string(),
            url: String::new(),
            payload: b"MFPCtestpayload".to_vec(),
            ident_key: format!("ed25519:{}", crypto::encode_bytes(&unregistered_public)),
            signing_key: format!("ed25519:{}", crypto::encode_bytes(&sign_public)),
            proof: "{}".to_string(),
            proof_sig: crypto::sign(&unregistered_private, b"unused").unwrap(),
            attestation: "{}".to_string(),
            attestation_sig: vec![0u8; 64],
        },
        &sign_private,
    );
    let parsed = package::parse_mfp_package(&stranger).unwrap();
    let report = validate_package(
        State(h.state.clone()),
        Json(PackageArtifactRequest {
            ident: parsed.ident.clone(),
            version: parsed.version.clone(),
            artifact: crypto::encode_bytes(&stranger),
            content_hash: parsed.content_hash_hex(),
            ident_fingerprint: parsed.ident_fingerprint().unwrap_or_default(),
            signing_fingerprint: parsed.signing_fingerprint().unwrap_or_default(),
            session_token: token.clone(),
        }),
    )
    .await
    .expect("validation ran")
    .0;
    assert!(!report.valid);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d == "session owner does not match package ident owner"),
        "{:?}",
        report.diagnostics,
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d == "package ident owner is not registered"),
        "{:?}",
        report.diagnostics,
    );

    // A validly signed session whose auth fingerprint is no longer one of
    // the owner's current keys stops at "session key does not match".
    let now = now_unix();
    let stale = session_with_claims(
        &h.store,
        owner.id,
        key.id,
        SessionClaims {
            sub: "alice".to_string(),
            owner_id: owner.id,
            auth_fingerprint: crypto::fingerprint(&crypto::generate_keypair().0),
            iat: now,
            exp: now + 3600,
            jti: Uuid::new_v4().to_string(),
            iss: SESSION_TOKEN_ISSUER.to_string(),
            aud: SESSION_TOKEN_AUDIENCE.to_string(),
        },
    );
    let (_artifact, mut request) = signed_request(
        &h.state,
        &keys,
        &token,
        "1.0.0",
        b"MFPCtestpayload".to_vec(),
    )
    .await;
    request.session_token = stale;
    let report = validate_package(State(h.state.clone()), Json(request))
        .await
        .expect("validation ran")
        .0;
    assert!(!report.valid);
    assert_eq!(
        report.diagnostics,
        vec!["session key does not match current owner key".to_string()],
    );

    // An author string that is not the owner's display name is flagged.
    let (signing_public, signing_private) = crypto::generate_keypair();
    let signing_fingerprint = crypto::fingerprint(&signing_public);
    let (attestation, attestation_sig) =
        real_attestation(&h.state, &token, "1.0.0", &signing_fingerprint).await;
    let ident_fingerprint = crypto::fingerprint(&keys.ident_public);
    let proof = format!(
        "{{\"owner\":\"alice\",\"ident\":\"alice#toolbox\",\"version\":\"1.0.0\",\"identFingerprint\":\"{ident_fingerprint}\",\"signingFingerprint\":\"{signing_fingerprint}\"}}",
    );
    let proof_sig = crypto::sign(
        &keys.ident_private,
        &crypto::proof_signing_input(proof.as_bytes()),
    )
    .unwrap();
    let impostor = package::test_support::serialize(
        &package::test_support::TestPackage {
            name: "toolbox".to_string(),
            ident: "alice#toolbox".to_string(),
            version: "1.0.0".to_string(),
            author: "someone-else".to_string(),
            url: String::new(),
            payload: b"MFPCtestpayload".to_vec(),
            ident_key: format!("ed25519:{}", crypto::encode_bytes(&keys.ident_public)),
            signing_key: format!("ed25519:{}", crypto::encode_bytes(&signing_public)),
            proof,
            proof_sig,
            attestation,
            attestation_sig,
        },
        &signing_private,
    );
    let parsed = package::parse_mfp_package(&impostor).unwrap();
    let report = validate_package(
        State(h.state.clone()),
        Json(PackageArtifactRequest {
            ident: parsed.ident.clone(),
            version: parsed.version.clone(),
            artifact: crypto::encode_bytes(&impostor),
            content_hash: parsed.content_hash_hex(),
            ident_fingerprint: parsed.ident_fingerprint().unwrap(),
            signing_fingerprint: parsed.signing_fingerprint().unwrap(),
            session_token: token.clone(),
        }),
    )
    .await
    .expect("validation ran")
    .0;
    assert!(!report.valid);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d == "package author does not match owner name"),
        "{:?}",
        report.diagnostics,
    );

    // Once a version is published, re-validating the same artifact reports
    // the collision instead of silently accepting a re-publish.
    publish_valid_package(&h.state, &keys, &token, "2.0.0").await;
    let (_artifact, republish) = signed_request(
        &h.state,
        &keys,
        &token,
        "2.0.0",
        b"MFPCtestpayload".to_vec(),
    )
    .await;
    let report = validate_package(State(h.state), Json(republish))
        .await
        .expect("validation ran")
        .0;
    assert!(!report.valid);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d == "package version alice#toolbox@2.0.0 is already published"),
        "{:?}",
        report.diagnostics,
    );
}

#[tokio::test]
async fn validate_refuses_an_unsigned_container() {
    let h = harness();
    let keys = register_owner_with_all_keys(&h.store, "alice");
    let token = open_session(&h.store, "alice", &keys.auth_private);

    // `test_support::serialize` always signs, so build the unsigned shape
    // (signatureType 0 / zero-length signature) here.
    let payload = b"MFPCtestpayload".to_vec();
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&[0x4d, 0x46, 0x50, 0x0d, 0x0a, 0x1a, 0x0a, 0x00]);
    put_u16(&mut bytes, 1);
    put_u16(&mut bytes, 0);
    put_u16(&mut bytes, 1);
    put_u16(&mut bytes, 0);
    put_u32(&mut bytes, 0);
    put_field(&mut bytes, b"toolbox");
    put_field(&mut bytes, b"alice#toolbox");
    put_field(&mut bytes, b"1.0.0");
    put_field(&mut bytes, b"alice");
    put_field(&mut bytes, b"");
    // An unsigned container may carry none of the trust-chain fields, so
    // every one of them is empty here.
    put_field(&mut bytes, b"");
    put_field(&mut bytes, b"");
    put_field(&mut bytes, b"");
    put_field(&mut bytes, &[]);
    put_field(&mut bytes, b"");
    put_field(&mut bytes, &[]);
    bytes.extend_from_slice(&crypto::sha256(&payload));
    put_u64(&mut bytes, payload.len() as u64);
    put_u16(&mut bytes, 0); // signatureType: unsigned
    put_u32(&mut bytes, 0); // signatureLength
    bytes.extend_from_slice(&payload);

    let parsed = package::parse_mfp_package(&bytes).expect("unsigned container parses");
    assert_eq!(parsed.signature_type, 0);
    let report = validate_package(
        State(h.state),
        Json(PackageArtifactRequest {
            ident: parsed.ident.clone(),
            version: parsed.version.clone(),
            artifact: crypto::encode_bytes(&bytes),
            content_hash: parsed.content_hash_hex(),
            ident_fingerprint: parsed.ident_fingerprint().unwrap_or_default(),
            signing_fingerprint: parsed.signing_fingerprint().unwrap_or_default(),
            session_token: token,
        }),
    )
    .await
    .expect("validation ran")
    .0;
    assert!(!report.valid);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d == "registry publishes require an Ed25519-signed package"),
        "{:?}",
        report.diagnostics,
    );
}

#[tokio::test]
async fn validate_requires_every_vendor_blob_to_be_uploaded_first() {
    let h = harness();
    let keys = register_owner_with_all_keys(&h.store, "alice");
    let token = open_session(&h.store, "alice", &keys.auth_private);

    let vendor_bytes = b"\x7fELF vendored archive".to_vec();
    let vendor_hash = crypto::sha256(&vendor_bytes);
    let vendor_hex = hex::encode(vendor_hash);
    let strings = mfpc_string_pool(&["snd", "libsnd.a", "linux", "aarch64"]);
    let payload = mfpc_container(&[
        (2, strings.clone()),
        (10, mfpc_vendor_table(1, &vendor_hash)),
        (15, mfpc_abi_section(&[(0, [0xab; 32])])),
    ]);

    // The blob is not uploaded yet: the section-10 reference is reported
    // per locator, naming the logical library and the source file.
    let (_artifact, request) =
        signed_request(&h.state, &keys, &token, "1.0.0", payload.clone()).await;
    let report = validate_package(State(h.state.clone()), Json(request))
        .await
        .expect("validation ran")
        .0;
    assert!(!report.valid);
    assert!(
        report.diagnostics.iter().any(|d| d
            == &format!(
                "native library 'snd' references vendor blob {vendor_hex} (libsnd.a) that is not uploaded"
            )),
        "{:?}",
        report.diagnostics,
    );
    // The ABI index is still parsed and served out of the same payload.
    assert_eq!(report.abi_index["snd"], hex::encode([0xab; 32]));

    // Upload the blob, then the same package validates and publishes, and
    // the version→blob edge makes the vendor blob reachable.
    put_blob(
        State(h.state.clone()),
        axum::extract::Path(vendor_hex.clone()),
        bearer_headers(&token),
        axum::body::Bytes::from(vendor_bytes),
    )
    .await
    .expect("vendor blob uploaded");
    let (_artifact, request) = signed_request(&h.state, &keys, &token, "1.0.0", payload).await;
    let published = publish_package(State(h.state.clone()), Json(request))
        .await
        .expect("publish succeeds once the vendor blob exists")
        .0;
    assert_eq!(published.version, "1.0.0");
    assert!(published.blob_stored);
    assert!(
        h.store.blob_is_reachable(&vendor_hex).unwrap(),
        "the publish must record the version -> vendor blob edge",
    );
    // The served index carries the ABI map the payload declared.
    let index = package_index(
        State(h.state.clone()),
        axum::extract::Path("alice#toolbox".to_string()),
    )
    .await
    .expect("index served")
    .0;
    assert_eq!(
        index.versions[0].abi_map().get("snd").map(String::as_str),
        Some(hex::encode([0xab; 32]).as_str()),
    );

    // A section-10 table that declares more entries than it can hold is a
    // hard diagnostic, not a silently empty vendor list.
    let malformed = mfpc_container(&[(2, strings), (10, mfpc_vendor_table(2000, &vendor_hash))]);
    let (_artifact, request) = signed_request(&h.state, &keys, &token, "2.0.0", malformed).await;
    let report = validate_package(State(h.state), Json(request))
        .await
        .expect("validation ran")
        .0;
    assert!(!report.valid);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.starts_with("native library table is malformed")),
        "{:?}",
        report.diagnostics,
    );
}

/// plan-61-A Phase 3: `author`/`url` round-trip from the **signed** MANIFEST
/// into `package_versions`, and an empty value is stored as NULL rather than
/// `''` — "the publisher set nothing" is one fact, not two.
#[tokio::test]
async fn author_and_url_round_trip_from_the_signed_manifest() {
    let h = harness();
    let keys = register_owner_with_all_keys(&h.store, "alice");
    let token = open_session(&h.store, "alice", &keys.auth_private);

    // strings: 0="", 1="alice", 2="https://example.invalid/toolbox"
    let strings = mfpc_string_pool(&["", "alice", "https://example.invalid/toolbox"]);
    let payload = mfpc_container(&[(1, mfpc_manifest_section(1, 2)), (2, strings)]);
    let (_artifact, request) = signed_request_with_header_metadata(
        &h.state,
        &keys,
        &token,
        "1.0.0",
        payload,
        "alice",
        "https://example.invalid/toolbox",
    )
    .await;
    let _ = publish_package(State(h.state.clone()), Json(request))
        .await
        .expect("publish succeeds");

    let (author, url) = h.store.version_metadata_for_test("alice#toolbox", "1.0.0");
    assert_eq!(author.as_deref(), Some("alice"));
    assert_eq!(url.as_deref(), Some("https://example.invalid/toolbox"));

    // An unset `url` stores NULL, not "".
    //
    // Only `url` is exercised here: an empty *author* cannot reach this
    // code at all, because `validate_package_request` already rejects a
    // package whose header author differs from the owner name
    // (`server.rs:2243`), and no owner is named "". The plan asked for
    // "publish with both empty"; that half is unbuildable. See §Corrections.
    let strings = mfpc_string_pool(&["", "alice"]);
    let payload = mfpc_container(&[(1, mfpc_manifest_section(1, 0)), (2, strings)]);
    let (_artifact, request) =
        signed_request_with_header_metadata(&h.state, &keys, &token, "2.0.0", payload, "alice", "")
            .await;
    let _ = publish_package(State(h.state.clone()), Json(request))
        .await
        .expect("publish succeeds with no url");

    let (author, url) = h.store.version_metadata_for_test("alice#toolbox", "2.0.0");
    assert_eq!(author.as_deref(), Some("alice"));
    assert_eq!(url, None, "an empty url is NULL, not the empty string");
}
