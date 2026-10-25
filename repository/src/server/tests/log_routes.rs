//! plan-176-I: `server::tests` — the log_routes tests. The shared harnesses (owners, transfers,\n//! MFPC builders, signed requests) live in `server/tests/mod.rs`.

use super::*;

#[tokio::test]
async fn log_endpoints_serve_verifiable_checkpoints_and_proofs() {
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
    // Grow the log: register (1) + three attestations (4 total).
    for version in ["1.0.0", "1.1.0", "1.2.0"] {
        let (signing_public, _signing_private) = crypto::generate_keypair();
        real_attestation(
            &state,
            &token,
            version,
            &crypto::fingerprint(&signing_public),
        )
        .await;
    }
    let checkpoint_small = log_checkpoint(State(state.clone()), peer("127.0.0.1"))
        .await
        .unwrap()
        .0;
    assert_eq!(checkpoint_small.size, 4);

    // The checkpoint signature verifies under the server key.
    let (server_public, _private) = store.server_keypair().unwrap();
    let root = hex::decode(&checkpoint_small.root_hash).unwrap();
    let mut root32 = [0u8; 32];
    root32.copy_from_slice(&root);
    crypto::verify(
        &server_public,
        &crate::log::checkpoint_signing_input(checkpoint_small.size as u64, &root32),
        &crypto::decode_bytes(&checkpoint_small.signature, "signature").unwrap(),
    )
    .expect("checkpoint signature verifies");

    // Every entry has a verifying inclusion proof against the head.
    for index in 0..checkpoint_small.size {
        let proof = log_inclusion_proof(
            State(state.clone()),
            peer("127.0.0.1"),
            axum::extract::Path(index),
            axum::extract::Query(ProofQuery { size: None }),
        )
        .await
        .unwrap()
        .0;
        let leaf = {
            let raw = hex::decode(&proof.leaf_hash).unwrap();
            let mut leaf = [0u8; 32];
            leaf.copy_from_slice(&raw);
            leaf
        };
        let path: Vec<[u8; 32]> = proof
            .path
            .iter()
            .map(|node| {
                let raw = hex::decode(node).unwrap();
                let mut out = [0u8; 32];
                out.copy_from_slice(&raw);
                out
            })
            .collect();
        crate::log::verify_inclusion(
            index as usize,
            checkpoint_small.size as usize,
            &leaf,
            &path,
            &root32,
        )
        .unwrap_or_else(|err| panic!("index {index}: {err}"));
    }

    // Append more entries; the consistency proof ties old head to new.
    let (signing_public, _signing_private) = crypto::generate_keypair();
    real_attestation(
        &state,
        &token,
        "2.0.0",
        &crypto::fingerprint(&signing_public),
    )
    .await;
    let checkpoint_big = log_checkpoint(State(state.clone()), peer("127.0.0.1"))
        .await
        .unwrap()
        .0;
    assert_eq!(checkpoint_big.size, 5);
    let proof = log_consistency_proof(
        State(state.clone()),
        peer("127.0.0.1"),
        axum::extract::Query(ConsistencyQuery {
            from: checkpoint_small.size,
            to: None,
        }),
    )
    .await
    .unwrap()
    .0;
    let new_root = {
        let raw = hex::decode(&checkpoint_big.root_hash).unwrap();
        let mut out = [0u8; 32];
        out.copy_from_slice(&raw);
        out
    };
    let path: Vec<[u8; 32]> = proof
        .path
        .iter()
        .map(|node| {
            let raw = hex::decode(node).unwrap();
            let mut out = [0u8; 32];
            out.copy_from_slice(&raw);
            out
        })
        .collect();
    crate::log::verify_consistency(
        checkpoint_small.size as usize,
        checkpoint_big.size as usize,
        &root32,
        &new_root,
        &path,
    )
    .expect("consistency proof verifies");
}

/// plan-61-B Phase 1's first task, and the reason `build_router` exists.
///
/// `GET /packages/:ident` makes a parameter segment a sibling of the static
/// `/packages/transfer/offer` and `/packages/transfer/accept` routes.
/// matchit is expected to prefer the static segment, but a conflict
/// **panics inside `Router::route` at construction**, which without this
/// test would surface as a server that dies at startup rather than as a red
/// test. Constructing the router at all is the assertion.
#[tokio::test]
async fn router_has_no_route_conflicts_and_static_beats_param() {
    use tower::ServiceExt;
    let h = harness();
    let router = build_router(h.state.clone());

    // Both transfer routes still resolve to their POST handlers, and are
    // not swallowed by `/packages/:ident`. A 405 (not 404) proves the path
    // matched the static route, which only accepts POST.
    for path in ["/packages/transfer/offer", "/packages/transfer/accept"] {
        let response = router
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("GET")
                    .uri(path)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::METHOD_NOT_ALLOWED,
            "{path} must still resolve to the static POST route, not to /packages/:ident",
        );
    }

    // And the param route is reachable for a real ident.
    let response = router
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("GET")
                .uri("/packages/alice%23toolbox")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "an unpublished ident reaches the handler and 404s",
    );
}

/// An unknown package 404s with the standard error shape, and an unknown
/// *owner* is indistinguishable from an unknown *package* — otherwise the
/// route is an owner-enumeration oracle.
#[tokio::test]
async fn the_read_routes_404_without_leaking_whether_an_owner_exists() {
    let h = harness();
    register_owner_with_all_keys(&h.store, "alice");

    let known_owner = err_of(
        package_detail(
            State(h.state.clone()),
            axum::extract::Path("alice#nosuchpackage".to_string()),
        )
        .await,
    );
    let unknown_owner = err_of(
        package_detail(
            State(h.state.clone()),
            axum::extract::Path("mallory#nosuchpackage".to_string()),
        )
        .await,
    );
    assert_eq!(known_owner.0, StatusCode::NOT_FOUND);
    assert_eq!(
        known_owner, unknown_owner,
        "a registered owner and an unregistered one must be indistinguishable",
    );

    // The audit route answers identically.
    let audit_known = err_of(
        package_audit(
            State(h.state.clone()),
            peer("127.0.0.1"),
            axum::extract::Path("alice#nosuchpackage".to_string()),
        )
        .await,
    );
    let audit_unknown = err_of(
        package_audit(
            State(h.state.clone()),
            peer("127.0.0.1"),
            axum::extract::Path("mallory#nosuchpackage".to_string()),
        )
        .await,
    );
    assert_eq!(audit_known.0, StatusCode::NOT_FOUND);
    assert_eq!(audit_known, audit_unknown);

    // A malformed ident is a 400, not a 404.
    for ident in ["noseparator", "#toolbox", "alice#"] {
        let (status, message) = err_of(
            package_detail(
                State(h.state.clone()),
                axum::extract::Path(ident.to_string()),
            )
            .await,
        );
        assert_eq!(status, StatusCode::BAD_REQUEST, "{ident}");
        assert!(message.contains("<owner>#<package>"), "{ident}: {message}");
    }
}

/// plan-61-B Phase 1 acceptance: the read routes are anonymous, and a
/// request carrying a valid credential behaves **identically** to one
/// carrying none.
///
/// Asserting equality both ways matters. "No credential required" would
/// still hold if the route quietly read a token and widened what it
/// returned; that is the shape this rules out. The handlers take no token
/// parameter at all, and this pins that as behaviour rather than as a
/// property of the current signature.
#[tokio::test]
async fn the_read_routes_ignore_credentials_entirely() {
    use tower::ServiceExt;
    let h = harness();
    let keys = register_owner_with_all_keys(&h.store, "alice");
    let token = open_session(&h.store, "alice", &keys.auth_private);
    let alice_id = h.store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
    h.store
        .publish_package_version(
            alice_id,
            "alice#toolbox",
            "1.0.0",
            "hash",
            "data/1.0.0.mfp",
            "{}",
            &[],
            &crate::store::PublishMetadata::default(),
        )
        .unwrap();

    let router = build_router(h.state.clone());
    let fetch = |authorized: bool| {
        let router = router.clone();
        let token = token.clone();
        async move {
            let mut request = axum::http::Request::builder()
                .method("GET")
                .uri("/packages/alice%23toolbox");
            if authorized {
                request = request
                    .header("Authorization", format!("Bearer {token}"))
                    .header("Cookie", "session=whatever");
            }
            let response = router
                .oneshot(request.body(axum::body::Body::empty()).unwrap())
                .await
                .unwrap();
            let status = response.status();
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            (status, body)
        }
    };

    let (anon_status, anon_body) = fetch(false).await;
    let (auth_status, auth_body) = fetch(true).await;
    assert_eq!(anon_status, StatusCode::OK);
    assert_eq!(
        anon_status, auth_status,
        "a credential must not change the status",
    );
    assert_eq!(
        anon_body, auth_body,
        "a credential must not change the body — the route is anonymous in \
             both directions, not merely credential-optional",
    );
}

/// bug-579: the anonymous transparency-log routes each loaded EVERY leaf,
/// rebuilt the whole Merkle root and produced a fresh signature, on every
/// request, with no limiter key consumed. Cost grew with the log and nothing
/// bounded the repetition.
///
/// This pins the route CONTRACT: a repeat at an unchanged size is identical,
/// an APPEND is observed rather than masked, and what is served still
/// verifies under the server key. A cache that got the append wrong would be
/// a far worse bug than the one being fixed, because clients pin what this
/// returns. The memo's mechanism is probed separately in
/// `store::tests::signed_checkpoint_is_memoised_by_log_size`.
#[tokio::test]
async fn log_checkpoint_follows_an_append_and_still_verifies() {
    let h = harness();
    register_owner_with_all_keys(&h.store, "alice");
    let alice_id = h.store.owner_with_ident_key("alice").unwrap().unwrap().0.id;

    let first = log_checkpoint(State(h.state.clone()), peer("10.0.0.1"))
        .await
        .unwrap()
        .0;

    // A repeat at an unchanged size is byte-identical.
    //
    // Note this alone does NOT prove the memo engaged: Ed25519 signing is
    // deterministic (RFC 8032), so a full recompute would produce exactly
    // the same bytes. It pins the contract, not the mechanism.
    let repeat = log_checkpoint(State(h.state.clone()), peer("10.0.0.1"))
        .await
        .unwrap()
        .0;
    assert_eq!(repeat.size, first.size);
    assert_eq!(repeat.root_hash, first.root_hash);
    assert_eq!(repeat.signature, first.signature);

    // An append MUST be observed. This is the half a stale cache breaks.
    // The memo's *mechanism* is probed in
    // `store::tests::signed_checkpoint_is_memoised_by_log_size`, which can
    // reach the connection directly; here the contract is what matters.
    h.store
        .publish_package_version(
            alice_id,
            "alice#toolbox",
            "1.0.0",
            "hash-1",
            "data/1.mfp",
            "{}",
            &[],
            &crate::store::PublishMetadata::default(),
        )
        .unwrap();
    let after = log_checkpoint(State(h.state.clone()), peer("10.0.0.1"))
        .await
        .unwrap()
        .0;
    assert!(
        after.size > first.size,
        "the memo must not hide an append: {} -> {}",
        first.size,
        after.size
    );
    assert_ne!(after.root_hash, first.root_hash);

    // And the served head is the real one, not a cached artefact: it must
    // verify under the server key and match a freshly computed root.
    let leaves = h.store.log_leaf_hashes(None).unwrap();
    assert_eq!(after.root_hash, hex::encode(crate::log::root(&leaves)));
    let (server_public, _) = h.store.server_keypair().unwrap();
    crypto::verify(
        &server_public,
        &crate::log::checkpoint_signing_input(after.size as u64, &crate::log::root(&leaves)),
        &crypto::decode_bytes(&after.signature, "signature").unwrap(),
    )
    .expect("the memoised checkpoint must still verify under the server key");
}

#[tokio::test]
async fn health_and_ident_endpoints_report_the_registry_key() {
    let h = harness();
    assert!(health().await.0.ok);

    let ident = server_ident(State(h.state)).await.expect("ident served").0;
    let expected = h.store.server_public_key().unwrap();
    assert_eq!(
        crypto::decode_bytes(&ident.server_key, "serverKey").unwrap(),
        expected,
    );
    assert_eq!(ident.server_fingerprint, crypto::fingerprint(&expected));
    // The published key is the one attestations are signed with, so a
    // client that pins it can verify /signing output.
    let (keypair_public, _private) = h.store.server_keypair().unwrap();
    assert_eq!(keypair_public, expected);
}

/// bug-584 (positive pin, end to end): a client that pinned the registry
/// root BEFORE a renewal still verifies the served chain after it. The
/// renewal re-delegates fresh online keys under a bumped root version, and
/// the client — anchored solely on the fingerprint it pinned — follows it
/// with no out-of-band step, picking up the newly delegated keys.
#[tokio::test]
async fn a_pinned_client_verifies_the_chain_across_an_authenticated_root_renewal() {
    let h = harness();
    let expires = now_unix() + 86_400;
    let root_private = h.store.init_registry_root("reg-1", expires).unwrap();
    // What the operator publishes and the client pins out of band.
    let pinned_fingerprint = root_metadata(State(h.state.clone()))
        .await
        .expect("root served")
        .0
        .root_fingerprint;

    let verify = |state: AppState| async move {
        let root = root_metadata(State(state.clone())).await.expect("root").0;
        let timestamp = timestamp_metadata(State(state.clone()))
            .await
            .expect("timestamp")
            .0;
        let snapshot = snapshot_metadata(State(state)).await.expect("snapshot").0;
        (root, timestamp, snapshot)
    };

    let (root, timestamp, snapshot) = verify(h.state.clone()).await;
    let before = crate::client::verify_registry_metadata(
        &root,
        &timestamp,
        &snapshot,
        "reg-1",
        &pinned_fingerprint,
        0,
        0,
        now_unix(),
    )
    .expect("the first chain verifies under the pinned root");
    assert_eq!(before.root_version, 1);

    // The operator renews with the offline root key they stored at init.
    let version = h
        .store
        .renew_registry_root("reg-1", expires + 86_400, &root_private)
        .expect("renewal under the offline root key");
    assert_eq!(version, 2);

    let (root, timestamp, snapshot) = verify(h.state.clone()).await;
    // The anchor the client pinned is unchanged...
    assert_eq!(root.root_fingerprint, pinned_fingerprint);
    let after = crate::client::verify_registry_metadata(
        &root,
        &timestamp,
        &snapshot,
        "reg-1",
        &pinned_fingerprint,
        before.root_version,
        before.snapshot_version,
        now_unix(),
    )
    .expect("the renewed chain verifies under the SAME pinned root");
    // The renewal advanced the pinned root version...
    assert_eq!(after.root_version, 2);
    // ...the delegated attestation key still cross-checks against the
    // server key the client pinned, and the root document really did move.
    assert_eq!(after.server_key, h.store.server_public_key().unwrap());
    let root_doc: serde_json::Value = serde_json::from_str(&root.signed).unwrap();
    assert_eq!(root_doc["version"], 2);
    assert_eq!(root_doc["expires"], expires + 86_400);
    let config = h.store.registry_config().unwrap().unwrap();
    assert_eq!(
        root_doc["snapshotKey"],
        crypto::encode_bytes(&config.snapshot_public)
    );

    // A re-anchor, by contrast, is exactly the break the client must NOT
    // follow: the pinned fingerprint no longer matches.
    h.store
        .reanchor_registry_root("reg-1", expires + 172_800)
        .unwrap();
    let (root, timestamp, snapshot) = verify(h.state.clone()).await;
    assert_ne!(root.root_fingerprint, pinned_fingerprint);
    let err = crate::client::verify_registry_metadata(
        &root,
        &timestamp,
        &snapshot,
        "reg-1",
        &pinned_fingerprint,
        after.root_version,
        after.snapshot_version,
        now_unix(),
    )
    .unwrap_err();
    assert!(
        err.contains("does not match the pinned root fingerprint"),
        "{err}"
    );
}

#[tokio::test]
async fn signed_metadata_is_absent_until_the_root_ceremony_then_verifies() {
    let h = harness();
    // Before the ceremony every signed-metadata endpoint 404s rather than
    // serving an unsigned or empty document.
    assert_eq!(
        err_of(root_metadata(State(h.state.clone())).await).0,
        StatusCode::NOT_FOUND,
    );
    assert_eq!(
        err_of(snapshot_metadata(State(h.state.clone())).await).0,
        StatusCode::NOT_FOUND,
    );
    let (status, message) = err_of(timestamp_metadata(State(h.state.clone())).await);
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(
        message.contains("root of trust is not initialized"),
        "{message}",
    );

    let expires = now_unix() + 86_400;
    h.store
        .init_registry_root("test-registry", expires)
        .unwrap();
    let config = h.store.registry_config().unwrap().unwrap();

    let root = root_metadata(State(h.state.clone()))
        .await
        .expect("root served")
        .0;
    assert_eq!(
        root.root_fingerprint,
        crypto::fingerprint(&config.root_public)
    );
    crypto::verify(
        &crypto::decode_bytes(&root.root_key, "rootKey").unwrap(),
        &crypto::root_signing_input(root.signed.as_bytes()),
        &crypto::decode_bytes(&root.signature, "signature").unwrap(),
    )
    .expect("root.json verifies under the published root key");
    let root_doc: serde_json::Value = serde_json::from_str(&root.signed).unwrap();
    assert_eq!(root_doc["type"], "root");
    assert_eq!(root_doc["registryId"], "test-registry");
    assert_eq!(root_doc["expires"], expires);

    let snapshot = snapshot_metadata(State(h.state.clone()))
        .await
        .expect("snapshot served")
        .0;
    crypto::verify(
        &config.snapshot_public,
        &crypto::snapshot_signing_input(snapshot.signed.as_bytes()),
        &crypto::decode_bytes(&snapshot.signature, "signature").unwrap(),
    )
    .expect("snapshot verifies under the root-delegated snapshot key");
    let snapshot_doc: serde_json::Value = serde_json::from_str(&snapshot.signed).unwrap();
    assert_eq!(snapshot_doc["type"], "snapshot");
    assert_eq!(snapshot_doc["registryId"], "test-registry");
    assert_eq!(
        snapshot_doc["indexHash"],
        h.store.index_canonical_hash().unwrap(),
    );
    assert_eq!(snapshot_doc["version"], h.store.log_size().unwrap());
    assert_eq!(
        snapshot_doc["checkpoint"]["size"],
        h.store.log_leaf_hashes(None).unwrap().len() as i64,
    );
    assert!(snapshot_doc["expires"].as_i64().unwrap() > now_unix());

    let timestamp = timestamp_metadata(State(h.state))
        .await
        .expect("timestamp served")
        .0;
    crypto::verify(
        &config.timestamp_public,
        &crypto::timestamp_signing_input(timestamp.signed.as_bytes()),
        &crypto::decode_bytes(&timestamp.signature, "signature").unwrap(),
    )
    .expect("timestamp verifies under the root-delegated timestamp key");
    let timestamp_doc: serde_json::Value = serde_json::from_str(&timestamp.signed).unwrap();
    assert_eq!(timestamp_doc["type"], "timestamp");
    // The timestamp points at the snapshot the registry is serving now.
    assert_eq!(timestamp_doc["snapshotVersion"], snapshot_doc["version"]);
    assert_eq!(timestamp_doc["indexHash"], snapshot_doc["indexHash"]);
    // ...and it is the short-lived half of the pair.
    assert!(timestamp_doc["expires"].as_i64().unwrap() < snapshot_doc["expires"].as_i64().unwrap());
}

#[tokio::test]
async fn log_publish_entry_and_proof_bounds_are_enforced() {
    let h = harness();
    let keys = register_owner_with_all_keys(&h.store, "alice");
    let token = open_session(&h.store, "alice", &keys.auth_private);

    assert_eq!(
        err_of(
            log_publish_entry(
                State(h.state.clone()),
                axum::extract::Query(PublishEntryQuery {
                    ident: "alice#toolbox".to_string(),
                    version: "1.0.0".to_string(),
                }),
            )
            .await
        )
        .1,
        "no publish log entry for that package",
    );

    publish_valid_package(&h.state, &keys, &token, "1.0.0").await;
    let entry = log_publish_entry(
        State(h.state.clone()),
        axum::extract::Query(PublishEntryQuery {
            ident: "alice#toolbox".to_string(),
            version: "1.0.0".to_string(),
        }),
    )
    .await
    .expect("publish entry served")
    .0;
    let stored = h
        .store
        .publish_log_entry("alice#toolbox", "1.0.0")
        .unwrap()
        .unwrap();
    assert_eq!(entry.index, stored.index);
    assert_eq!(entry.leaf_hash, hex::encode(stored.leaf_hash));
    let leaves = h.store.log_leaf_hashes(None).unwrap();
    assert_eq!(hex::encode(leaves[entry.index as usize]), entry.leaf_hash);

    // An index outside the tree is a 400, never a panic on the slice.
    let size = leaves.len() as i64;
    for index in [-1, size] {
        let (status, message) = err_of(
            log_inclusion_proof(
                State(h.state.clone()),
                peer("127.0.0.1"),
                axum::extract::Path(index),
                axum::extract::Query(ProofQuery { size: None }),
            )
            .await,
        );
        assert_eq!(status, StatusCode::BAD_REQUEST, "index {index}");
        assert_eq!(message, "log entry index is outside the tree");
    }

    // `?size=` serves a proof against a historic head, and the bound moves
    // with it: index 1 is outside a one-leaf tree.
    let historic = log_inclusion_proof(
        State(h.state.clone()),
        peer("127.0.0.1"),
        axum::extract::Path(0),
        axum::extract::Query(ProofQuery { size: Some(1) }),
    )
    .await
    .expect("historic proof served")
    .0;
    assert_eq!(historic.size, 1);
    assert!(historic.path.is_empty());
    assert_eq!(historic.leaf_hash, hex::encode(leaves[0]));
    assert_eq!(
        err_of(
            log_inclusion_proof(
                State(h.state.clone()),
                peer("127.0.0.1"),
                axum::extract::Path(1),
                axum::extract::Query(ProofQuery { size: Some(1) }),
            )
            .await
        )
        .1,
        "log entry index is outside the tree",
    );

    for from in [-1, size + 1] {
        let (status, message) = err_of(
            log_consistency_proof(
                State(h.state.clone()),
                peer("127.0.0.1"),
                axum::extract::Query(ConsistencyQuery { from, to: None }),
            )
            .await,
        );
        assert_eq!(status, StatusCode::BAD_REQUEST, "from {from}");
        assert_eq!(message, "consistency proof sizes are invalid");
    }
}

#[tokio::test]
async fn package_index_and_ident_chain_reject_malformed_lookups() {
    let h = harness();
    for ident in ["noseparator", "#toolbox", "alice#"] {
        let (status, message) = err_of(
            package_index(
                State(h.state.clone()),
                axum::extract::Path(ident.to_string()),
            )
            .await,
        );
        assert_eq!(status, StatusCode::BAD_REQUEST, "{ident}");
        assert_eq!(message, "ident must use <owner>#<package>");
    }
    assert_eq!(
        err_of(ident_chain(State(h.state), axum::extract::Path("nobody".to_string())).await).1,
        "unknown owner",
    );
}

#[test]
fn error_helpers_map_store_messages_to_status_codes() {
    assert_eq!(bad_request("nope".to_string()).0, StatusCode::BAD_REQUEST);
    assert_eq!(
        internal("boom".to_string()).0,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(unauthorized("no").0, StatusCode::UNAUTHORIZED);
    // Only the two collision messages become a 409; everything else is a
    // client error, so a store failure is never reported as a conflict.
    assert_eq!(
        conflict_or_bad_request("owner name 'alice' is already in use".to_string()).0,
        StatusCode::CONFLICT,
    );
    assert_eq!(
        conflict_or_bad_request("reused challenge".to_string()).0,
        StatusCode::CONFLICT,
    );
    assert_eq!(
        conflict_or_bad_request("unknown owner".to_string()).0,
        StatusCode::BAD_REQUEST,
    );
    assert_eq!(too_many_requests().0, StatusCode::TOO_MANY_REQUESTS);
}

#[test]
fn blob_hash_paths_must_be_lowercase_hex() {
    validate_blob_hash(&"a".repeat(64)).expect("64 lowercase hex characters are accepted");
    for bad in [
        "a".repeat(63),
        "a".repeat(65),
        "A".repeat(64),
        "g".repeat(64),
    ] {
        assert_eq!(
            err_of(validate_blob_hash(&bad)),
            (
                StatusCode::BAD_REQUEST,
                "blob hash must be 64 lowercase hex characters".to_string()
            ),
            "{bad}",
        );
    }
}

/// Drive the real `serve()` entry point over a loopback socket: the router
/// wiring, the `ConnectInfo` plumbing the per-IP throttles depend on, and
/// the JSON codec on the wire are only exercised through an actual request.
#[tokio::test]
async fn serve_binds_the_router_and_answers_over_http() {
    let h = harness();
    // `serve()` reports the port it bound only when it returns, after its
    // shutdown future fires; take a free one from the OS, release it, and
    // hand it over.
    let probe = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = probe.local_addr().unwrap();
    drop(probe);

    let store = h.store.clone();
    let blob_store = BlobStore::local(h.packages_dir.clone());
    tokio::spawn(async move {
        let _ = serve(store, blob_store, addr, std::future::pending()).await;
    });

    let base = format!("http://{addr}");
    let client = reqwest::Client::new();
    // Wait for the accept loop to come up.
    let mut health = None;
    for _ in 0..100 {
        match client.get(format!("{base}/health")).send().await {
            Ok(response) => {
                health = Some(response);
                break;
            }
            Err(_) => tokio::time::sleep(std::time::Duration::from_millis(20)).await,
        }
    }
    let health = health.expect("the served router accepts connections");
    assert_eq!(health.status(), reqwest::StatusCode::OK);
    assert_eq!(
        health.json::<serde_json::Value>().await.unwrap(),
        serde_json::json!({ "ok": true }),
    );

    // A GET route reading state.
    let ident: ServerIdentResponse = client
        .get(format!("{base}/ident"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        ident.server_fingerprint,
        crypto::fingerprint(&h.store.server_public_key().unwrap()),
    );

    // The full anonymous onboarding chain over the wire: register, then
    // challenge, then login. `/accounts/register` and `/auth/login` are the
    // two routes that need `ConnectInfo`, so this also proves the
    // connect-info make-service is wired up.
    let (payload, auth_private) = registration_payload_with_auth_private("alice");
    let registered = client
        .post(format!("{base}/accounts/register"))
        .json(&payload)
        .send()
        .await
        .unwrap();
    assert_eq!(registered.status(), reqwest::StatusCode::OK);
    let registered: RegisterResponse = registered.json().await.unwrap();
    assert_eq!(registered.owner, "alice");

    let challenge: ChallengeResponse = client
        .post(format!("{base}/auth/challenge"))
        .json(&ChallengeRequest {
            owner: "alice".to_string(),
            auth_fingerprint: registered.auth_fingerprint.clone(),
        })
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let nonce = crypto::decode_bytes(&challenge.nonce, "nonce").unwrap();
    let signature = crypto::sign(
        &auth_private,
        &crypto::challenge_message(&challenge.challenge_id, &nonce),
    )
    .unwrap();
    let session: LoginResponse = client
        .post(format!("{base}/auth/login"))
        .json(&LoginRequest {
            challenge_id: challenge.challenge_id,
            signature: crypto::encode_bytes(&signature),
        })
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        verify_session_token(&h.store, &session.session_token)
            .expect("the token minted over HTTP is a live session")
            .sub,
        "alice",
    );

    // An unrouted path is a 404 rather than a 500 or a hang.
    assert_eq!(
        client
            .get(format!("{base}/no/such/route"))
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::NOT_FOUND,
    );
    // ...and an unknown blob hash 404s through the real `/blob/:hash` route.
    assert_eq!(
        client
            .get(format!("{base}/blob/{}", "0".repeat(64)))
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::NOT_FOUND,
    );
}

/// Like `registration_payload`, but also hands back the auth private key so
/// the caller can go on to sign a login challenge.
fn registration_payload_with_auth_private(owner: &str) -> (RegisterRequest, Vec<u8>) {
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
        auth_private,
    )
}
