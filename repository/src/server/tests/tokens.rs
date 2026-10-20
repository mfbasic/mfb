//! plan-176-I: `server::tests` — the tokens tests. The shared harnesses (owners, transfers,\n//! MFPC builders, signed requests) live in `server/tests/mod.rs`.

use super::*;

#[tokio::test]
async fn org_roles_are_ident_authorized_and_role_gated() {
    let temp = tempfile::tempdir().unwrap();
    let opened =
        Store::open_repository(&temp.path().join("meta.db"), &temp.path().join("data")).unwrap();
    let store = opened.store;
    let org = register_owner_with_all_keys(&store, "acme");
    let _member = register_owner_with_all_keys(&store, "alice");
    let mallory = register_owner_with_all_keys(&store, "mallory");
    let org_token = open_session(&store, "acme", &org.auth_private);
    let mallory_token = open_session(&store, "mallory", &mallory.auth_private);
    let state = AppState {
        store: store.clone(),
        blob_store: BlobStore::local(opened.packages_dir.clone()),
        rate_limiter: RateLimiter::new(),
    };

    let grant = |grantor: &str, ident_private: &[u8], token: &str, member: &str, role: &str| {
        let sig = crypto::encode_bytes(
            &crypto::sign(
                ident_private,
                &crypto::org_role_message("acme", member, role),
            )
            .unwrap(),
        );
        OrgMemberRequest {
            org: "acme".to_string(),
            grantor: grantor.to_string(),
            member: member.to_string(),
            role: role.to_string(),
            action: "grant".to_string(),
            session_token: token.to_string(),
            ident_signature: sig,
        }
    };

    // The org bootstraps its first member.
    let _ = org_members(
        State(state.clone()),
        Json(grant(
            "acme",
            &org.ident_private,
            &org_token,
            "alice",
            "admin",
        )),
    )
    .await
    .expect("org grants first member");
    assert_eq!(
        store.org_member_role("acme", "alice").unwrap().as_deref(),
        Some("admin")
    );

    // A non-member cannot grant roles even with a valid ident signature.
    let refused = org_members(
        State(state.clone()),
        Json(grant(
            "mallory",
            &mallory.ident_private,
            &mallory_token,
            "mallory",
            "owner",
        )),
    )
    .await;
    assert!(refused.is_err());

    // Removal is logged and takes effect.
    let remove = OrgMemberRequest {
        org: "acme".to_string(),
        grantor: "acme".to_string(),
        member: "alice".to_string(),
        role: String::new(),
        action: "remove".to_string(),
        session_token: org_token.clone(),
        ident_signature: crypto::encode_bytes(
            &crypto::sign(
                &org.ident_private,
                &crypto::org_role_message("acme", "alice", "removed"),
            )
            .unwrap(),
        ),
    };
    let _ = org_members(State(state.clone()), Json(remove))
        .await
        .expect("removal");
    assert!(store.org_member_role("acme", "alice").unwrap().is_none());
}

/// bug-420 item 1: an `admin` member must not be able to seize the org by
/// promoting itself to `owner` or by removing an existing owner. Only an
/// owner (or the org itself) may manage the owner role.
#[tokio::test]
async fn admin_cannot_grant_or_remove_the_owner_role() {
    let temp = tempfile::tempdir().unwrap();
    let opened =
        Store::open_repository(&temp.path().join("meta.db"), &temp.path().join("data")).unwrap();
    let store = opened.store;
    let org = register_owner_with_all_keys(&store, "acme");
    let alice = register_owner_with_all_keys(&store, "alice");
    let _bob = register_owner_with_all_keys(&store, "bob");
    let _carol = register_owner_with_all_keys(&store, "carol");
    let org_token = open_session(&store, "acme", &org.auth_private);
    let alice_token = open_session(&store, "alice", &alice.auth_private);
    let state = AppState {
        store: store.clone(),
        blob_store: BlobStore::local(opened.packages_dir.clone()),
        rate_limiter: RateLimiter::new(),
    };

    let request = |grantor: &str,
                   ident_private: &[u8],
                   token: &str,
                   member: &str,
                   role: &str,
                   action: &str| {
        let message_role = if action == "remove" { "removed" } else { role };
        let sig = crypto::encode_bytes(
            &crypto::sign(
                ident_private,
                &crypto::org_role_message("acme", member, message_role),
            )
            .unwrap(),
        );
        OrgMemberRequest {
            org: "acme".to_string(),
            grantor: grantor.to_string(),
            member: member.to_string(),
            role: role.to_string(),
            action: action.to_string(),
            session_token: token.to_string(),
            ident_signature: sig,
        }
    };

    // The org bootstraps alice as admin and bob as owner.
    let _ = org_members(
        State(state.clone()),
        Json(request(
            "acme",
            &org.ident_private,
            &org_token,
            "alice",
            "admin",
            "grant",
        )),
    )
    .await
    .expect("org grants alice admin");
    let _ = org_members(
        State(state.clone()),
        Json(request(
            "acme",
            &org.ident_private,
            &org_token,
            "bob",
            "owner",
            "grant",
        )),
    )
    .await
    .expect("org grants bob owner");

    // The admin cannot promote itself (or anyone) to owner.
    let self_promote = org_members(
        State(state.clone()),
        Json(request(
            "alice",
            &alice.ident_private,
            &alice_token,
            "alice",
            "owner",
            "grant",
        )),
    )
    .await;
    assert!(
        self_promote.is_err(),
        "an admin must not be able to grant itself the owner role"
    );
    assert_eq!(
        store.org_member_role("acme", "alice").unwrap().as_deref(),
        Some("admin"),
        "the admin's role must be unchanged after the refused self-promotion"
    );

    // The admin cannot remove the org's existing owner.
    let remove_owner = org_members(
        State(state.clone()),
        Json(request(
            "alice",
            &alice.ident_private,
            &alice_token,
            "bob",
            "owner",
            "remove",
        )),
    )
    .await;
    assert!(
        remove_owner.is_err(),
        "an admin must not be able to remove an existing owner"
    );
    assert_eq!(
        store.org_member_role("acme", "bob").unwrap().as_deref(),
        Some("owner"),
        "the owner must still hold the owner role after the refused removal"
    );

    // The admin retains authority over non-owner roles.
    let _ = org_members(
        State(state.clone()),
        Json(request(
            "alice",
            &alice.ident_private,
            &alice_token,
            "carol",
            "publisher",
            "grant",
        )),
    )
    .await
    .expect("an admin may still grant a publisher");
    assert_eq!(
        store.org_member_role("acme", "carol").unwrap().as_deref(),
        Some("publisher")
    );
}

#[tokio::test]
async fn publish_tokens_are_scoped_and_revocable() {
    let temp = tempfile::tempdir().unwrap();
    let opened =
        Store::open_repository(&temp.path().join("meta.db"), &temp.path().join("data")).unwrap();
    let store = opened.store;
    let alice = register_owner_with_all_keys(&store, "alice");
    let owner_token = open_session(&store, "alice", &alice.auth_private);
    let state = AppState {
        store: store.clone(),
        blob_store: BlobStore::local(opened.packages_dir.clone()),
        rate_limiter: RateLimiter::new(),
    };

    // Issue a token scoped to exactly alice#toolbox.
    let (token_public, token_private) = crypto::generate_keypair();
    let token_fingerprint = crypto::fingerprint(&token_public);
    let proof = crypto::sign(
        &token_private,
        &crypto::registration_message(crypto::ROLE_AUTH, "alice", &token_public),
    )
    .unwrap();
    let issue = TokenIssueRequest {
        owner: "alice".to_string(),
        token_key: crypto::encode_bytes(&token_public),
        proof: crypto::encode_bytes(&proof),
        scope: "alice#toolbox".to_string(),
        ttl_seconds: 3600,
        session_token: owner_token.clone(),
        ident_signature: crypto::encode_bytes(
            &crypto::sign(
                &alice.ident_private,
                &crypto::token_issue_message("alice", &token_fingerprint, "alice#toolbox"),
            )
            .unwrap(),
        ),
    };
    let _ = issue_token(State(state.clone()), Json(issue))
        .await
        .expect("token issued");

    // The token opens its own session and can attest within scope...
    let token_session = open_session_for_key(&store, "alice", &token_private, &token_fingerprint);
    let (in_scope_public, _) = crypto::generate_keypair();
    let _ = signing(
        State(state.clone()),
        peer("127.0.0.1"),
        Json(SigningRequest {
            owner: "alice".to_string(),
            ident: "alice#toolbox".to_string(),
            version: "1.0.0".to_string(),
            signing_fingerprint: crypto::fingerprint(&in_scope_public),
            session_token: token_session.clone(),
        }),
    )
    .await
    .expect("in-scope attestation");

    // ...but not outside its scope.
    let (out_public, _) = crypto::generate_keypair();
    let refused = signing(
        State(state.clone()),
        peer("127.0.0.1"),
        Json(SigningRequest {
            owner: "alice".to_string(),
            ident: "alice#other".to_string(),
            version: "1.0.0".to_string(),
            signing_fingerprint: crypto::fingerprint(&out_public),
            session_token: token_session.clone(),
        }),
    )
    .await;
    assert!(refused.err().unwrap().1.error.contains("scope"));

    // Revocation (ident-authorized) kills the token session.
    let _ = revoke_token(
        State(state.clone()),
        Json(TokenRevokeRequest {
            owner: "alice".to_string(),
            token_fingerprint: token_fingerprint.clone(),
            session_token: owner_token.clone(),
            ident_signature: crypto::encode_bytes(
                &crypto::sign(
                    &alice.ident_private,
                    &crypto::token_revoke_message("alice", &token_fingerprint),
                )
                .unwrap(),
            ),
        }),
    )
    .await
    .expect("token revoked");
    let after = signing(
        State(state.clone()),
        peer("127.0.0.1"),
        Json(SigningRequest {
            owner: "alice".to_string(),
            ident: "alice#toolbox".to_string(),
            version: "2.0.0".to_string(),
            signing_fingerprint: crypto::fingerprint(&in_scope_public),
            session_token: token_session,
        }),
    )
    .await;
    assert!(after.is_err());
}

#[tokio::test]
async fn issue_token_enforces_scope_ownership_and_signature() {
    let h = harness();
    let keys = register_owner_with_all_keys(&h.store, "alice");
    let token = open_session(&h.store, "alice", &keys.auth_private);
    let (token_public, token_private) = crypto::generate_keypair();
    let token_fingerprint = crypto::fingerprint(&token_public);
    let proof = crypto::encode_bytes(
        &crypto::sign(
            &token_private,
            &crypto::registration_message(crypto::ROLE_AUTH, "alice", &token_public),
        )
        .unwrap(),
    );
    let request = |scope: &str| TokenIssueRequest {
        owner: "alice".to_string(),
        token_key: crypto::encode_bytes(&token_public),
        proof: proof.clone(),
        scope: scope.to_string(),
        ttl_seconds: 3600,
        session_token: token.clone(),
        ident_signature: crypto::encode_bytes(
            &crypto::sign(
                &keys.ident_private,
                &crypto::token_issue_message("alice", &token_fingerprint, scope),
            )
            .unwrap(),
        ),
    };

    let mut bad_key = request("alice#*");
    bad_key.token_key = NOT_BASE64.to_string();
    assert_eq!(
        err_of(issue_token(State(h.state.clone()), Json(bad_key)).await).1,
        "malformed tokenKey",
    );
    let mut bad_proof = request("alice#*");
    bad_proof.proof = NOT_BASE64.to_string();
    assert_eq!(
        err_of(issue_token(State(h.state.clone()), Json(bad_proof)).await).1,
        "malformed proof",
    );
    let mut bad_signature = request("alice#*");
    bad_signature.ident_signature = NOT_BASE64.to_string();
    assert_eq!(
        err_of(issue_token(State(h.state.clone()), Json(bad_signature)).await).1,
        "malformed identSignature",
    );
    // A signature over a *different* scope cannot authorize this one.
    let mut swapped_scope = request("alice#toolbox");
    swapped_scope.scope = "alice#*".to_string();
    assert_eq!(
        err_of(issue_token(State(h.state.clone()), Json(swapped_scope)).await).1,
        "invalid token issuance ident signature",
    );
    // Scope must live under the issuing owner — including a scope with no
    // owner separator at all.
    for scope in ["bob#*", "noseparator"] {
        assert_eq!(
            err_of(issue_token(State(h.state.clone()), Json(request(scope))).await).1,
            "token scope must be within the issuing owner",
            "{scope}",
        );
    }
    // Store-level validation still applies (a zero TTL is refused).
    let mut zero_ttl = request("alice#*");
    zero_ttl.ttl_seconds = 0;
    let (status, message) = err_of(issue_token(State(h.state.clone()), Json(zero_ttl)).await);
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(message.contains("ttl"), "{message}");

    let issued = issue_token(State(h.state.clone()), Json(request("alice#*")))
        .await
        .expect("wildcard token issued")
        .0;
    assert_eq!(issued.owner, "alice");
    assert_eq!(issued.token_fingerprint, token_fingerprint);
    assert_eq!(issued.scope, "alice#*");
    assert!(issued.expires_at > now_unix());

    // A wildcard token may attest any package of its owner...
    let session = open_session_for_key(&h.store, "alice", &token_private, &token_fingerprint);
    for ident in ["alice#toolbox", "alice#widgets"] {
        let (signing_public, _private) = crypto::generate_keypair();
        let _ = signing(
            State(h.state.clone()),
            peer("127.0.0.1"),
            Json(SigningRequest {
                owner: "alice".to_string(),
                ident: ident.to_string(),
                version: "1.0.0".to_string(),
                signing_fingerprint: crypto::fingerprint(&signing_public),
                session_token: session.clone(),
            }),
        )
        .await
        .unwrap_or_else(|_| panic!("wildcard scope permits {ident}"));
    }
    // ...but never another owner's, even though the ident owner check
    // would otherwise be satisfied by the session owner alone.
    let (signing_public, _private) = crypto::generate_keypair();
    assert_eq!(
        err_of(
            signing(
                State(h.state),
                peer("127.0.0.1"),
                Json(SigningRequest {
                    owner: "alice".to_string(),
                    ident: "bob#toolbox".to_string(),
                    version: "1.0.0".to_string(),
                    signing_fingerprint: crypto::fingerprint(&signing_public),
                    session_token: session,
                }),
            )
            .await
        )
        .1,
        "publish token scope does not permit this package",
    );
}

#[tokio::test]
async fn an_expired_publish_token_cannot_attest() {
    let h = harness();
    let keys = register_owner_with_all_keys(&h.store, "alice");
    let token = open_session(&h.store, "alice", &keys.auth_private);
    let (token_public, token_private) = crypto::generate_keypair();
    let token_fingerprint = crypto::fingerprint(&token_public);
    let proof = crypto::sign(
        &token_private,
        &crypto::registration_message(crypto::ROLE_AUTH, "alice", &token_public),
    )
    .unwrap();
    let _ = issue_token(
        State(h.state.clone()),
        Json(TokenIssueRequest {
            owner: "alice".to_string(),
            token_key: crypto::encode_bytes(&token_public),
            proof: crypto::encode_bytes(&proof),
            scope: "alice#*".to_string(),
            ttl_seconds: 1,
            session_token: token,
            ident_signature: crypto::encode_bytes(
                &crypto::sign(
                    &keys.ident_private,
                    &crypto::token_issue_message("alice", &token_fingerprint, "alice#*"),
                )
                .unwrap(),
            ),
        }),
    )
    .await
    .expect("one-second token issued");
    let session = open_session_for_key(&h.store, "alice", &token_private, &token_fingerprint);

    // The token's own session JWT is still live (an hour), so only the
    // token expiry can refuse this — which is the point of the check.
    tokio::time::sleep(std::time::Duration::from_millis(2100)).await;
    assert!(verify_session_token(&h.store, &session).is_ok());
    let (signing_public, _private) = crypto::generate_keypair();
    assert_eq!(
        err_of(
            signing(
                State(h.state),
                peer("127.0.0.1"),
                Json(SigningRequest {
                    owner: "alice".to_string(),
                    ident: "alice#toolbox".to_string(),
                    version: "1.0.0".to_string(),
                    signing_fingerprint: crypto::fingerprint(&signing_public),
                    session_token: session,
                }),
            )
            .await
        )
        .1,
        "publish token has expired",
    );
}

/// Issue a publish token for `alice` scoped to `scope` through the real
/// `/tokens` handler; returns the token's private key and fingerprint so
/// the caller can open a session with it.
async fn issue_scoped_token(
    h: &Harness,
    keys: &TestOwnerKeys,
    owner_session: &str,
    scope: &str,
    ttl_seconds: i64,
) -> (Vec<u8>, String) {
    let (token_public, token_private) = crypto::generate_keypair();
    let token_fingerprint = crypto::fingerprint(&token_public);
    let proof = crypto::sign(
        &token_private,
        &crypto::registration_message(crypto::ROLE_AUTH, "alice", &token_public),
    )
    .unwrap();
    let _ = issue_token(
        State(h.state.clone()),
        Json(TokenIssueRequest {
            owner: "alice".to_string(),
            token_key: crypto::encode_bytes(&token_public),
            proof: crypto::encode_bytes(&proof),
            scope: scope.to_string(),
            ttl_seconds,
            session_token: owner_session.to_string(),
            ident_signature: crypto::encode_bytes(
                &crypto::sign(
                    &keys.ident_private,
                    &crypto::token_issue_message("alice", &token_fingerprint, scope),
                )
                .unwrap(),
            ),
        }),
    )
    .await
    .expect("token issued");
    (token_private, token_fingerprint)
}

/// bug-492 (audit-3 REPO-01): a publish token is a *delegated* credential
/// — one package, a TTL, revocable. `/machines/link` accepted any session,
/// so a token session could park a pairing blob and then fetch it with a
/// fresh keypair of its own choosing, minting a permanent, unscoped auth key
/// that outlived the token's revocation. Enrolling a machine is account
/// authority: a token-backed session is refused outright, while an ordinary
/// machine key — including one that was itself linked — still pairs.
#[tokio::test]
async fn a_publish_token_session_cannot_enrol_a_machine() {
    let h = harness();
    let alice = register_owner_with_all_keys(&h.store, "alice");
    let owner_session = open_session(&h.store, "alice", &alice.auth_private);
    let (token_private, token_fingerprint) =
        issue_scoped_token(&h, &alice, &owner_session, "alice#toolbox", 3600).await;
    let token_session = open_session_for_key(&h.store, "alice", &token_private, &token_fingerprint);

    let code = crypto::generate_pairing_code();
    // Both halves derive their pairing approval material from the code, as
    // the real client does (bug-583).
    let start = |session: &str, code: &str| LinkStartRequest {
        owner: "alice".to_string(),
        lookup: crypto::pairing_lookup(code),
        blob: crypto::encode_bytes(&[7u8; 64]),
        salt: crypto::encode_bytes(&[9u8; 16]),
        approval_key: crypto::encode_bytes(&crypto::pairing_approval_keypair(code).unwrap().0),
        session_token: session.to_string(),
    };
    assert_eq!(
        err_of(link_start(State(h.state.clone()), Json(start(&token_session, &code)),).await),
        (
            StatusCode::BAD_REQUEST,
            "a publish token session cannot link a machine".to_string(),
        ),
    );
    // Nothing was parked: the fetch half finds no pairing under that code,
    // so the token cannot complete the self-pairing with a key of its own.
    let (new_public, new_private) = crypto::generate_keypair();
    let proof = crypto::encode_bytes(
        &crypto::sign(
            &new_private,
            &crypto::registration_message(crypto::ROLE_AUTH, "alice", &new_public),
        )
        .unwrap(),
    );
    let fetch = |code: &str| {
        let lookup = crypto::pairing_lookup(code);
        let approval_private = crypto::pairing_approval_keypair(code).unwrap().1;
        LinkFetchRequest {
            approval: crypto::encode_bytes(
                &crypto::sign(
                    &approval_private,
                    &crypto::pairing_approval_message(&lookup, &new_public),
                )
                .unwrap(),
            ),
            owner: "alice".to_string(),
            lookup,
            auth_key: crypto::encode_bytes(&new_public),
            proof: proof.clone(),
        }
    };
    assert_eq!(
        err_of(link_fetch(State(h.state.clone()), Json(fetch(&code))).await).1,
        "unknown, used, or expired pairing code",
    );
    // Refusing the link did not narrow the token further: it still attests
    // within its scope.
    let (signing_public, _) = crypto::generate_keypair();
    let _ = signing(
        State(h.state.clone()),
        peer("127.0.0.1"),
        Json(SigningRequest {
            owner: "alice".to_string(),
            ident: "alice#toolbox".to_string(),
            version: "1.0.0".to_string(),
            signing_fingerprint: crypto::fingerprint(&signing_public),
            session_token: token_session.clone(),
        }),
    )
    .await
    .expect("an in-scope attestation is still issued to the token");

    // A real two-machine link is untouched: the account's machine key parks
    // the blob and the new machine fetches it.
    let _ = link_start(State(h.state.clone()), Json(start(&owner_session, &code)))
        .await
        .expect("a machine-key session still starts a link");
    let fetched = link_fetch(State(h.state.clone()), Json(fetch(&code)))
        .await
        .expect("the new machine still fetches the pairing")
        .0;
    assert_eq!(fetched.auth_fingerprint, crypto::fingerprint(&new_public));
    // ...and the newly linked key is a full machine key, not a token: it may
    // itself enrol the next machine.
    let linked_session =
        open_session_for_key(&h.store, "alice", &new_private, &fetched.auth_fingerprint);
    let next_code = crypto::generate_pairing_code();
    let _ = link_start(
        State(h.state.clone()),
        Json(start(&linked_session, &next_code)),
    )
    .await
    .expect("a linked machine key can pair another machine");
}

/// bug-492, the other half: a token's scope and expiry were consulted by
/// `/signing` alone, so every other authenticated route treated a token
/// session as the account itself. They are a property of the *session* now
/// — `/validate` and `/publish` refuse an out-of-scope package, and an
/// expired token is refused at `PUT /blob` and at `/auth/login` even while
/// its session JWT is still live (the JWT outlives a short token by design,
/// which is exactly why the token row has to be re-read per request).
#[tokio::test]
async fn a_publish_token_is_bounded_on_every_authenticated_route() {
    let h = harness();
    let alice = register_owner_with_all_keys(&h.store, "alice");
    let owner_session = open_session(&h.store, "alice", &alice.auth_private);

    // A token for a *different* package cannot validate or publish
    // alice#toolbox even with a perfectly valid, owner-attested artifact.
    let (other_private, other_fingerprint) =
        issue_scoped_token(&h, &alice, &owner_session, "alice#other", 3600).await;
    let other_session = open_session_for_key(&h.store, "alice", &other_private, &other_fingerprint);
    let (_artifact, mut request) = signed_request(
        &h.state,
        &alice,
        &owner_session,
        "1.0.0",
        b"MFPCtestpayload".to_vec(),
    )
    .await;
    request.session_token = other_session.clone();
    let refused = (
        StatusCode::BAD_REQUEST,
        "publish token scope does not permit this package".to_string(),
    );
    assert_eq!(
        err_of(validate_package(State(h.state.clone()), Json(clone_request(&request))).await),
        refused,
    );
    assert_eq!(
        err_of(publish_package(State(h.state.clone()), Json(clone_request(&request))).await),
        refused,
    );
    assert!(!h
        .store
        .package_version_exists("alice#toolbox", "1.0.0")
        .unwrap());
    // Scope is about packages; a content-addressed blob upload has none, so
    // a live token may still stage native blobs.
    let body = b"token-upload".to_vec();
    let hash = hex::encode(crypto::sha256(&body));
    let put = |session: &str| {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {session}").parse().unwrap(),
        );
        put_blob(
            State(h.state.clone()),
            axum::extract::Path(hash.clone()),
            headers,
            axum::body::Bytes::from(body.clone()),
        )
    };
    assert_eq!(put(&other_session).await.unwrap(), StatusCode::CREATED);

    // A one-second token: its session JWT is minted for an hour, so after
    // the token lapses only the token expiry can refuse these.
    let (short_private, short_fingerprint) =
        issue_scoped_token(&h, &alice, &owner_session, "alice#toolbox", 1).await;
    let short_session = open_session_for_key(&h.store, "alice", &short_private, &short_fingerprint);
    tokio::time::sleep(std::time::Duration::from_millis(2100)).await;
    assert!(verify_session_token(&h.store, &short_session).is_ok());
    assert_eq!(
        err_of(put(&short_session).await),
        (
            StatusCode::UNAUTHORIZED,
            "publish token has expired".to_string()
        ),
    );
    let challenge = h
        .store
        .create_auth_challenge("alice", &short_fingerprint)
        .unwrap();
    let signature = crypto::sign(
        &short_private,
        &crypto::challenge_message(&challenge.id, &challenge.nonce),
    )
    .unwrap();
    assert_eq!(
        err_of(
            login(
                State(h.state.clone()),
                peer("127.0.0.1"),
                Json(LoginRequest {
                    challenge_id: challenge.id,
                    signature: crypto::encode_bytes(&signature),
                }),
            )
            .await
        ),
        (
            StatusCode::BAD_REQUEST,
            "publish token has expired".to_string()
        ),
    );
}

#[test]
fn token_scope_matching_is_owner_folded_and_separator_strict() {
    assert!(scope_owner_matches("alice#toolbox", "alice"));
    assert!(scope_owner_matches("Alice#toolbox", "alice"));
    assert!(!scope_owner_matches("bob#toolbox", "alice"));
    assert!(!scope_owner_matches("noseparator", "alice"));

    assert!(scope_permits("alice#*", "alice#toolbox", "alice"));
    assert!(scope_permits("Alice#*", "alice#toolbox", "alice"));
    assert!(scope_permits("alice#toolbox", "alice#toolbox", "alice"));
    assert!(!scope_permits("alice#toolbox", "alice#other", "alice"));
    assert!(!scope_permits("alice#*", "bob#toolbox", "bob"));
    // A scope or an ident without the separator matches nothing.
    assert!(!scope_permits("noseparator", "alice#toolbox", "alice"));
    assert!(!scope_permits("alice#*", "noseparator", "alice"));
    // plan-168-B: a wildcard follows the governing (stored) owner, not
    // the ident prefix.
    assert!(scope_permits("bob#*", "alice#toolbox", "bob"));
    assert!(!scope_permits("alice#*", "alice#toolbox", "bob"));
}
