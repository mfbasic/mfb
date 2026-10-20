//! plan-176-I: `server::tests` — the accounts tests. The shared harnesses (owners, transfers,\n//! MFPC builders, signed requests) live in `server/tests/mod.rs`.

use super::*;

#[test]
fn jwt_creation_sets_expected_claims_and_verifies() {
    let temp = tempfile::tempdir().unwrap();
    let db_path = temp.path().join("meta.db");
    let data_path = temp.path().join("data");
    let opened = Store::open_repository(&db_path, &data_path).unwrap();
    let store = opened.store;
    let (_public, private) = register_owner_with_keys(&store, "alice");

    let challenge = store.create_challenge("alice").unwrap();
    let message = crypto::challenge_message(&challenge.id, &challenge.nonce);
    let signature = crypto::sign(&private, &message).unwrap();
    let (owner, key) = store.complete_challenge(&challenge.id, &signature).unwrap();
    let issued_at = crate::store::now_unix();
    let expires_at = issued_at + 3600;
    let jwt_id = Uuid::new_v4().to_string();
    let claims = SessionClaims {
        sub: owner.owner_display,
        owner_id: owner.id,
        auth_fingerprint: key.fingerprint,
        iat: issued_at,
        exp: expires_at,
        jti: jwt_id.clone(),
        iss: SESSION_TOKEN_ISSUER.to_string(),
        aud: SESSION_TOKEN_AUDIENCE.to_string(),
    };
    let token = encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(&store.server_secret().unwrap()),
    )
    .unwrap();
    store
        .insert_session(&NewSession {
            owner_id: owner.id,
            key_id: key.id,
            jwt_id,
            issued_at,
            expires_at,
        })
        .unwrap();

    let payload = token.split('.').nth(1).unwrap();
    let decoded = URL_SAFE_NO_PAD.decode(payload).unwrap();
    let decoded: SessionClaims = serde_json::from_slice(&decoded).unwrap();
    assert_eq!(decoded.sub, "alice");
    assert!(decoded.exp - decoded.iat <= 3600);
    assert_eq!(verify_session_token(&store, &token).unwrap().sub, "alice");
    assert!(verify_session_token(&store, "bad.token").is_err());
}

#[test]
fn jwt_verification_rejects_expired_wrong_signature_and_unknown_session() {
    let temp = tempfile::tempdir().unwrap();
    let db_path = temp.path().join("meta.db");
    let data_path = temp.path().join("data");
    let opened = Store::open_repository(&db_path, &data_path).unwrap();
    let store = opened.store;
    register_owner_with_keys(&store, "alice");
    let (owner, key) = store.owner_with_auth_key("alice").unwrap().unwrap();
    let now = crate::store::now_unix();

    let expired_jti = Uuid::new_v4().to_string();
    let expired_claims = SessionClaims {
        sub: "alice".to_string(),
        owner_id: owner.id,
        auth_fingerprint: key.fingerprint.clone(),
        iat: now - 7200,
        exp: now - 3600,
        jti: expired_jti.clone(),
        iss: SESSION_TOKEN_ISSUER.to_string(),
        aud: SESSION_TOKEN_AUDIENCE.to_string(),
    };
    let expired_token = encode(
        &Header::new(Algorithm::HS256),
        &expired_claims,
        &EncodingKey::from_secret(&store.server_secret().unwrap()),
    )
    .unwrap();
    store
        .insert_session(&NewSession {
            owner_id: owner.id,
            key_id: key.id,
            jwt_id: expired_jti,
            issued_at: now - 7200,
            expires_at: now - 3600,
        })
        .unwrap();
    assert!(verify_session_token(&store, &expired_token)
        .unwrap_err()
        .contains("expired or malformed"));

    let unknown_claims = SessionClaims {
        sub: "alice".to_string(),
        owner_id: owner.id,
        auth_fingerprint: key.fingerprint.clone(),
        iat: now,
        exp: now + 3600,
        jti: Uuid::new_v4().to_string(),
        iss: SESSION_TOKEN_ISSUER.to_string(),
        aud: SESSION_TOKEN_AUDIENCE.to_string(),
    };
    let unknown_token = encode(
        &Header::new(Algorithm::HS256),
        &unknown_claims,
        &EncodingKey::from_secret(&store.server_secret().unwrap()),
    )
    .unwrap();
    assert!(verify_session_token(&store, &unknown_token)
        .unwrap_err()
        .contains("unknown session"));

    let wrong_signature_token = encode(
        &Header::new(Algorithm::HS256),
        &unknown_claims,
        &EncodingKey::from_secret(b"wrong-secret"),
    )
    .unwrap();
    assert!(verify_session_token(&store, &wrong_signature_token)
        .unwrap_err()
        .contains("expired or malformed"));
}

fn registration_payload(owner: &str) -> (RegisterRequest, Vec<u8>, Vec<u8>) {
    let (auth_public, auth_private) = crypto::generate_keypair();
    let (ident_public, ident_private) = crypto::generate_keypair();
    let auth_proof = crypto::sign(
        &auth_private,
        &crypto::registration_message(crypto::ROLE_AUTH, owner, &auth_public),
    )
    .unwrap();
    let ident_proof = crypto::sign(
        &ident_private,
        &crypto::registration_message(crypto::ROLE_IDENT, owner, &ident_public),
    )
    .unwrap();
    (
        RegisterRequest {
            owner: owner.to_string(),
            auth_key: crypto::encode_bytes(&auth_public),
            ident_key: crypto::encode_bytes(&ident_public),
            proofs: RegisterProofs {
                auth: crypto::encode_bytes(&auth_proof),
                ident: crypto::encode_bytes(&ident_proof),
            },
        },
        auth_public,
        ident_public,
    )
}

#[tokio::test]
async fn register_handler_creates_the_account_and_rejects_bad_keys() {
    let h = harness();
    let (request, auth_public, ident_public) = registration_payload("alice");
    let response = register(State(h.state.clone()), peer("127.0.0.1"), Json(request))
        .await
        .expect("registration accepted")
        .0;
    assert_eq!(response.owner, "alice");
    assert_eq!(response.auth_fingerprint, crypto::fingerprint(&auth_public));
    assert_eq!(
        response.ident_fingerprint,
        crypto::fingerprint(&ident_public)
    );
    // The account really landed, bound to the ident key it named.
    let (owner, ident_key) = h.store.owner_with_ident_key("alice").unwrap().unwrap();
    assert_eq!(owner.owner_display, "alice");
    assert_eq!(ident_key.public_key, ident_public);

    // Re-registering a taken name is a 409, not a 400 and not a silent
    // takeover of the existing account's keys.
    let (again, _, _) = registration_payload("alice");
    let (status, message) =
        err_of(register(State(h.state.clone()), peer("127.0.0.2"), Json(again)).await);
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(message.contains("already in use"), "{message}");
    assert_eq!(
        h.store
            .owner_with_ident_key("alice")
            .unwrap()
            .unwrap()
            .1
            .public_key,
        ident_public,
    );

    // Every base64 field is decoded, and the diagnostic names which one.
    for (field, index) in [
        ("authKey", 0usize),
        ("identKey", 1),
        ("auth proof", 2),
        ("ident proof", 3),
    ] {
        let (mut request, _, _) = registration_payload("carol");
        match index {
            0 => request.auth_key = NOT_BASE64.to_string(),
            1 => request.ident_key = NOT_BASE64.to_string(),
            2 => request.proofs.auth = NOT_BASE64.to_string(),
            _ => request.proofs.ident = NOT_BASE64.to_string(),
        }
        let (status, message) =
            err_of(register(State(h.state.clone()), peer("127.0.0.3"), Json(request)).await);
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(message, format!("malformed {field}"));
    }

    // Role separation: a proof made for the ident role cannot stand in as
    // the auth proof (and vice versa), so one leaked proof is not two.
    let (mut swapped, _, _) = registration_payload("dave");
    std::mem::swap(&mut swapped.proofs.auth, &mut swapped.proofs.ident);
    let (status, message) =
        err_of(register(State(h.state.clone()), peer("127.0.0.4"), Json(swapped)).await);
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(message.contains("proof-of-possession"), "{message}");
    assert!(h.store.owner_with_ident_key("dave").unwrap().is_none());
}

#[tokio::test]
async fn challenge_handler_issues_a_nonce_and_refuses_unknown_owners() {
    let h = harness();
    let keys = register_owner_with_all_keys(&h.store, "alice");
    let auth_public = crypto::public_from_private(&keys.auth_private).unwrap();

    let (status, message) = err_of(
        challenge(
            State(h.state.clone()),
            peer("127.0.0.1"),
            Json(ChallengeRequest {
                owner: "nobody".to_string(),
                auth_fingerprint: String::new(),
            }),
        )
        .await,
    );
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(message, "unknown owner");

    let issued = challenge(
        State(h.state.clone()),
        peer("127.0.0.1"),
        Json(ChallengeRequest {
            owner: "alice".to_string(),
            auth_fingerprint: crypto::fingerprint(&auth_public),
        }),
    )
    .await
    .expect("challenge issued")
    .0;
    assert!(!issued.challenge_id.is_empty());
    assert_eq!(
        crypto::decode_bytes(&issued.nonce, "nonce").unwrap().len(),
        32,
    );
    assert!(issued.expires_at > now_unix());

    // A challenge is bound to one specific machine key: an unknown
    // fingerprint gets no nonce to sign.
    let (status, _) = err_of(
        challenge(
            State(h.state),
            peer("127.0.0.1"),
            Json(ChallengeRequest {
                owner: "alice".to_string(),
                auth_fingerprint: crypto::fingerprint(&crypto::generate_keypair().0),
            }),
        )
        .await,
    );
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn login_handler_mints_a_verifiable_session_and_refuses_replay() {
    let h = harness();
    let keys = register_owner_with_all_keys(&h.store, "alice");
    let auth_public = crypto::public_from_private(&keys.auth_private).unwrap();
    let fingerprint = crypto::fingerprint(&auth_public);

    let issued = challenge(
        State(h.state.clone()),
        peer("127.0.0.1"),
        Json(ChallengeRequest {
            owner: "alice".to_string(),
            auth_fingerprint: fingerprint.clone(),
        }),
    )
    .await
    .unwrap()
    .0;
    let nonce = crypto::decode_bytes(&issued.nonce, "nonce").unwrap();
    let signature = crypto::sign(
        &keys.auth_private,
        &crypto::challenge_message(&issued.challenge_id, &nonce),
    )
    .unwrap();

    // A malformed signature is a 400 and never reaches the store.
    let (status, message) = err_of(
        login(
            State(h.state.clone()),
            peer("10.0.0.1"),
            Json(LoginRequest {
                challenge_id: issued.challenge_id.clone(),
                signature: NOT_BASE64.to_string(),
            }),
        )
        .await,
    );
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(message, "malformed signature");

    // A signature by the wrong key does not open a session.
    let (other_public, other_private) = crypto::generate_keypair();
    let _ = other_public;
    let wrong = crypto::sign(
        &other_private,
        &crypto::challenge_message(&issued.challenge_id, &nonce),
    )
    .unwrap();
    let (status, _) = err_of(
        login(
            State(h.state.clone()),
            peer("10.0.0.1"),
            Json(LoginRequest {
                challenge_id: issued.challenge_id.clone(),
                signature: crypto::encode_bytes(&wrong),
            }),
        )
        .await,
    );
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let before = now_unix();
    let session = login(
        State(h.state.clone()),
        peer("10.0.0.1"),
        Json(LoginRequest {
            challenge_id: issued.challenge_id.clone(),
            signature: crypto::encode_bytes(&signature),
        }),
    )
    .await
    .expect("login succeeds")
    .0;
    assert_eq!(session.owner, "alice");
    assert!(session.expires_at >= before + 3600);
    let claims = verify_session_token(&h.store, &session.session_token)
        .expect("the minted token is a live session");
    assert_eq!(claims.sub, "alice");
    assert_eq!(claims.auth_fingerprint, fingerprint);
    assert_eq!(claims.iss, SESSION_TOKEN_ISSUER);
    assert_eq!(claims.aud, SESSION_TOKEN_AUDIENCE);
    assert_eq!(claims.exp, session.expires_at);

    // A challenge is single-use: replaying the same signed nonce is a 409,
    // so a captured login cannot be replayed for a second session.
    let (status, message) = err_of(
        login(
            State(h.state),
            peer("10.0.0.1"),
            Json(LoginRequest {
                challenge_id: issued.challenge_id,
                signature: crypto::encode_bytes(&signature),
            }),
        )
        .await,
    );
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(message.contains("reused challenge"), "{message}");
}

#[tokio::test]
async fn machine_link_relays_an_opaque_blob_exactly_once() {
    let h = harness();
    let keys = register_owner_with_all_keys(&h.store, "alice");
    let token = open_session(&h.store, "alice", &keys.auth_private);
    register_owner_with_all_keys(&h.store, "bob");

    let code = crypto::generate_pairing_code();
    let lookup = crypto::pairing_lookup(&code);
    // The relayed ciphertext is opaque to the server: assert it comes back
    // byte-identical rather than re-deriving it here.
    let blob = vec![7u8; 128];
    let salt = vec![9u8; 16];
    let good_blob = crypto::encode_bytes(&blob);
    let good_salt = crypto::encode_bytes(&salt);
    let (approval_public, approval_private) = crypto::pairing_approval_keypair(&code).unwrap();
    let good_approval_key = crypto::encode_bytes(&approval_public);
    let start =
        |owner: &str, session: &str, lookup: &str, blob: &str, salt: &str| LinkStartRequest {
            owner: owner.to_string(),
            lookup: lookup.to_string(),
            blob: blob.to_string(),
            salt: salt.to_string(),
            approval_key: good_approval_key.clone(),
            session_token: session.to_string(),
        };

    // An anonymous caller cannot park a blob on an account.
    assert_eq!(
        err_of(
            link_start(
                State(h.state.clone()),
                Json(start("alice", "bad.token", &lookup, &good_blob, &good_salt)),
            )
            .await
        )
        .1,
        "expired or malformed session token",
    );
    // Nor can alice's session park one on bob.
    assert_eq!(
        err_of(
            link_start(
                State(h.state.clone()),
                Json(start("bob", &token, &lookup, &good_blob, &good_salt)),
            )
            .await
        )
        .1,
        "session owner does not match requested owner",
    );
    for bad_lookup in ["short", &"A".repeat(64)] {
        assert_eq!(
            err_of(
                link_start(
                    State(h.state.clone()),
                    Json(start("alice", &token, bad_lookup, &good_blob, &good_salt)),
                )
                .await
            )
            .1,
            "malformed pairing lookup",
            "{bad_lookup}",
        );
    }
    assert_eq!(
        err_of(
            link_start(
                State(h.state.clone()),
                Json(start("alice", &token, &lookup, NOT_BASE64, &good_salt)),
            )
            .await
        )
        .1,
        "malformed blob",
    );
    assert_eq!(
        err_of(
            link_start(
                State(h.state.clone()),
                Json(start("alice", &token, &lookup, &good_blob, NOT_BASE64)),
            )
            .await
        )
        .1,
        "malformed salt",
    );
    // Size bounds: an empty blob and an oversized one are both refused, so
    // the relay cannot be used as free storage.
    for bad in [Vec::new(), vec![0u8; 4097]] {
        assert_eq!(
            err_of(
                link_start(
                    State(h.state.clone()),
                    Json(start(
                        "alice",
                        &token,
                        &lookup,
                        &crypto::encode_bytes(&bad),
                        &good_salt,
                    )),
                )
                .await
            )
            .1,
            "malformed pairing blob",
        );
    }

    let started = link_start(
        State(h.state.clone()),
        Json(start("alice", &token, &lookup, &good_blob, &good_salt)),
    )
    .await
    .expect("pairing blob stored")
    .0;
    assert_eq!(started.owner, "alice");
    assert!(started.expires_at > now_unix());

    let (new_public, new_private) = crypto::generate_keypair();
    let proof = crypto::encode_bytes(
        &crypto::sign(
            &new_private,
            &crypto::registration_message(crypto::ROLE_AUTH, "alice", &new_public),
        )
        .unwrap(),
    );
    let key = crypto::encode_bytes(&new_public);
    // The pairing approval: the typed code signs THIS auth key for THIS
    // pairing (bug-583).
    let approval = crypto::encode_bytes(
        &crypto::sign(
            &approval_private,
            &crypto::pairing_approval_message(&lookup, &new_public),
        )
        .unwrap(),
    );
    let fetch = |owner: &str, lookup: &str, auth_key: &str, proof: &str| LinkFetchRequest {
        owner: owner.to_string(),
        lookup: lookup.to_string(),
        auth_key: auth_key.to_string(),
        proof: proof.to_string(),
        approval: approval.clone(),
    };

    assert_eq!(
        err_of(
            link_fetch(
                State(h.state.clone()),
                Json(fetch("alice", &lookup, NOT_BASE64, &proof)),
            )
            .await
        )
        .1,
        "malformed authKey",
    );
    assert_eq!(
        err_of(
            link_fetch(
                State(h.state.clone()),
                Json(fetch("alice", &lookup, &key, NOT_BASE64)),
            )
            .await
        )
        .1,
        "malformed proof",
    );
    assert_eq!(
        err_of(
            link_fetch(
                State(h.state.clone()),
                Json(fetch("nobody", &lookup, &key, &proof)),
            )
            .await
        )
        .1,
        "unknown owner",
    );
    // A proof-of-possession bound to a different account name does not
    // unlock this one (the message is owner-scoped).
    let bob_proof = crypto::encode_bytes(
        &crypto::sign(
            &new_private,
            &crypto::registration_message(crypto::ROLE_AUTH, "bob", &new_public),
        )
        .unwrap(),
    );
    assert_eq!(
        err_of(
            link_fetch(
                State(h.state.clone()),
                Json(fetch("alice", &lookup, &key, &bob_proof)),
            )
            .await
        )
        .1,
        "invalid auth proof-of-possession signature",
    );
    // A wrong code finds nothing — and crucially does not burn the pending
    // pairing, which the successful fetch below proves.
    assert_eq!(
        err_of(
            link_fetch(
                State(h.state.clone()),
                Json(fetch("alice", &"b".repeat(64), &key, &proof)),
            )
            .await
        )
        .1,
        "unknown, used, or expired pairing code",
    );

    let fetched = link_fetch(
        State(h.state.clone()),
        Json(fetch("alice", &lookup, &key, &proof)),
    )
    .await
    .expect("pairing relayed")
    .0;
    assert_eq!(fetched.owner, "alice");
    assert_eq!(crypto::decode_bytes(&fetched.blob, "blob").unwrap(), blob);
    assert_eq!(crypto::decode_bytes(&fetched.salt, "salt").unwrap(), salt);
    assert_eq!(fetched.auth_fingerprint, crypto::fingerprint(&new_public));

    // The new machine's key is now a first-class credential on the account.
    let new_session =
        open_session_for_key(&h.store, "alice", &new_private, &fetched.auth_fingerprint);
    assert_eq!(
        verify_session_token(&h.store, &new_session).unwrap().sub,
        "alice",
    );
    // And the blob is single-use — a second fetch gets nothing.
    assert_eq!(
        err_of(link_fetch(State(h.state), Json(fetch("alice", &lookup, &key, &proof)),).await).1,
        "unknown, used, or expired pairing code",
    );
}

/// bug-583: the server-visible `lookup` is NOT the pairing approval. The
/// relay (or any database reader) sees the lookup the moment the old
/// machine parks the blob; if that alone enrols an auth key, a server
/// that provably cannot read the ident blob can still impersonate the
/// account at the auth layer — exactly what plan-23 §2 ("a full server
/// compromise yields zero user keys", machines are equals) forbids.
/// Authorization must require the pairing CODE, which only the user
/// carries from the old machine to the new one.
#[tokio::test]
async fn pairing_lookup_without_code_cannot_enrol_an_auth_key() {
    let h = harness();
    let keys = register_owner_with_all_keys(&h.store, "alice");
    let token = open_session(&h.store, "alice", &keys.auth_private);

    // An honest link starts: the old machine seals its ident keypair
    // under the code and parks it. The relay now knows the lookup.
    let code = crypto::generate_pairing_code();
    let lookup = crypto::pairing_lookup(&code);
    let mut plaintext = keys.ident_private.clone();
    plaintext.extend_from_slice(&keys.ident_public);
    let (blob, salt) = crypto::seal_pairing_blob(&code, &plaintext).unwrap();
    let (approval_public, approval_private) = crypto::pairing_approval_keypair(&code).unwrap();
    let _ = link_start(
        State(h.state.clone()),
        Json(LinkStartRequest {
            owner: "alice".to_string(),
            lookup: lookup.clone(),
            blob: crypto::encode_bytes(&blob),
            salt: crypto::encode_bytes(&salt),
            approval_key: crypto::encode_bytes(&approval_public),
            session_token: token.clone(),
        }),
    )
    .await
    .expect("pairing blob stored");

    // The relay adversary: it has the lookup, the blob, the salt and the
    // approval public key — everything the server stores — and nothing
    // else. It cannot open the blob, but it can mint a keypair and sign
    // the ordinary auth proof-of-possession over its OWN key, and it can
    // attach an approval signature made with a key of its own choosing.
    let (relay_public, relay_private) = crypto::generate_keypair();
    let relay_proof = crypto::sign(
        &relay_private,
        &crypto::registration_message(crypto::ROLE_AUTH, "alice", &relay_public),
    )
    .unwrap();
    let (forged_public, forged_private) = crypto::generate_keypair();
    assert_ne!(forged_public, approval_public);
    let relay_fetch = |approval: Vec<u8>| LinkFetchRequest {
        owner: "alice".to_string(),
        lookup: lookup.clone(),
        auth_key: crypto::encode_bytes(&relay_public),
        proof: crypto::encode_bytes(&relay_proof),
        approval: crypto::encode_bytes(&approval),
    };
    let forged = crypto::sign(
        &forged_private,
        &crypto::pairing_approval_message(&lookup, &relay_public),
    )
    .unwrap();
    for attempt in [forged, vec![0u8; crypto::SIGNATURE_LEN]] {
        let (status, message) =
            err_of(link_fetch(State(h.state.clone()), Json(relay_fetch(attempt))).await);
        assert_eq!(status, StatusCode::BAD_REQUEST, "{message}");
        assert_eq!(message, "pairing approval does not prove the pairing code");
    }
    // No auth key was enrolled for the relay's fingerprint, so it cannot
    // complete a login challenge.
    assert!(h
        .store
        .owner_auth_key_by_fingerprint("alice", &crypto::fingerprint(&relay_public))
        .unwrap()
        .is_none());

    // An approval the honest new machine made for ITS key cannot be
    // re-pointed at the relay's key: the auth key is inside the signed
    // bytes.
    let (new_public, new_private) = crypto::generate_keypair();
    let new_proof = crypto::sign(
        &new_private,
        &crypto::registration_message(crypto::ROLE_AUTH, "alice", &new_public),
    )
    .unwrap();
    let honest_approval = crypto::sign(
        &approval_private,
        &crypto::pairing_approval_message(&lookup, &new_public),
    )
    .unwrap();
    assert_eq!(
        err_of(
            link_fetch(
                State(h.state.clone()),
                Json(relay_fetch(honest_approval.clone())),
            )
            .await
        )
        .1,
        "pairing approval does not prove the pairing code",
    );

    // POSITIVE PIN: none of the refusals burned the pairing. The honest
    // new machine, holding the typed code, still completes the link, gets
    // the exact relayed bytes back, and its key opens a real session.
    let fetched = link_fetch(
        State(h.state.clone()),
        Json(LinkFetchRequest {
            owner: "alice".to_string(),
            lookup: lookup.clone(),
            auth_key: crypto::encode_bytes(&new_public),
            proof: crypto::encode_bytes(&new_proof),
            approval: crypto::encode_bytes(&honest_approval),
        }),
    )
    .await
    .expect("the machine holding the code still links")
    .0;
    assert_eq!(crypto::decode_bytes(&fetched.blob, "blob").unwrap(), blob);
    assert_eq!(
        crypto::open_pairing_blob(
            &code,
            &crypto::decode_bytes(&fetched.blob, "blob").unwrap(),
            &crypto::decode_bytes(&fetched.salt, "salt").unwrap(),
        )
        .unwrap(),
        plaintext,
    );
    let session = open_session_for_key(
        &h.store,
        "alice",
        &new_private,
        &crypto::fingerprint(&new_public),
    );
    assert_eq!(
        verify_session_token(&h.store, &session).unwrap().sub,
        "alice"
    );
}

#[tokio::test]
async fn session_preamble_cross_checks_jwt_claims_against_the_database() {
    let h = harness();
    let keys = register_owner_with_all_keys(&h.store, "alice");
    register_owner_with_all_keys(&h.store, "bob");
    let (owner, key) = h.store.owner_with_auth_key("alice").unwrap().unwrap();
    let good = open_session(&h.store, "alice", &keys.auth_private);
    let now = now_unix();
    let claims = |auth_fingerprint: String, owner_id: i64| SessionClaims {
        sub: "alice".to_string(),
        owner_id,
        auth_fingerprint,
        iat: now,
        exp: now + 3600,
        jti: Uuid::new_v4().to_string(),
        iss: SESSION_TOKEN_ISSUER.to_string(),
        aud: SESSION_TOKEN_AUDIENCE.to_string(),
    };
    let stale_key = session_with_claims(
        &h.store,
        owner.id,
        key.id,
        claims(crypto::fingerprint(&crypto::generate_keypair().0), owner.id),
    );
    let wrong_owner_id = session_with_claims(
        &h.store,
        owner.id,
        key.id,
        claims(key.fingerprint.clone(), owner.id + 10_000),
    );

    // `/tokens/revoke` funnels through `session_and_ident`, so it is the
    // cheapest probe of that shared preamble.
    let request = |session: &str, owner: &str| TokenRevokeRequest {
        owner: owner.to_string(),
        token_fingerprint: "0".repeat(64),
        session_token: session.to_string(),
        ident_signature: crypto::encode_bytes(&[0u8; 64]),
    };
    assert_eq!(
        err_of(revoke_token(State(h.state.clone()), Json(request("bad.token", "alice"))).await).1,
        "expired or malformed session token",
    );
    assert_eq!(
        err_of(revoke_token(State(h.state.clone()), Json(request(&good, "bob"))).await).1,
        "session owner does not match requested owner",
    );
    assert_eq!(
        err_of(revoke_token(State(h.state.clone()), Json(request(&stale_key, "alice"))).await).1,
        "session key is not a current auth key",
    );
    assert_eq!(
        err_of(
            revoke_token(
                State(h.state.clone()),
                Json(request(&wrong_owner_id, "alice")),
            )
            .await
        )
        .1,
        "session owner does not match requested owner",
    );

    // A well-formed session still needs a real ident signature.
    assert_eq!(
        err_of(revoke_token(State(h.state.clone()), Json(request(&good, "alice"))).await).1,
        "invalid token revocation ident signature",
    );
    let mut malformed = request(&good, "alice");
    malformed.ident_signature = NOT_BASE64.to_string();
    assert_eq!(
        err_of(revoke_token(State(h.state.clone()), Json(malformed)).await).1,
        "malformed identSignature",
    );
    // ...and revoking a fingerprint that names no live token is reported.
    let mut unknown = request(&good, "alice");
    unknown.ident_signature = crypto::encode_bytes(
        &crypto::sign(
            &keys.ident_private,
            &crypto::token_revoke_message("alice", &"0".repeat(64)),
        )
        .unwrap(),
    );
    assert_eq!(
        err_of(revoke_token(State(h.state), Json(unknown)).await).1,
        "no active token with that fingerprint",
    );
}
