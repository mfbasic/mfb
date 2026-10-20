//! plan-176-I: `server::tests` — the transfers tests. The shared harnesses (owners, transfers,\n//! MFPC builders, signed requests) live in `server/tests/mod.rs`.

use super::*;

#[tokio::test]
async fn ownership_transfer_is_two_sided_and_rebinds_the_package() {
    let temp = tempfile::tempdir().unwrap();
    let opened =
        Store::open_repository(&temp.path().join("meta.db"), &temp.path().join("data")).unwrap();
    let store = opened.store;
    let alice = register_owner_with_all_keys(&store, "alice");
    let bob = register_owner_with_all_keys(&store, "bob");
    let alice_token = open_session(&store, "alice", &alice.auth_private);
    let bob_token = open_session(&store, "bob", &bob.auth_private);
    let state = AppState {
        store: store.clone(),
        blob_store: BlobStore::local(opened.packages_dir.clone()),
        rate_limiter: RateLimiter::new(),
    };
    publish_valid_package(&state, &alice, &alice_token, "1.0.0").await;

    // An offer signed by the wrong ident is refused.
    let bad_offer = signed_offer(&state, "alice#toolbox", "alice", &bob, &alice_token, "bob").await;
    assert!(transfer_offer(State(state.clone()), Json(bad_offer))
        .await
        .is_err());

    // A correctly signed offer, then a bob-signed acceptance, re-binds it.
    let offer = signed_offer(
        &state,
        "alice#toolbox",
        "alice",
        &alice,
        &alice_token,
        "bob",
    )
    .await;
    let _ = transfer_offer(State(state.clone()), Json(offer))
        .await
        .expect("offer");
    let accept = signed_accept(&state, "alice#toolbox", "bob", &bob, &bob_token).await;
    let _ = transfer_accept(State(state.clone()), Json(accept))
        .await
        .expect("accept");

    // The package is re-bound to bob; the already-published version persists.
    assert_eq!(
        store
            .package_owner("alice#toolbox")
            .unwrap()
            .unwrap()
            .owner_display,
        "bob"
    );
    assert_eq!(
        store.list_package_versions("alice#toolbox").unwrap().len(),
        1
    );
}

/// plan-168-A: a recipient can pass the package on. The second offer is
/// signed by bob — the stored owner — and the former owner is refused.
#[tokio::test]
async fn a_second_transfer_is_offered_by_the_current_owner() {
    let h = harness();
    let alice = register_owner_with_all_keys(&h.store, "alice");
    let bob = register_owner_with_all_keys(&h.store, "bob");
    let carol = register_owner_with_all_keys(&h.store, "carol");
    let alice_token = open_session(&h.store, "alice", &alice.auth_private);
    let bob_token = open_session(&h.store, "bob", &bob.auth_private);
    let carol_token = open_session(&h.store, "carol", &carol.auth_private);
    publish_valid_package(&h.state, &alice, &alice_token, "1.0.0").await;
    transfer_through_handlers(
        &h.state,
        "alice#toolbox",
        "alice",
        &alice,
        &alice_token,
        "bob",
        &bob,
        &bob_token,
    )
    .await;

    // alice, the ident prefix but no longer the owner, cannot offer.
    let stale = signed_offer(
        &h.state,
        "alice#toolbox",
        "alice",
        &alice,
        &alice_token,
        "carol",
    )
    .await;
    assert_eq!(
        err_of(transfer_offer(State(h.state.clone()), Json(stale)).await).1,
        "offering owner does not currently own the package; it is owned by bob",
    );
    transfer_through_handlers(
        &h.state,
        "alice#toolbox",
        "bob",
        &bob,
        &bob_token,
        "carol",
        &carol,
        &carol_token,
    )
    .await;
    assert_eq!(
        h.store
            .package_owner("alice#toolbox")
            .unwrap()
            .unwrap()
            .owner_display,
        "carol"
    );
    let history = h.store.transfer_history("alice#toolbox").unwrap();
    let links: Vec<_> = history
        .iter()
        .map(|r| (r.sequence, r.from_owner.as_str(), r.to_owner.as_str()))
        .collect();
    assert_eq!(links, vec![(0, "alice", "bob"), (1, "bob", "carol")]);
}

/// plan-168-A: a recorded link cannot be replayed. After alice -> bob ->
/// alice, re-POSTing the exact seq-0 offer bytes (alice -> bob) is refused:
/// it names position 0, and the next link is position 2.
#[tokio::test]
async fn a_replayed_link_is_refused() {
    let h = harness();
    let alice = register_owner_with_all_keys(&h.store, "alice");
    let bob = register_owner_with_all_keys(&h.store, "bob");
    let alice_token = open_session(&h.store, "alice", &alice.auth_private);
    let bob_token = open_session(&h.store, "bob", &bob.auth_private);
    publish_valid_package(&h.state, &alice, &alice_token, "1.0.0").await;
    let recorded = signed_offer(
        &h.state,
        "alice#toolbox",
        "alice",
        &alice,
        &alice_token,
        "bob",
    )
    .await;
    let replay = TransferOfferRequest {
        ident: recorded.ident.clone(),
        from_owner: recorded.from_owner.clone(),
        to_owner: recorded.to_owner.clone(),
        sequence: recorded.sequence,
        release_digest: recorded.release_digest.clone(),
        session_token: recorded.session_token.clone(),
        ident_signature: recorded.ident_signature.clone(),
    };
    let _ = transfer_offer(State(h.state.clone()), Json(recorded))
        .await
        .expect("offer");
    let accept = signed_accept(&h.state, "alice#toolbox", "bob", &bob, &bob_token).await;
    let recorded_accept = TransferAcceptRequest {
        ident: accept.ident.clone(),
        to_owner: accept.to_owner.clone(),
        session_token: accept.session_token.clone(),
        ident_signature: accept.ident_signature.clone(),
    };
    let _ = transfer_accept(State(h.state.clone()), Json(accept))
        .await
        .expect("accept");
    transfer_through_handlers(
        &h.state,
        "alice#toolbox",
        "bob",
        &bob,
        &bob_token,
        "alice",
        &alice,
        &alice_token,
    )
    .await;

    // The replayed offer names a position that is no longer next.
    let (status, message) = err_of(transfer_offer(State(h.state.clone()), Json(replay)).await);
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(message.contains("transfer terms are stale"), "{message}");
    // And the recorded acceptance does not verify over today's terms.
    let (status, message) =
        err_of(transfer_accept(State(h.state.clone()), Json(recorded_accept)).await);
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(message, "invalid transfer accept ident signature");
    assert_eq!(
        h.store
            .package_owner("alice#toolbox")
            .unwrap()
            .unwrap()
            .owner_display,
        "alice"
    );
    assert_eq!(h.store.transfer_history("alice#toolbox").unwrap().len(), 2);
}

/// plan-168-A Phase 3: `/index` serves the transfer as a link whose every
/// signature verifies — the offer under alice's key, the accept under bob's,
/// and bob's current binding under the server key — and tags each release
/// with its publisher and epoch.
#[tokio::test]
async fn the_index_serves_a_verifiable_transfer_chain() {
    let h = harness();
    let alice = register_owner_with_all_keys(&h.store, "alice");
    let bob = register_owner_with_all_keys(&h.store, "bob");
    let alice_token = open_session(&h.store, "alice", &alice.auth_private);
    let bob_token = open_session(&h.store, "bob", &bob.auth_private);
    publish_valid_package(&h.state, &alice, &alice_token, "1.0.0").await;
    transfer_through_handlers(
        &h.state,
        "alice#toolbox",
        "alice",
        &alice,
        &alice_token,
        "bob",
        &bob,
        &bob_token,
    )
    .await;

    let index = package_index(
        State(h.state.clone()),
        axum::extract::Path("alice#toolbox".to_string()),
    )
    .await
    .expect("index served")
    .0;
    // The top-level binding still describes the ident-prefix owner.
    assert_eq!(index.owner, "alice");
    assert_eq!(
        index.ident_fingerprint,
        crypto::fingerprint(&alice.ident_public)
    );
    assert_eq!(index.transfers.len(), 1);
    let link = &index.transfers[0];
    assert_eq!(
        (
            link.sequence,
            link.from_owner.as_str(),
            link.to_owner.as_str()
        ),
        (0, "alice", "bob")
    );
    let key = |value: &Option<String>| {
        package::decode_metadata_key(value.as_deref().unwrap(), "key").unwrap()
    };
    let from_key = key(&link.from_ident_key);
    let to_key = key(&link.to_ident_key);
    assert_eq!(from_key, alice.ident_public);
    assert_eq!(to_key, bob.ident_public);
    let digest = link.release_digest.clone().unwrap();
    assert_eq!(
        digest,
        crypto::release_digest(
            index
                .versions
                .iter()
                .map(|v| (v.version.as_str(), v.hash.as_str()))
        )
    );
    let from_fp = crypto::fingerprint(&from_key);
    let terms = crypto::TransferTermsRef {
        ident: "alice#toolbox",
        sequence: 0,
        from_owner: &link.from_owner,
        from_ident_fingerprint: &from_fp,
        to_owner: &link.to_owner,
        release_digest: &digest,
    };
    let signature = |value: &Option<String>| {
        crypto::decode_bytes(value.as_deref().unwrap(), "signature").unwrap()
    };
    crypto::verify(
        &from_key,
        &crypto::transfer_offer_message_v2(terms),
        &signature(&link.offer_signature),
    )
    .expect("the offer verifies under alice's key");
    crypto::verify(
        &to_key,
        &crypto::transfer_accept_message_v2(terms),
        &signature(&link.accept_signature),
    )
    .expect("the accept verifies under bob's key");
    let server_public = h.store.server_public_key().unwrap();
    let bound = crate::client::verify_owner_binding(&server_public, &link.to_binding)
        .expect("the toBinding verifies under the server key");
    assert_eq!(bound, bob.ident_public);
    assert_eq!(link.to_binding.owner, "bob");

    assert_eq!(index.versions.len(), 1);
    let publisher = index.versions[0].publisher.clone().unwrap();
    assert_eq!(publisher.owner, "alice");
    assert_eq!(
        publisher.ident_fingerprint,
        Some(crypto::fingerprint(&alice.ident_public))
    );
    assert_eq!(index.versions[0].epoch, 0);
}

/// plan-168-A Phase 3: a pre-plan transfer chain, replayed from the log by
/// `migrate`, is served as unsigned links carrying their real owners and
/// positions — and with a current binding for each recipient.
#[tokio::test]
async fn a_legacy_transfer_is_served_unsigned() {
    let h = harness();
    let alice = register_owner_with_all_keys(&h.store, "alice");
    let bob = register_owner_with_all_keys(&h.store, "bob");
    register_owner_with_all_keys(&h.store, "carol");
    let alice_token = open_session(&h.store, "alice", &alice.auth_private);
    publish_valid_package(&h.state, &alice, &alice_token, "1.0.0").await;
    crate::store::tests::legacy_transfer(&h.store, "alice#toolbox", "alice", "bob");
    crate::store::tests::legacy_transfer(&h.store, "alice#toolbox", "bob", "carol");
    h.store.migrate().unwrap();

    let index = package_index(
        State(h.state.clone()),
        axum::extract::Path("alice#toolbox".to_string()),
    )
    .await
    .expect("index served")
    .0;
    let links: Vec<_> = index
        .transfers
        .iter()
        .map(|l| (l.sequence, l.from_owner.as_str(), l.to_owner.as_str()))
        .collect();
    assert_eq!(links, vec![(0, "alice", "bob"), (1, "bob", "carol")]);
    for link in &index.transfers {
        assert_eq!(link.from_ident_key, None);
        assert_eq!(link.to_ident_key, None);
        assert_eq!(link.release_digest, None);
        assert_eq!(link.offer_signature, None);
        assert_eq!(link.accept_signature, None);
    }
    let server_public = h.store.server_public_key().unwrap();
    assert_eq!(
        crate::client::verify_owner_binding(&server_public, &index.transfers[0].to_binding)
            .unwrap(),
        bob.ident_public
    );
    assert_eq!(index.versions[0].publisher.as_ref().unwrap().owner, "alice");
    assert_eq!(index.versions[0].epoch, 0);
}

/// plan-168-A Phase 3: the package page names the stored owner and shows
/// THAT owner's key. It used to pair "bob" with alice's (prefix) key.
#[tokio::test]
async fn the_package_page_pairs_the_current_owner_with_their_key() {
    let h = harness();
    let alice = register_owner_with_all_keys(&h.store, "alice");
    let bob = register_owner_with_all_keys(&h.store, "bob");
    let alice_token = open_session(&h.store, "alice", &alice.auth_private);
    let bob_token = open_session(&h.store, "bob", &bob.auth_private);
    publish_valid_package(&h.state, &alice, &alice_token, "1.0.0").await;
    transfer_through_handlers(
        &h.state,
        "alice#toolbox",
        "alice",
        &alice,
        &alice_token,
        "bob",
        &bob,
        &bob_token,
    )
    .await;
    let detail = package_detail(
        State(h.state.clone()),
        axum::extract::Path("alice#toolbox".to_string()),
    )
    .await
    .expect("detail served")
    .0;
    assert_eq!(detail.owner, "bob");
    assert_eq!(
        detail.ident_fingerprint,
        crypto::fingerprint(&bob.ident_public)
    );
    assert_eq!(
        detail.ident_key,
        format!("ed25519:{}", crypto::encode_bytes(&bob.ident_public))
    );
}

/// bug-792's reproduction, landed as written in the bug document except that
/// the transfer goes through the real signed handlers (plan-168-B Phase 1):
/// the recipient of a transferred package attests and validates a new
/// release under the unchanged ident.
#[tokio::test]
async fn spec_probe_transfer_recipient_can_publish() {
    let h = harness();
    let alice = register_owner_with_all_keys(&h.store, "alice");
    let bob = register_owner_with_all_keys(&h.store, "bob");
    let alice_token = open_session(&h.store, "alice", &alice.auth_private);
    let bob_token = open_session(&h.store, "bob", &bob.auth_private);
    publish_valid_package(&h.state, &alice, &alice_token, "1.0.0").await;
    transfer_through_handlers(
        &h.state,
        "alice#toolbox",
        "alice",
        &alice,
        &alice_token,
        "bob",
        &bob,
        &bob_token,
    )
    .await;
    assert_eq!(
        h.store
            .package_owner("alice#toolbox")
            .unwrap()
            .unwrap()
            .owner_display,
        "bob"
    );
    let (pubkey, private) = crypto::generate_keypair();
    let idfp = crypto::fingerprint(&bob.ident_public);
    let signfp = crypto::fingerprint(&pubkey);
    let attestation = signing(
        State(h.state.clone()),
        peer("127.0.0.1"),
        Json(SigningRequest {
            owner: "bob".into(),
            ident: "alice#toolbox".into(),
            version: "2.0.0".into(),
            signing_fingerprint: signfp.clone(),
            session_token: bob_token.clone(),
        }),
    )
    .await
    .unwrap()
    .0;
    let proof = serde_json::json!({"owner":"bob", "ident":"alice#toolbox", "version":"2.0.0",
        "identFingerprint":idfp, "signingFingerprint":signfp, "issued":now_unix()})
    .to_string();
    let proof_sig = crypto::sign(
        &bob.ident_private,
        &crypto::proof_signing_input(proof.as_bytes()),
    )
    .unwrap();
    let artifact = package::test_support::serialize(
        &package::test_support::TestPackage {
            name: "toolbox".into(),
            ident: "alice#toolbox".into(),
            version: "2.0.0".into(),
            author: "bob".into(),
            url: String::new(),
            payload: b"MFPCtestpayload".to_vec(),
            ident_key: format!("ed25519:{}", crypto::encode_bytes(&bob.ident_public)),
            signing_key: format!("ed25519:{}", crypto::encode_bytes(&pubkey)),
            proof,
            proof_sig,
            attestation: attestation.attestation,
            attestation_sig: crypto::decode_bytes(&attestation.attestation_signature, "sig")
                .unwrap(),
        },
        &private,
    );
    let parsed = package::parse_mfp_package(&artifact).unwrap();
    let report = validate_package(
        State(h.state.clone()),
        Json(PackageArtifactRequest {
            ident: parsed.ident.clone(),
            version: parsed.version.clone(),
            artifact: crypto::encode_bytes(&artifact),
            content_hash: parsed.content_hash_hex(),
            ident_fingerprint: idfp,
            signing_fingerprint: signfp,
            session_token: bob_token,
        }),
    )
    .await
    .unwrap()
    .0;
    assert!(
        report.valid,
        "recipient validation failed: {:?}",
        report.diagnostics
    );
}

async fn validation_diagnostics(state: &AppState, request: PackageArtifactRequest) -> Vec<String> {
    validate_package(State(state.clone()), Json(request))
        .await
        .expect("validation report")
        .0
        .diagnostics
}

/// plan-168-B contrast: after the transfer the former owner — still the
/// ident prefix, holding a genuine attestation issued before she gave the
/// package away — is refused by `/validate` itself.
#[tokio::test]
async fn the_former_owner_cannot_publish_after_transfer() {
    let h = harness();
    let alice = register_owner_with_all_keys(&h.store, "alice");
    let bob = register_owner_with_all_keys(&h.store, "bob");
    let alice_token = open_session(&h.store, "alice", &alice.auth_private);
    let bob_token = open_session(&h.store, "bob", &bob.auth_private);
    publish_valid_package(&h.state, &alice, &alice_token, "1.0.0").await;
    let stale = owner_signed_request(
        &h.state,
        "alice",
        &alice,
        &alice_token,
        "alice#toolbox",
        "2.0.0",
    )
    .await;
    transfer_through_handlers(
        &h.state,
        "alice#toolbox",
        "alice",
        &alice,
        &alice_token,
        "bob",
        &bob,
        &bob_token,
    )
    .await;
    let diagnostics = validation_diagnostics(&h.state, stale).await;
    assert!(
        diagnostics.contains(&"session owner is not the package's current owner".to_string()),
        "{diagnostics:?}"
    );
}

/// plan-168-B contrast: with no package row yet, the ident prefix still
/// decides who may publish, with the existing diagnostic.
#[tokio::test]
async fn first_publish_is_still_prefix_bound() {
    let h = harness();
    let alice = register_owner_with_all_keys(&h.store, "alice");
    let bob = register_owner_with_all_keys(&h.store, "bob");
    let alice_token = open_session(&h.store, "alice", &alice.auth_private);
    let bob_token = open_session(&h.store, "bob", &bob.auth_private);
    // bob cannot even be attested for alice's new ident, so his package
    // carries the attestation he got for his own ident.
    let request = owner_signed_request_attested_for(
        &h.state,
        "bob",
        &bob,
        &bob_token,
        "alice#newpkg",
        "bob#newpkg",
        "1.0.0",
    )
    .await;
    let diagnostics = validation_diagnostics(&h.state, request).await;
    assert!(
        diagnostics.contains(&"session owner does not match package ident owner".to_string()),
        "{diagnostics:?}"
    );
    let own = owner_signed_request(
        &h.state,
        "alice",
        &alice,
        &alice_token,
        "alice#newpkg",
        "1.0.0",
    )
    .await;
    assert_eq!(
        validation_diagnostics(&h.state, own).await,
        Vec::<String>::new()
    );
}

/// plan-168-B contrast (B-D3): past an unsigned legacy transfer no client
/// can verify a later owner's release, so the registry refuses to publish
/// one rather than list a release nobody can install.
#[tokio::test]
async fn a_legacy_recipient_cannot_publish_an_unverifiable_release() {
    let h = harness();
    let alice = register_owner_with_all_keys(&h.store, "alice");
    let bob = register_owner_with_all_keys(&h.store, "bob");
    let alice_token = open_session(&h.store, "alice", &alice.auth_private);
    let bob_token = open_session(&h.store, "bob", &bob.auth_private);
    publish_valid_package(&h.state, &alice, &alice_token, "1.0.0").await;
    crate::store::tests::legacy_transfer(&h.store, "alice#toolbox", "alice", "bob");
    h.store.migrate().unwrap();
    let request =
        owner_signed_request(&h.state, "bob", &bob, &bob_token, "alice#toolbox", "2.0.0").await;
    let diagnostics = validation_diagnostics(&h.state, request).await;
    assert_eq!(
        diagnostics,
        vec![
            "package ownership moved before signed transfers, so clients cannot verify a \
                 release by a later owner"
                .to_string()
        ],
    );
}

/// plan-168-B contrast (B-D3): after an unsigned alice -> bob -> alice round
/// trip, the origin owner publishes exactly as she did before plan-168 —
/// clients verify her under the prefix-owner fallback.
#[tokio::test]
async fn the_origin_owner_publishes_after_a_legacy_round_trip() {
    let h = harness();
    let alice = register_owner_with_all_keys(&h.store, "alice");
    register_owner_with_all_keys(&h.store, "bob");
    let alice_token = open_session(&h.store, "alice", &alice.auth_private);
    publish_valid_package(&h.state, &alice, &alice_token, "1.0.0").await;
    crate::store::tests::legacy_transfer(&h.store, "alice#toolbox", "alice", "bob");
    crate::store::tests::legacy_transfer(&h.store, "alice#toolbox", "bob", "alice");
    h.store.migrate().unwrap();
    let request = owner_signed_request(
        &h.state,
        "alice",
        &alice,
        &alice_token,
        "alice#toolbox",
        "2.0.0",
    )
    .await;
    assert_eq!(
        validation_diagnostics(&h.state, request).await,
        Vec::<String>::new()
    );
}

/// Issue a publish token for `owner` scoped to `scope` through the real
/// `/tokens` handler and open a session with it.
async fn token_session_for(
    h: &Harness,
    owner: &str,
    keys: &TestOwnerKeys,
    owner_session: &str,
    scope: &str,
) -> Result<String, (StatusCode, String)> {
    let (token_public, token_private) = crypto::generate_keypair();
    let token_fingerprint = crypto::fingerprint(&token_public);
    let proof = crypto::sign(
        &token_private,
        &crypto::registration_message(crypto::ROLE_AUTH, owner, &token_public),
    )
    .unwrap();
    let issued = issue_token(
        State(h.state.clone()),
        Json(TokenIssueRequest {
            owner: owner.to_string(),
            token_key: crypto::encode_bytes(&token_public),
            proof: crypto::encode_bytes(&proof),
            scope: scope.to_string(),
            ttl_seconds: 3600,
            session_token: owner_session.to_string(),
            ident_signature: crypto::encode_bytes(
                &crypto::sign(
                    &keys.ident_private,
                    &crypto::token_issue_message(owner, &token_fingerprint, scope),
                )
                .unwrap(),
            ),
        }),
    )
    .await;
    if let Err(err) = issued {
        return Err(err_of::<()>(Err(err)));
    }
    Ok(open_session_for_key(
        &h.store,
        owner,
        &token_private,
        &token_fingerprint,
    ))
}

/// plan-168-B (B-D2): publish-token scopes follow the stored owner. After
/// the transfer bob can scope a CI token to `alice#toolbox` (exactly, or
/// through `bob#*`) and attest with it; alice can no longer issue one.
#[tokio::test]
async fn a_recipient_token_scoped_to_the_transferred_package_can_attest() {
    let h = harness();
    let alice = register_owner_with_all_keys(&h.store, "alice");
    let bob = register_owner_with_all_keys(&h.store, "bob");
    let alice_token = open_session(&h.store, "alice", &alice.auth_private);
    let bob_token = open_session(&h.store, "bob", &bob.auth_private);
    publish_valid_package(&h.state, &alice, &alice_token, "1.0.0").await;
    transfer_through_handlers(
        &h.state,
        "alice#toolbox",
        "alice",
        &alice,
        &alice_token,
        "bob",
        &bob,
        &bob_token,
    )
    .await;
    let attest = |session: String| SigningRequest {
        owner: "bob".to_string(),
        ident: "alice#toolbox".to_string(),
        version: "2.0.0".to_string(),
        signing_fingerprint: crypto::fingerprint(&crypto::generate_keypair().0),
        session_token: session,
    };
    for scope in ["alice#toolbox", "bob#*"] {
        let session = token_session_for(&h, "bob", &bob, &bob_token, scope)
            .await
            .unwrap_or_else(|err| panic!("bob may scope a token to {scope}: {err:?}"));
        let attested = signing(
            State(h.state.clone()),
            peer("127.0.0.1"),
            Json(attest(session)),
        )
        .await
        .unwrap_or_else(|_| panic!("a {scope} token attests the transferred package"))
        .0;
        assert_eq!(attested.owner, "bob");
    }
    // A token for one of bob's own packages does not reach this one.
    let session = token_session_for(&h, "bob", &bob, &bob_token, "bob#other")
        .await
        .unwrap();
    assert_eq!(
        err_of(
            signing(
                State(h.state.clone()),
                peer("127.0.0.1"),
                Json(attest(session))
            )
            .await
        )
        .1,
        "publish token scope does not permit this package",
    );
    // The former owner cannot scope a token to the package any more.
    assert_eq!(
        token_session_for(&h, "alice", &alice, &alice_token, "alice#toolbox")
            .await
            .unwrap_err()
            .1,
        "token scope must be within the issuing owner",
    );
}

/// plan-168-B: bob's release of the transferred package publishes, and
/// `/index` records him as its publisher in ownership epoch 1 — while the
/// pre-transfer release keeps alice at epoch 0.
#[tokio::test]
async fn the_publish_records_bob_as_publisher_at_epoch_1() {
    let h = harness();
    let alice = register_owner_with_all_keys(&h.store, "alice");
    let bob = register_owner_with_all_keys(&h.store, "bob");
    let alice_token = open_session(&h.store, "alice", &alice.auth_private);
    let bob_token = open_session(&h.store, "bob", &bob.auth_private);
    publish_valid_package(&h.state, &alice, &alice_token, "1.0.0").await;
    transfer_through_handlers(
        &h.state,
        "alice#toolbox",
        "alice",
        &alice,
        &alice_token,
        "bob",
        &bob,
        &bob_token,
    )
    .await;
    let request =
        owner_signed_request(&h.state, "bob", &bob, &bob_token, "alice#toolbox", "2.0.0").await;
    let published = publish_package(State(h.state.clone()), Json(request))
        .await
        .expect("the recipient publishes")
        .0;
    assert_eq!(published.version, "2.0.0");

    let index = package_index(
        State(h.state.clone()),
        axum::extract::Path("alice#toolbox".to_string()),
    )
    .await
    .expect("index served")
    .0;
    let summary: Vec<_> = index
        .versions
        .iter()
        .map(|v| {
            let publisher = v.publisher.clone().unwrap();
            (
                v.version.clone(),
                publisher.owner,
                publisher.ident_fingerprint,
                v.epoch,
            )
        })
        .collect();
    assert_eq!(
        summary,
        vec![
            (
                "1.0.0".to_string(),
                "alice".to_string(),
                Some(crypto::fingerprint(&alice.ident_public)),
                0
            ),
            (
                "2.0.0".to_string(),
                "bob".to_string(),
                Some(crypto::fingerprint(&bob.ident_public)),
                1
            ),
        ]
    );
}

/// bug-494 (audit-3 REPO-03): `/release-state` and `/signing` authorized
/// on the ident's `owner#` prefix, which never moves when ownership does.
/// After a correctly guarded transfer the former owner could still yank the
/// package (a permanent denial of something she handed over) and obtain a
/// registry attestation for it, while the real owner was locked out of
/// both. Authorization is on `packages.owner_id` now; the prefix check is
/// kept only for a package with no row yet — first publish, which
/// `/signing` must keep serving.
#[tokio::test]
async fn a_transferred_package_is_governed_by_its_current_owner() {
    let h = harness();
    let alice = register_owner_with_all_keys(&h.store, "alice");
    let bob = register_owner_with_all_keys(&h.store, "bob");
    let alice_token = open_session(&h.store, "alice", &alice.auth_private);
    let bob_token = open_session(&h.store, "bob", &bob.auth_private);
    publish_valid_package(&h.state, &alice, &alice_token, "1.0.0").await;

    // Hand the package to bob through the real two-sided handshake.
    transfer_through_handlers(
        &h.state,
        "alice#toolbox",
        "alice",
        &alice,
        &alice_token,
        "bob",
        &bob,
        &bob_token,
    )
    .await;
    assert_eq!(
        h.store
            .package_owner("alice#toolbox")
            .unwrap()
            .unwrap()
            .owner_display,
        "bob"
    );

    let release =
        |owner: &str, keys: &TestOwnerKeys, session: &str, new_state: &str| ReleaseStateRequest {
            owner: owner.to_string(),
            ident: "alice#toolbox".to_string(),
            version: "1.0.0".to_string(),
            state: new_state.to_string(),
            session_token: session.to_string(),
            ident_signature: crypto::encode_bytes(
                &crypto::sign(
                    &keys.ident_private,
                    &crypto::release_state_message("alice#toolbox", "1.0.0", new_state),
                )
                .unwrap(),
            ),
        };
    let attest = |owner: &str, session: &str, ident: &str, version: &str| SigningRequest {
        owner: owner.to_string(),
        ident: ident.to_string(),
        version: version.to_string(),
        signing_fingerprint: crypto::fingerprint(&crypto::generate_keypair().0),
        session_token: session.to_string(),
    };
    let not_owner = (
        StatusCode::BAD_REQUEST,
        "package is not owned by the session owner".to_string(),
    );

    // The former owner, with a perfectly valid ident signature, can neither
    // change the release state nor obtain an attestation.
    assert_eq!(
        err_of(
            release_state(
                State(h.state.clone()),
                Json(release("alice", &alice, &alice_token, "yanked")),
            )
            .await
        ),
        not_owner,
    );
    assert_eq!(
        h.store.list_package_versions("alice#toolbox").unwrap()[0].state,
        "available",
        "a refused yank must leave the version untouched",
    );
    assert_eq!(
        err_of(
            signing(
                State(h.state.clone()),
                peer("127.0.0.1"),
                Json(attest("alice", &alice_token, "alice#toolbox", "2.0.0")),
            )
            .await
        ),
        not_owner,
    );

    // The current owner can do both, even though the ident still carries
    // the former owner's name.
    let yanked = release_state(
        State(h.state.clone()),
        Json(release("bob", &bob, &bob_token, "yanked")),
    )
    .await
    .expect("the current owner yanks")
    .0;
    assert_eq!(yanked.state, "yanked");
    assert_eq!(
        h.store.list_package_versions("alice#toolbox").unwrap()[0].state,
        "yanked"
    );
    let _ = release_state(
        State(h.state.clone()),
        Json(release("bob", &bob, &bob_token, "available")),
    )
    .await
    .expect("the current owner un-yanks");
    let attested = signing(
        State(h.state.clone()),
        peer("127.0.0.1"),
        Json(attest("bob", &bob_token, "alice#toolbox", "2.0.0")),
    )
    .await
    .expect("the current owner is attested")
    .0;
    assert_eq!(attested.owner, "bob");

    // First publish is untouched: with no `packages` row yet, the ident's
    // owner prefix still decides, so alice attests a brand-new package of
    // hers and bob is refused for it exactly as before.
    let _ = signing(
        State(h.state.clone()),
        peer("127.0.0.1"),
        Json(attest("alice", &alice_token, "alice#brandnew", "1.0.0")),
    )
    .await
    .expect("first-publish attestation for the prefix owner");
    assert_eq!(
        err_of(
            signing(
                State(h.state.clone()),
                peer("127.0.0.1"),
                Json(attest("bob", &bob_token, "alice#brandnew", "1.0.0")),
            )
            .await
        )
        .1,
        "ident owner does not match session owner",
    );
}
