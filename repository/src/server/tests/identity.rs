//! plan-176-I: `server::tests` — the identity tests. The shared harnesses (owners, transfers,\n//! MFPC builders, signed requests) live in `server/tests/mod.rs`.

use super::*;

#[tokio::test]
async fn signing_requires_session_and_matching_owner_and_issues_attestation() {
    let temp = tempfile::tempdir().unwrap();
    let db_path = temp.path().join("meta.db");
    let data_path = temp.path().join("data");
    let opened = Store::open_repository(&db_path, &data_path).unwrap();
    let store = opened.store;
    let (_public, private) = register_owner_with_keys(&store, "alice");
    let token = open_session(&store, "alice", &private);
    let state = AppState {
        store: store.clone(),
        blob_store: BlobStore::local(temp.path().join("data")),
        rate_limiter: RateLimiter::new(),
    };
    let (signing_public, _signing_private) = crypto::generate_keypair();
    let signing_fingerprint = crypto::fingerprint(&signing_public);

    // No/garbage session is refused.
    let refused = signing(
        State(state.clone()),
        peer("127.0.0.1"),
        Json(SigningRequest {
            owner: "alice".to_string(),
            ident: "alice#toolbox".to_string(),
            version: "1.2.3".to_string(),
            signing_fingerprint: signing_fingerprint.clone(),
            session_token: "bad.token.here".to_string(),
        }),
    )
    .await;
    assert!(refused.is_err());

    // A session for alice cannot request an attestation naming bob.
    let refused = signing(
        State(state.clone()),
        peer("127.0.0.1"),
        Json(SigningRequest {
            owner: "alice".to_string(),
            ident: "bob#toolbox".to_string(),
            version: "1.2.3".to_string(),
            signing_fingerprint: signing_fingerprint.clone(),
            session_token: token.clone(),
        }),
    )
    .await;
    let (_status, body) = refused.err().expect("mismatched ident owner refused");
    assert!(
        body.error.contains("ident owner does not match"),
        "{}",
        body.error
    );

    // A session for alice cannot pose as bob either.
    let refused = signing(
        State(state.clone()),
        peer("127.0.0.1"),
        Json(SigningRequest {
            owner: "bob".to_string(),
            ident: "bob#toolbox".to_string(),
            version: "1.2.3".to_string(),
            signing_fingerprint: signing_fingerprint.clone(),
            session_token: token.clone(),
        }),
    )
    .await;
    let (_status, body) = refused.err().expect("mismatched session owner refused");
    assert!(
        body.error.contains("session owner does not match"),
        "{}",
        body.error
    );

    // The happy path issues an attestation that verifies under the
    // server key and pins the exact package, version, and keys.
    let response = signing(
        State(state),
        peer("127.0.0.1"),
        Json(SigningRequest {
            owner: "alice".to_string(),
            ident: "alice#toolbox".to_string(),
            version: "1.2.3".to_string(),
            signing_fingerprint: signing_fingerprint.clone(),
            session_token: token,
        }),
    )
    .await
    .expect("attestation issued");
    let (server_public, _server_private) = store.server_keypair().unwrap();
    let signature = crypto::decode_bytes(&response.0.attestation_signature, "signature").unwrap();
    crypto::verify(
        &server_public,
        &crypto::attestation_signing_input(response.0.attestation.as_bytes()),
        &signature,
    )
    .expect("attestation verifies under the server key");
    let attestation: serde_json::Value = serde_json::from_str(&response.0.attestation).unwrap();
    assert_eq!(attestation["owner"], "alice");
    assert_eq!(attestation["ident"], "alice#toolbox");
    assert_eq!(attestation["version"], "1.2.3");
    assert_eq!(
        attestation["signingFingerprint"],
        signing_fingerprint.as_str()
    );
    assert_eq!(
        attestation["repoFingerprint"],
        crypto::fingerprint(&server_public).as_str()
    );
    let (_owner, ident_key) = store.owner_with_ident_key("alice").unwrap().unwrap();
    assert_eq!(
        attestation["identFingerprint"],
        ident_key.fingerprint.as_str()
    );
    assert!(attestation["issued"].is_i64());
}

#[tokio::test]
async fn rotation_refuses_stale_attestations_and_serves_the_chain() {
    let temp = tempfile::tempdir().unwrap();
    let opened =
        Store::open_repository(&temp.path().join("meta.db"), &temp.path().join("data")).unwrap();
    let store = opened.store;
    let keys = register_owner_with_all_keys(&store, "alice");
    let token = open_session(&store, "alice", &keys.auth_private);
    let state = AppState {
        store: store.clone(),
        blob_store: BlobStore::local(temp.path().join("data")),
        rate_limiter: RateLimiter::new(),
    };

    // Build a fully valid package (attestation minted pre-rotation).
    let (signing_public, signing_private) = crypto::generate_keypair();
    let ident_fingerprint = crypto::fingerprint(&keys.ident_public);
    let signing_fingerprint = crypto::fingerprint(&signing_public);
    let (attestation, attestation_sig) =
        real_attestation(&state, &token, "1.0.0", &signing_fingerprint).await;
    let proof = format!(
        "{{\"owner\":\"alice\",\"ident\":\"alice#toolbox\",\"version\":\"1.0.0\",\"identFingerprint\":\"{}\",\"signingFingerprint\":\"{}\",\"issued\":1}}",
        ident_fingerprint, signing_fingerprint,
    );
    let proof_sig = crypto::sign(
        &keys.ident_private,
        &crypto::proof_signing_input(proof.as_bytes()),
    )
    .unwrap();
    let artifact = package::test_support::serialize(
        &package::test_support::TestPackage {
            name: "toolbox".to_string(),
            ident: "alice#toolbox".to_string(),
            version: "1.0.0".to_string(),
            author: "alice".to_string(),
            url: String::new(),
            payload: b"MFPCtestpayload".to_vec(),
            ident_key: format!("ed25519:{}", crypto::encode_bytes(&keys.ident_public)),
            signing_key: format!("ed25519:{}", crypto::encode_bytes(&signing_public)),
            proof: proof.clone(),
            proof_sig: proof_sig.clone(),
            attestation,
            attestation_sig,
        },
        &signing_private,
    );
    let parsed = package::parse_mfp_package(&artifact).unwrap();
    let request = PackageArtifactRequest {
        ident: parsed.ident.clone(),
        version: parsed.version.clone(),
        artifact: crypto::encode_bytes(&artifact),
        content_hash: parsed.content_hash_hex(),
        ident_fingerprint: parsed.ident_fingerprint().unwrap(),
        signing_fingerprint: parsed.signing_fingerprint().unwrap(),
        session_token: token.clone(),
    };
    let report = validate_package_request(&state, &request, "validate", VALIDATE_PER_OWNER_MAX)
        .await
        .unwrap();
    assert!(report.valid, "{:?}", report.diagnostics);

    // Rotate the ident through the handler.
    let (new_public, new_private) = crypto::generate_keypair();
    let chain_signature = crypto::sign(
        &keys.ident_private,
        &crypto::ident_rotation_message("alice", &ident_fingerprint, &new_public),
    )
    .unwrap();
    let possession_proof = crypto::sign(
        &new_private,
        &crypto::registration_message(crypto::ROLE_IDENT, "alice", &new_public),
    )
    .unwrap();
    let rotated = rotate_ident(
        State(state.clone()),
        Json(RotateRequest {
            owner: "alice".to_string(),
            new_ident_key: crypto::encode_bytes(&new_public),
            chain_signature: crypto::encode_bytes(&chain_signature),
            possession_proof: crypto::encode_bytes(&possession_proof),
            session_token: token.clone(),
        }),
    )
    .await
    .expect("rotation accepted")
    .0;
    assert_eq!(rotated.ident_fingerprint, crypto::fingerprint(&new_public));

    // §3.4 step 5 is now reachable: the same pre-rotation package (its
    // attestation names the PAST ident) is refused as stale.
    let report = validate_package_request(&state, &request, "validate", VALIDATE_PER_OWNER_MAX)
        .await
        .unwrap();
    assert!(!report.valid);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains("does not match the owner's current ident key")),
        "{:?}",
        report.diagnostics
    );

    // Refetch + rebuild under the new ident succeeds.
    let (signing_public2, signing_private2) = crypto::generate_keypair();
    let signing_fingerprint2 = crypto::fingerprint(&signing_public2);
    let new_fingerprint = crypto::fingerprint(&new_public);
    let (attestation2, attestation_sig2) =
        real_attestation(&state, &token, "1.0.0", &signing_fingerprint2).await;
    let proof2 = format!(
        "{{\"owner\":\"alice\",\"ident\":\"alice#toolbox\",\"version\":\"1.0.0\",\"identFingerprint\":\"{}\",\"signingFingerprint\":\"{}\",\"issued\":2}}",
        new_fingerprint, signing_fingerprint2,
    );
    let proof_sig2 = crypto::sign(
        &new_private,
        &crypto::proof_signing_input(proof2.as_bytes()),
    )
    .unwrap();
    let rebuilt = package::test_support::serialize(
        &package::test_support::TestPackage {
            name: "toolbox".to_string(),
            ident: "alice#toolbox".to_string(),
            version: "1.0.0".to_string(),
            author: "alice".to_string(),
            url: String::new(),
            payload: b"MFPCtestpayload".to_vec(),
            ident_key: format!("ed25519:{}", crypto::encode_bytes(&new_public)),
            signing_key: format!("ed25519:{}", crypto::encode_bytes(&signing_public2)),
            proof: proof2,
            proof_sig: proof_sig2,
            attestation: attestation2,
            attestation_sig: attestation_sig2,
        },
        &signing_private2,
    );
    let reparsed = package::parse_mfp_package(&rebuilt).unwrap();
    let request = PackageArtifactRequest {
        ident: reparsed.ident.clone(),
        version: reparsed.version.clone(),
        artifact: crypto::encode_bytes(&rebuilt),
        content_hash: reparsed.content_hash_hex(),
        ident_fingerprint: reparsed.ident_fingerprint().unwrap(),
        signing_fingerprint: reparsed.signing_fingerprint().unwrap(),
        session_token: token.clone(),
    };
    let report = validate_package_request(&state, &request, "validate", VALIDATE_PER_OWNER_MAX)
        .await
        .unwrap();
    assert!(report.valid, "{:?}", report.diagnostics);

    // The chain endpoint serves the verifiable link, and a client can
    // follow it from the old pin to the new key.
    let chain = ident_chain(
        State(state.clone()),
        axum::extract::Path("alice".to_string()),
    )
    .await
    .expect("chain served")
    .0;
    assert_eq!(chain.ident_fingerprint, crypto::fingerprint(&new_public));
    assert_eq!(chain.chain.len(), 1);
    let followed = crate::client::follow_ident_chain("alice", &keys.ident_public, &chain.chain)
        .unwrap()
        .expect("chain reaches a successor");
    assert_eq!(followed, new_public);

    // A re-anchor records NO chain link: following from the (rotated)
    // current key yields None — the hard-error case for consumers.
    let (anchor_public, _anchor_private) = crypto::generate_keypair();
    store.reanchor_ident("alice", &anchor_public).unwrap();
    let chain = ident_chain(State(state), axum::extract::Path("alice".to_string()))
        .await
        .expect("chain served")
        .0;
    assert_eq!(chain.ident_fingerprint, crypto::fingerprint(&anchor_public));
    assert!(
        crate::client::follow_ident_chain("alice", &new_public, &chain.chain)
            .unwrap()
            .is_none(),
        "a re-anchored ident must not be reachable through the chain"
    );
}

#[tokio::test]
async fn machine_revocation_is_ident_authorized_and_kills_the_session() {
    let h = harness();
    let keys = register_owner_with_all_keys(&h.store, "alice");
    let token = open_session(&h.store, "alice", &keys.auth_private);
    let auth_public = crypto::public_from_private(&keys.auth_private).unwrap();
    let fingerprint = crypto::fingerprint(&auth_public);

    assert_eq!(
        err_of(
            revoke_challenge(
                State(h.state.clone()),
                Json(RevokeChallengeRequest {
                    owner: "nobody".to_string(),
                }),
            )
            .await
        )
        .1,
        "unknown owner",
    );

    // A fresh ident challenge per attempt: a failed attempt must not be
    // able to reuse a nonce.
    let new_challenge = |state: AppState| async move {
        revoke_challenge(
            State(state),
            Json(RevokeChallengeRequest {
                owner: "alice".to_string(),
            }),
        )
        .await
        .expect("ident challenge issued")
        .0
    };

    let issued = new_challenge(h.state.clone()).await;
    assert!(issued.expires_at > now_unix());
    assert_eq!(
        err_of(
            revoke_machine(
                State(h.state.clone()),
                Json(RevokeRequest {
                    challenge_id: issued.challenge_id.clone(),
                    auth_fingerprint: fingerprint.clone(),
                    ident_signature: NOT_BASE64.to_string(),
                }),
            )
            .await
        )
        .1,
        "malformed identSignature",
    );

    // The AUTH key is a live credential but NOT the revocation authority:
    // signing the revocation with it is refused (plan-23 §3.6).
    let issued = new_challenge(h.state.clone()).await;
    let nonce = crypto::decode_bytes(&issued.nonce, "nonce").unwrap();
    let by_auth = crypto::sign(
        &keys.auth_private,
        &crypto::revocation_message(&issued.challenge_id, &nonce, &fingerprint),
    )
    .unwrap();
    let (status, _) = err_of(
        revoke_machine(
            State(h.state.clone()),
            Json(RevokeRequest {
                challenge_id: issued.challenge_id,
                auth_fingerprint: fingerprint.clone(),
                ident_signature: crypto::encode_bytes(&by_auth),
            }),
        )
        .await,
    );
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        verify_session_token(&h.store, &token).is_ok(),
        "a refused revocation must leave the session alone",
    );

    // bug-493 (audit-3 REPO-02): the challenge loader constrained neither
    // the key role nor the challenge's purpose, so an ordinary
    // `/auth/challenge` for an AUTH key — which any linked machine or
    // publish token can mint for itself — satisfied `/machines/revoke` once
    // the revocation message was signed with that auth key. A challenge is
    // bound to its purpose at creation now: a login challenge cannot revoke
    // (and the refused attempt does not burn it), and a revocation
    // challenge cannot log in.
    let login_challenge = h
        .store
        .create_auth_challenge("alice", &fingerprint)
        .unwrap();
    let forged = crypto::sign(
        &keys.auth_private,
        &crypto::revocation_message(&login_challenge.id, &login_challenge.nonce, &fingerprint),
    )
    .unwrap();
    assert_eq!(
        err_of(
            revoke_machine(
                State(h.state.clone()),
                Json(RevokeRequest {
                    challenge_id: login_challenge.id.clone(),
                    auth_fingerprint: fingerprint.clone(),
                    ident_signature: crypto::encode_bytes(&forged),
                }),
            )
            .await
        ),
        (
            StatusCode::BAD_REQUEST,
            "challenge was not issued for revocation".to_string(),
        ),
    );
    assert!(
        h.store
            .owner_auth_key_by_fingerprint("alice", &fingerprint)
            .unwrap()
            .is_some(),
        "an auth-key challenge must not revoke anything",
    );
    let login_signature = crypto::sign(
        &keys.auth_private,
        &crypto::challenge_message(&login_challenge.id, &login_challenge.nonce),
    )
    .unwrap();
    let _ = login(
        State(h.state.clone()),
        peer("127.0.0.1"),
        Json(LoginRequest {
            challenge_id: login_challenge.id,
            signature: crypto::encode_bytes(&login_signature),
        }),
    )
    .await
    .expect("the refused revocation did not burn the login challenge");
    let ident_challenge = new_challenge(h.state.clone()).await;
    let ident_nonce = crypto::decode_bytes(&ident_challenge.nonce, "nonce").unwrap();
    let ident_login = crypto::sign(
        &keys.ident_private,
        &crypto::challenge_message(&ident_challenge.challenge_id, &ident_nonce),
    )
    .unwrap();
    assert_eq!(
        err_of(
            login(
                State(h.state.clone()),
                peer("127.0.0.1"),
                Json(LoginRequest {
                    challenge_id: ident_challenge.challenge_id,
                    signature: crypto::encode_bytes(&ident_login),
                }),
            )
            .await
        ),
        (
            StatusCode::BAD_REQUEST,
            "challenge was not issued for login".to_string(),
        ),
    );

    // An ident-signed revocation of a fingerprint that is not a current
    // auth key is reported rather than silently accepted.
    let issued = new_challenge(h.state.clone()).await;
    let nonce = crypto::decode_bytes(&issued.nonce, "nonce").unwrap();
    let stranger = crypto::fingerprint(&crypto::generate_keypair().0);
    let signature = crypto::sign(
        &keys.ident_private,
        &crypto::revocation_message(&issued.challenge_id, &nonce, &stranger),
    )
    .unwrap();
    assert_eq!(
        err_of(
            revoke_machine(
                State(h.state.clone()),
                Json(RevokeRequest {
                    challenge_id: issued.challenge_id,
                    auth_fingerprint: stranger,
                    ident_signature: crypto::encode_bytes(&signature),
                }),
            )
            .await
        )
        .1,
        "no current auth key with that fingerprint",
    );

    // A second linked machine, so the revocation below is not of the
    // account's last machine key.
    let (second_public, second_private) = crypto::generate_keypair();
    let second_proof = crypto::sign(
        &second_private,
        &crypto::registration_message(crypto::ROLE_AUTH, "alice", &second_public),
    )
    .unwrap();
    let (_owner, second_key) = h
        .store
        .add_auth_key("alice", &second_public, &second_proof)
        .unwrap();

    // The happy path: the ident key revokes the machine, and the machine's
    // live session dies with it.
    let issued = new_challenge(h.state.clone()).await;
    let nonce = crypto::decode_bytes(&issued.nonce, "nonce").unwrap();
    let signature = crypto::sign(
        &keys.ident_private,
        &crypto::revocation_message(&issued.challenge_id, &nonce, &fingerprint),
    )
    .unwrap();
    let response = revoke_machine(
        State(h.state.clone()),
        Json(RevokeRequest {
            challenge_id: issued.challenge_id,
            auth_fingerprint: fingerprint.clone(),
            ident_signature: crypto::encode_bytes(&signature),
        }),
    )
    .await
    .expect("ident-authorized revocation accepted")
    .0;
    assert_eq!(response.owner, "alice");
    assert_eq!(response.auth_fingerprint, fingerprint);
    assert!(response.revoked);
    assert!(h
        .store
        .owner_auth_key_by_fingerprint("alice", &fingerprint)
        .unwrap()
        .is_none());
    assert!(
        verify_session_token(&h.store, &token).is_err(),
        "revoking the key must invalidate its outstanding session",
    );

    // bug-493: revoking the last current machine key would lock the account
    // out for good — no ident-authorized route adds an auth key — so it is
    // refused even with a valid ident signature, and the key stays current.
    let issued = new_challenge(h.state.clone()).await;
    let nonce = crypto::decode_bytes(&issued.nonce, "nonce").unwrap();
    let signature = crypto::sign(
        &keys.ident_private,
        &crypto::revocation_message(&issued.challenge_id, &nonce, &second_key.fingerprint),
    )
    .unwrap();
    assert_eq!(
        err_of(
            revoke_machine(
                State(h.state.clone()),
                Json(RevokeRequest {
                    challenge_id: issued.challenge_id,
                    auth_fingerprint: second_key.fingerprint.clone(),
                    ident_signature: crypto::encode_bytes(&signature),
                }),
            )
            .await
        ),
        (
            StatusCode::BAD_REQUEST,
            "cannot revoke the account's last auth key; link another machine first".to_string(),
        ),
    );
    assert!(h
        .store
        .owner_auth_key_by_fingerprint("alice", &second_key.fingerprint)
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn rotate_ident_rejects_bad_sessions_and_malformed_material() {
    let h = harness();
    let keys = register_owner_with_all_keys(&h.store, "alice");
    register_owner_with_all_keys(&h.store, "bob");
    let token = open_session(&h.store, "alice", &keys.auth_private);
    let ident_fingerprint = crypto::fingerprint(&keys.ident_public);
    let (new_public, new_private) = crypto::generate_keypair();
    let chain = crypto::encode_bytes(
        &crypto::sign(
            &keys.ident_private,
            &crypto::ident_rotation_message("alice", &ident_fingerprint, &new_public),
        )
        .unwrap(),
    );
    let possession = crypto::encode_bytes(
        &crypto::sign(
            &new_private,
            &crypto::registration_message(crypto::ROLE_IDENT, "alice", &new_public),
        )
        .unwrap(),
    );
    let request = |owner: &str, session: &str| RotateRequest {
        owner: owner.to_string(),
        new_ident_key: crypto::encode_bytes(&new_public),
        chain_signature: chain.clone(),
        possession_proof: possession.clone(),
        session_token: session.to_string(),
    };

    assert_eq!(
        err_of(rotate_ident(State(h.state.clone()), Json(request("alice", "bad.token"))).await).1,
        "expired or malformed session token",
    );
    assert_eq!(
        err_of(rotate_ident(State(h.state.clone()), Json(request("bob", &token))).await).1,
        "session owner does not match requested owner",
    );
    for field in ["newIdentKey", "chainSignature", "possessionProof"] {
        let mut bad = request("alice", &token);
        match field {
            "newIdentKey" => bad.new_ident_key = NOT_BASE64.to_string(),
            "chainSignature" => bad.chain_signature = NOT_BASE64.to_string(),
            _ => bad.possession_proof = NOT_BASE64.to_string(),
        }
        assert_eq!(
            err_of(rotate_ident(State(h.state.clone()), Json(bad)).await).1,
            format!("malformed {field}"),
        );
    }
    // A chain link signed by something other than the retiring ident key
    // cannot re-anchor the account.
    let mut forged = request("alice", &token);
    forged.chain_signature = crypto::encode_bytes(
        &crypto::sign(
            &new_private,
            &crypto::ident_rotation_message("alice", &ident_fingerprint, &new_public),
        )
        .unwrap(),
    );
    assert_eq!(
        err_of(rotate_ident(State(h.state.clone()), Json(forged)).await).0,
        StatusCode::BAD_REQUEST,
    );
    assert_eq!(
        h.store
            .owner_with_ident_key("alice")
            .unwrap()
            .unwrap()
            .1
            .fingerprint,
        ident_fingerprint,
        "a refused rotation must leave the ident binding alone",
    );
}

#[tokio::test]
async fn signing_validates_ident_version_and_fingerprint_shapes() {
    let h = harness();
    let keys = register_owner_with_all_keys(&h.store, "alice");
    let token = open_session(&h.store, "alice", &keys.auth_private);
    let (signing_public, _private) = crypto::generate_keypair();
    let fingerprint = crypto::fingerprint(&signing_public);
    let request = |ident: &str, version: &str, signing_fingerprint: &str| SigningRequest {
        owner: "alice".to_string(),
        ident: ident.to_string(),
        version: version.to_string(),
        signing_fingerprint: signing_fingerprint.to_string(),
        session_token: token.clone(),
    };

    assert_eq!(
        err_of(
            signing(
                State(h.state.clone()),
                peer("127.0.0.1"),
                Json(request("noseparator", "1.0.0", &fingerprint)),
            )
            .await
        )
        .1,
        "ident must use <owner>#<package>",
    );
    // An empty package half, and an over-long ident, are both malformed.
    for ident in ["alice#".to_string(), format!("alice#{}", "p".repeat(250))] {
        assert_eq!(
            err_of(
                signing(
                    State(h.state.clone()),
                    peer("127.0.0.1"),
                    Json(request(&ident, "1.0.0", &fingerprint)),
                )
                .await
            )
            .1,
            "malformed ident",
            "{ident}",
        );
    }
    for version in ["".to_string(), "9".repeat(65)] {
        assert_eq!(
            err_of(
                signing(
                    State(h.state.clone()),
                    peer("127.0.0.1"),
                    Json(request("alice#toolbox", &version, &fingerprint)),
                )
                .await
            )
            .1,
            "malformed version",
            "{version}",
        );
    }
    // The one-off key fingerprint the server is about to put its name on
    // must be 64 lowercase hex characters.
    for bad in ["deadbeef".to_string(), fingerprint.to_uppercase()] {
        assert_eq!(
            err_of(
                signing(
                    State(h.state.clone()),
                    peer("127.0.0.1"),
                    Json(request("alice#toolbox", "1.0.0", &bad)),
                )
                .await
            )
            .1,
            "signingFingerprint must be 64 lowercase hex characters",
            "{bad}",
        );
    }
    // Nothing above was logged: a refused request never records a signing
    // intent, let alone issues an attestation.
    assert_eq!(h.store.log_size().unwrap(), 1);
}

#[tokio::test]
async fn signing_is_rate_limited_per_client_ip() {
    // bug-419: the signing bucket was keyed on the unauthenticated
    // `request.owner` (checked before session verification), so an anonymous
    // attacker naming a victim could exhaust that victim's signing budget
    // and block the victim from obtaining publish attestations. Re-keyed to
    // the peer IP, an attacker only spends their own bucket.
    let h = harness();
    register_owner_with_all_keys(&h.store, "alice");
    // The gate runs before session verification, so even rejected requests
    // spend the client's budget — the point of the cap is CPU, not success.
    let attempt = |ip: &'static str| {
        signing(
            State(h.state.clone()),
            peer(ip),
            Json(SigningRequest {
                owner: "alice".to_string(),
                ident: "alice#toolbox".to_string(),
                version: "1.0.0".to_string(),
                signing_fingerprint: "0".repeat(64),
                session_token: "bad.token".to_string(),
            }),
        )
    };
    let mut last = StatusCode::OK;
    for _ in 0..(SIGNING_PER_IP_MAX + 1) {
        last = err_of(attempt("10.9.9.9").await).0;
    }
    assert_eq!(last, StatusCode::TOO_MANY_REQUESTS);
    // A different client IP has its own bucket — the victim naming the same
    // owner is not locked out, and fails only on its (bad) session token.
    assert_eq!(err_of(attempt("10.1.1.1").await).0, StatusCode::BAD_REQUEST);
}

// --- bug-793: a retried ident rotation keeps the committed staged key ---

/// The local state a client is left in when `/keys/rotate` committed on the
/// server but its response, and the reconciliation after it, were lost: the
/// live pair is still the old ident, and the new pair is staged.
struct LostRotation {
    _dir: tempfile::TempDir,
    paths: crate::local::LocalPaths,
    old_public: Vec<u8>,
    staged_public: Vec<u8>,
    staged_private: Vec<u8>,
}

fn stage_rotation(h: &Harness, commit: bool) -> LostRotation {
    use crate::local::{self, LocalPaths};
    let alice = register_owner_with_all_keys(&h.store, "alice");
    let token = open_session(&h.store, "alice", &alice.auth_private);
    let dir = tempfile::tempdir().unwrap();
    let paths = LocalPaths::new(dir.path().to_owned());
    local::write_ident_keypair(&paths, "alice", &alice.ident_public, &alice.ident_private).unwrap();
    local::write_session(&paths, "alice", &token).unwrap();
    let (staged_public, staged_private) = crypto::generate_keypair();
    local::write_pending_ident_keypair(&paths, "alice", &staged_public, &staged_private).unwrap();
    if commit {
        let chain = crypto::sign(
            &alice.ident_private,
            &crypto::ident_rotation_message(
                "alice",
                &crypto::fingerprint(&alice.ident_public),
                &staged_public,
            ),
        )
        .unwrap();
        let possession = crypto::sign(
            &staged_private,
            &crypto::registration_message(crypto::ROLE_IDENT, "alice", &staged_public),
        )
        .unwrap();
        h.store
            .rotate_ident("alice", &staged_public, &chain, &possession)
            .unwrap();
    }
    LostRotation {
        _dir: dir,
        paths,
        old_public: alice.ident_public,
        staged_public,
        staged_private,
    }
}

/// Run the real client's `rotate_ident` against a real local registry.
async fn retry_rotation(
    h: &Harness,
    paths: &crate::local::LocalPaths,
) -> Result<RotateResponse, String> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let app = build_router(h.state.clone());
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    let paths = paths.clone();
    let result =
        tokio::task::spawn_blocking(move || crate::client::rotate_ident(&url, &paths, "alice"))
            .await
            .unwrap();
    server.abort();
    result
}

/// bug-793: the retry used to overwrite the staged pair, sign a second
/// rotation with the old key (rejected), and then discard the replacement,
/// leaving no copy of the key the registry trusts. It must instead recover
/// the committed staging as the live keypair.
#[tokio::test]
async fn rotation_retry_preserves_committed_staged_key() {
    let _env = crate::local::tests::env_guard();
    let h = harness();
    let lost = stage_rotation(&h, true);

    let response = retry_rotation(&h, &lost.paths)
        .await
        .expect("retry recovers");
    assert_eq!(
        response.ident_fingerprint,
        crypto::fingerprint(&lost.staged_public)
    );
    assert_eq!(
        crate::local::read_ident_private_key(&lost.paths, "alice").unwrap(),
        lost.staged_private,
        "the committed ident private key must become the live key"
    );
    assert_eq!(
        crate::local::read_ident_public_key(&lost.paths, "alice").unwrap(),
        lost.staged_public
    );
    assert!(!lost.paths.ident_pending_private_key_path("alice").exists());
}

/// A promotion interrupted after the live public key was rewritten (the
/// staged private key still pending) is finished by the retry.
#[tokio::test]
async fn rotation_retry_finishes_an_interrupted_promotion() {
    let _env = crate::local::tests::env_guard();
    let h = harness();
    let lost = stage_rotation(&h, true);
    crate::local::write_ident_keypair(
        &lost.paths,
        "alice",
        &lost.staged_public,
        &crate::local::read_ident_private_key(&lost.paths, "alice").unwrap(),
    )
    .unwrap();
    std::fs::remove_file(lost.paths.ident_pending_public_key_path("alice")).unwrap();

    retry_rotation(&h, &lost.paths)
        .await
        .expect("retry recovers");
    assert_eq!(
        crate::local::read_ident_private_key(&lost.paths, "alice").unwrap(),
        lost.staged_private
    );
}

/// A staging the registry never committed (it still holds the live key) is
/// stale: it is discarded and a fresh rotation goes through.
#[tokio::test]
async fn rotation_retry_replaces_an_uncommitted_staging() {
    let _env = crate::local::tests::env_guard();
    let h = harness();
    let lost = stage_rotation(&h, false);

    let response = retry_rotation(&h, &lost.paths)
        .await
        .expect("fresh rotation");
    let live = crate::local::read_ident_public_key(&lost.paths, "alice").unwrap();
    assert_ne!(live, lost.old_public);
    assert_ne!(live, lost.staged_public);
    assert_eq!(response.ident_fingerprint, crypto::fingerprint(&live));
    assert!(!lost.paths.ident_pending_private_key_path("alice").exists());
}

/// The registry holds a key that is neither the staged nor the live one
/// (another machine rotated): nothing is proven, so the staging is kept,
/// nothing is signed with the stale live key, and the error says so.
#[tokio::test]
async fn rotation_retry_keeps_the_staging_when_the_registry_holds_another_key() {
    let _env = crate::local::tests::env_guard();
    let h = harness();
    let lost = stage_rotation(&h, true);
    let (other_public, other_private) = crypto::generate_keypair();
    crate::local::write_pending_ident_keypair(&lost.paths, "alice", &other_public, &other_private)
        .unwrap();

    let err = retry_rotation(&h, &lost.paths).await.unwrap_err();
    assert!(err.contains("staged ident key was left at"), "{err}");
    assert!(err.contains("mfb key rotate"), "{err}");
    assert!(lost.paths.ident_pending_private_key_path("alice").exists());
    assert_eq!(
        crate::local::read_ident_public_key(&lost.paths, "alice").unwrap(),
        lost.old_public
    );
}
