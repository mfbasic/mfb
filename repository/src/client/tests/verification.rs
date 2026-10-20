//! plan-176-I: `client::tests` — the verification tests. The shared metadata fixtures and the\n//! in-process HTTP stub live in `client/tests/mod.rs`.

use super::*;

/// An inclusion proof must be bound to the package being verified (bug-273).
///
/// The registry here is honest about the Merkle math and dishonest about
/// *which* entry it hands back: `/log/publish` always answers with entry E,
/// and serves a genuinely valid RFC-6962 proof for E. That is the correct
/// answer for E's own package and a forgery for anything else.
///
/// Before the fix the client checked only that the served leaf was in the tree,
/// so both queries below returned "verified". Asserting the honest query still
/// passes is what keeps the fix from being "reject everything".
#[test]
fn publish_inclusion_rejects_a_leaf_for_a_different_package() {
    let temp = tempfile::tempdir().unwrap();
    let paths = LocalPaths::new(temp.path().join(".mfb"));

    // Entry E — the one real publish the registry will keep pointing at.
    let (e_ident, e_version, e_hash) = ("alice#real", "2.0.0", "b".repeat(64));
    let e_leaf =
        crate::log::leaf_hash(publish_leaf_payload(e_ident, e_version, &e_hash).as_bytes());
    // A second leaf so the tree has a non-trivial inclusion path.
    let other_leaf = crate::log::leaf_hash(b"{\"kind\":\"register\"}");
    let leaves = [e_leaf, other_leaf];
    let root = crate::log::root(&leaves);
    let path: Vec<String> = crate::log::inclusion_path(0, &leaves)
        .iter()
        .map(hex::encode)
        .collect();

    let (server_public, server_private) = crypto::generate_keypair();
    let signature = crypto::sign(
        &server_private,
        &crate::log::checkpoint_signing_input(leaves.len() as u64, &root),
    )
    .unwrap();

    let ident_body = format!(
        "{{\"serverKey\":\"{}\",\"serverFingerprint\":\"{}\"}}",
        crypto::encode_bytes(&server_public),
        crypto::fingerprint(&server_public)
    );
    let checkpoint_body = format!(
        "{{\"size\":{},\"rootHash\":\"{}\",\"signature\":\"{}\"}}",
        leaves.len(),
        hex::encode(root),
        crypto::encode_bytes(&signature)
    );
    let publish_body = format!("{{\"index\":0,\"leafHash\":\"{}\"}}", hex::encode(e_leaf));
    let proof_body = format!(
        "{{\"index\":0,\"size\":{},\"leafHash\":\"{}\",\"path\":[{}]}}",
        leaves.len(),
        hex::encode(e_leaf),
        path.iter()
            .map(|node| format!("\"{node}\""))
            .collect::<Vec<_>>()
            .join(",")
    );

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        // Two verifications × four requests each (/ident, /log/checkpoint,
        // /log/publish, /log/proof). `Connection: close` keeps them on
        // separate accepts so the dispatch below stays simple.
        for _ in 0..8 {
            let Ok((mut sock, _)) = listener.accept() else {
                return;
            };
            let mut scratch = [0u8; 4096];
            let read = sock.read(&mut scratch).unwrap_or(0);
            let head = String::from_utf8_lossy(&scratch[..read]).to_string();
            let body = if head.contains("/ident") {
                &ident_body
            } else if head.contains("/log/checkpoint") {
                &checkpoint_body
            } else if head.contains("/log/publish") {
                &publish_body
            } else {
                &proof_body
            };
            let _ = write!(
                sock,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = sock.flush();
        }
    });
    let url = format!("http://127.0.0.1:{port}");

    // Honest query: the served leaf really is this package's.
    let (entry, checkpoint) = verify_publish_inclusion(&url, &paths, e_ident, e_version, &e_hash)
        .expect("a genuinely logged publish still verifies");
    assert_eq!(entry.index, 0);
    assert_eq!(checkpoint.size, leaves.len() as i64);

    // Substituted query: same valid proof, different package. Must be refused.
    let err = verify_publish_inclusion(&url, &paths, "alice#pkg", "1.0.0", &"a".repeat(64))
        .expect_err("a leaf for a different package must not verify");
    assert!(
        err.contains("does not match"),
        "error should name the leaf mismatch, got: {err}"
    );

    drop(server);
}

/// The reconstructed leaf payload must match `store.rs`'s canonical encoding
/// byte-for-byte, including `serde_json` escaping — any drift turns every
/// honest inclusion check into a failure.
#[test]
fn publish_leaf_payload_matches_the_store_encoding() {
    assert_eq!(
        publish_leaf_payload("alice#pkg", "1.0.0", "abc"),
        "{\"ident\":\"alice#pkg\",\"version\":\"1.0.0\",\"hash\":\"abc\"}"
    );
    // Quotes and backslashes must escape exactly as serde_json writes them.
    assert_eq!(
        publish_leaf_payload("a\"b", "1.0\\0", "h"),
        "{\"ident\":\"a\\\"b\",\"version\":\"1.0\\\\0\",\"hash\":\"h\"}"
    );
}

/// Build a fully valid metadata chain for the given parameters; individual
/// tests then mutate one field to exercise a rejection.
fn build_metadata(
    registry_id: &str,
    version: i64,
    expires: i64,
    index_hash: &str,
    timestamp_index_hash: &str,
) -> MetadataFixture {
    let (root_public, root_private) = crypto::generate_keypair();
    let (snapshot_public, snapshot_private) = crypto::generate_keypair();
    let (timestamp_public, timestamp_private) = crypto::generate_keypair();
    let (server_public, _server_private) = crypto::generate_keypair();
    let root_signed = format!(
        "{{\"type\":\"root\",\"registryId\":\"{registry_id}\",\"version\":1,\"expires\":{expires},\"serverKey\":\"{}\",\"snapshotKey\":\"{}\",\"timestampKey\":\"{}\"}}",
        crypto::encode_bytes(&server_public),
        crypto::encode_bytes(&snapshot_public),
        crypto::encode_bytes(&timestamp_public),
    );
    let root_signature = crypto::sign(
        &root_private,
        &crypto::root_signing_input(root_signed.as_bytes()),
    )
    .unwrap();
    let timestamp_signed = format!(
        "{{\"type\":\"timestamp\",\"registryId\":\"{registry_id}\",\"version\":{version},\"expires\":{expires},\"snapshotVersion\":{version},\"indexHash\":\"{timestamp_index_hash}\"}}",
    );
    let timestamp_signature = crypto::sign(
        &timestamp_private,
        &crypto::timestamp_signing_input(timestamp_signed.as_bytes()),
    )
    .unwrap();
    let snapshot_signed = format!(
        "{{\"type\":\"snapshot\",\"registryId\":\"{registry_id}\",\"version\":{version},\"expires\":{expires},\"indexHash\":\"{index_hash}\"}}",
    );
    let snapshot_signature = crypto::sign(
        &snapshot_private,
        &crypto::snapshot_signing_input(snapshot_signed.as_bytes()),
    )
    .unwrap();
    MetadataFixture {
        root_fingerprint: crypto::fingerprint(&root_public),
        server_public,
        root: RootResponse {
            signed: root_signed,
            signature: crypto::encode_bytes(&root_signature),
            root_key: crypto::encode_bytes(&root_public),
            root_fingerprint: crypto::fingerprint(&root_public),
        },
        timestamp: SignedMetadataResponse {
            signed: timestamp_signed,
            signature: crypto::encode_bytes(&timestamp_signature),
        },
        snapshot: SignedMetadataResponse {
            signed: snapshot_signed,
            signature: crypto::encode_bytes(&snapshot_signature),
        },
    }
}

#[test]
fn metadata_chain_verifies_and_returns_the_delegated_server_key() {
    let m = build_metadata("reg-1", 5, 2_000, "idxhash", "idxhash");
    let delegated = verify_registry_metadata(
        &m.root,
        &m.timestamp,
        &m.snapshot,
        "reg-1",
        &m.root_fingerprint,
        0,
        0,
        1_000,
    )
    .expect("valid chain verifies");
    assert_eq!(delegated.server_key, m.server_public);
    assert_eq!(delegated.snapshot_version, 5);
}

#[test]
fn metadata_chain_rejects_tampering_and_rollback() {
    let m = build_metadata("reg-1", 5, 2_000, "idxhash", "idxhash");

    // Wrong pinned root fingerprint.
    assert!(verify_registry_metadata(
        &m.root,
        &m.timestamp,
        &m.snapshot,
        "reg-1",
        "deadbeef",
        0,
        0,
        1_000,
    )
    .is_err());

    // Registry-id mismatch.
    assert!(verify_registry_metadata(
        &m.root,
        &m.timestamp,
        &m.snapshot,
        "reg-2",
        &m.root_fingerprint,
        0,
        0,
        1_000,
    )
    .is_err());

    // Expired (now past the expiry).
    assert!(verify_registry_metadata(
        &m.root,
        &m.timestamp,
        &m.snapshot,
        "reg-1",
        &m.root_fingerprint,
        0,
        0,
        9_000,
    )
    .unwrap_err()
    .contains("expired"));

    // Rollback below the pinned snapshot version.
    assert!(verify_registry_metadata(
        &m.root,
        &m.timestamp,
        &m.snapshot,
        "reg-1",
        &m.root_fingerprint,
        0,
        6,
        1_000,
    )
    .unwrap_err()
    .contains("ROLLBACK"));

    // bug-584: a root version below the pinned floor is a rollback, even
    // though the chain is otherwise perfect.
    assert!(verify_registry_metadata(
        &m.root,
        &m.timestamp,
        &m.snapshot,
        "reg-1",
        &m.root_fingerprint,
        2,
        0,
        1_000,
    )
    .unwrap_err()
    .contains("root version 1 is below the pinned version 2"));

    // Tampered snapshot signature.
    let mut tampered = build_metadata("reg-1", 5, 2_000, "idxhash", "idxhash");
    tampered.snapshot.signed = tampered.snapshot.signed.replace("idxhash", "evilhash");
    assert!(verify_registry_metadata(
        &tampered.root,
        &tampered.timestamp,
        &tampered.snapshot,
        "reg-1",
        &tampered.root_fingerprint,
        0,
        0,
        1_000,
    )
    .is_err());

    // Timestamp and snapshot disagree on the index hash.
    let m2 = build_metadata("reg-1", 5, 2_000, "idxhash", "otherhash");
    assert!(verify_registry_metadata(
        &m2.root,
        &m2.timestamp,
        &m2.snapshot,
        "reg-1",
        &m2.root_fingerprint,
        0,
        0,
        1_000,
    )
    .unwrap_err()
    .contains("index hash"));
}

#[test]
fn metadata_chain_rejects_undelegated_server_key_and_bad_root_signature() {
    // A bad root signature (root_key present but signature is garbage).
    let mut m = build_metadata("reg-1", 5, 2_000, "idxhash", "idxhash");
    m.root.signature = crypto::encode_bytes(&[0u8; 64]);
    assert!(verify_registry_metadata(
        &m.root,
        &m.timestamp,
        &m.snapshot,
        "reg-1",
        &m.root_fingerprint,
        0,
        0,
        1_000,
    )
    .unwrap_err()
    .contains("root.json signature"));

    // A tampered timestamp signature.
    let mut m2 = build_metadata("reg-1", 5, 2_000, "idxhash", "idxhash");
    m2.timestamp.signature = crypto::encode_bytes(&[0u8; 64]);
    assert!(verify_registry_metadata(
        &m2.root,
        &m2.timestamp,
        &m2.snapshot,
        "reg-1",
        &m2.root_fingerprint,
        0,
        0,
        1_000,
    )
    .unwrap_err()
    .contains("timestamp.json signature"));
}

/// Assemble a metadata chain from explicit doc versions so the
/// snapshot-vs-timestamp version disagreement branch can be exercised with
/// correctly-signed docs (the signature check would otherwise fire first).
fn build_metadata_versions(
    registry_id: &str,
    timestamp_version: i64,
    snapshot_version: i64,
) -> MetadataFixture {
    let (root_public, root_private) = crypto::generate_keypair();
    let (snapshot_public, snapshot_private) = crypto::generate_keypair();
    let (timestamp_public, timestamp_private) = crypto::generate_keypair();
    let (server_public, _server_private) = crypto::generate_keypair();
    let expires = 9_000i64;
    let root_signed = format!(
        "{{\"type\":\"root\",\"registryId\":\"{registry_id}\",\"version\":1,\"expires\":{expires},\"serverKey\":\"{}\",\"snapshotKey\":\"{}\",\"timestampKey\":\"{}\"}}",
        crypto::encode_bytes(&server_public),
        crypto::encode_bytes(&snapshot_public),
        crypto::encode_bytes(&timestamp_public),
    );
    let root_signature = crypto::sign(
        &root_private,
        &crypto::root_signing_input(root_signed.as_bytes()),
    )
    .unwrap();
    let timestamp_signed = format!(
        "{{\"type\":\"timestamp\",\"registryId\":\"{registry_id}\",\"version\":{timestamp_version},\"expires\":{expires},\"snapshotVersion\":{timestamp_version},\"indexHash\":\"h\"}}",
    );
    let timestamp_signature = crypto::sign(
        &timestamp_private,
        &crypto::timestamp_signing_input(timestamp_signed.as_bytes()),
    )
    .unwrap();
    let snapshot_signed = format!(
        "{{\"type\":\"snapshot\",\"registryId\":\"{registry_id}\",\"version\":{snapshot_version},\"expires\":{expires},\"indexHash\":\"h\"}}",
    );
    let snapshot_signature = crypto::sign(
        &snapshot_private,
        &crypto::snapshot_signing_input(snapshot_signed.as_bytes()),
    )
    .unwrap();
    MetadataFixture {
        root_fingerprint: crypto::fingerprint(&root_public),
        server_public,
        root: RootResponse {
            signed: root_signed,
            signature: crypto::encode_bytes(&root_signature),
            root_key: crypto::encode_bytes(&root_public),
            root_fingerprint: crypto::fingerprint(&root_public),
        },
        timestamp: SignedMetadataResponse {
            signed: timestamp_signed,
            signature: crypto::encode_bytes(&timestamp_signature),
        },
        snapshot: SignedMetadataResponse {
            signed: snapshot_signed,
            signature: crypto::encode_bytes(&snapshot_signature),
        },
    }
}

#[test]
fn metadata_chain_rejects_snapshot_timestamp_version_disagreement() {
    let m = build_metadata_versions("reg-1", 5, 6);
    let err = verify_registry_metadata(
        &m.root,
        &m.timestamp,
        &m.snapshot,
        "reg-1",
        &m.root_fingerprint,
        0,
        0,
        1_000,
    )
    .unwrap_err();
    assert!(err.contains("disagree on the snapshot version"), "{err}");
    // Sanity: the fixture's server key would be the delegated one on a
    // matching chain.
    let matched = build_metadata_versions("reg-1", 7, 7);
    let delegated = verify_registry_metadata(
        &matched.root,
        &matched.timestamp,
        &matched.snapshot,
        "reg-1",
        &matched.root_fingerprint,
        0,
        0,
        1_000,
    )
    .unwrap();
    assert_eq!(delegated.server_key, matched.server_public);
}

// --- transparency log -------------------------------------------------

/// The checkpoint is verified under the pinned server key and then held to
/// append-only growth: a shrunken tree or a different root at the same size
/// is refused, and neither ever overwrites the pin.
#[test]
fn fetch_checkpoint_pins_then_refuses_rollback_fork_and_bad_signatures() {
    let registry = Registry::new();
    let (_temp, paths) = temp_paths();
    let leaves = [
        crate::log::leaf_hash(b"a"),
        crate::log::leaf_hash(b"b"),
        crate::log::leaf_hash(b"c"),
        crate::log::leaf_hash(b"d"),
    ];
    let root4 = crate::log::root(&leaves);
    let root2 = crate::log::root(&leaves[..2]);

    let good = registry
        .routes()
        .ok("/log/checkpoint", registry.checkpoint_body(4, &root4))
        .serve();
    let checkpoint = fetch_checkpoint(&good.url, &paths).expect("first checkpoint pins");
    assert_eq!(checkpoint.size, 4);
    assert_eq!(
        local::read_checkpoint(&paths).unwrap(),
        Some((4, hex::encode(root4)))
    );

    // Smaller tree than the pin: a rollback.
    let shrunk = registry
        .routes()
        .ok("/log/checkpoint", registry.checkpoint_body(2, &root2))
        .serve();
    let err = fetch_checkpoint(&shrunk.url, &paths).unwrap_err();
    assert!(err.contains("ROLLBACK"), "{err}");

    // Same size, different root: a fork.
    let forked = registry
        .routes()
        .ok("/log/checkpoint", registry.checkpoint_body(4, &root2))
        .serve();
    let err = fetch_checkpoint(&forked.url, &paths).unwrap_err();
    assert!(err.contains("FORK"), "{err}");

    // A checkpoint nobody signed.
    let unsigned = registry
        .routes()
        .ok(
            "/log/checkpoint",
            serde_json::json!({
                "size": 8,
                "rootHash": hex::encode(root4),
                "signature": crypto::encode_bytes(&[0u8; 64]),
            })
            .to_string(),
        )
        .serve();
    let err = fetch_checkpoint(&unsigned.url, &paths).unwrap_err();
    assert!(
        err.contains("does not verify under the pinned server key"),
        "{err}"
    );

    // None of the three rejections moved the pin.
    assert_eq!(
        local::read_checkpoint(&paths).unwrap(),
        Some((4, hex::encode(root4)))
    );
}

/// A proven extension advances the pin; an unproven one must not — pinning
/// before verifying would erase the evidence a fork is caught by
/// (bug-276 R2).
#[test]
fn verify_log_consistency_advances_the_pin_only_on_a_proven_extension() {
    let registry = Registry::new();
    let (_temp, paths) = temp_paths();
    let leaves = [
        crate::log::leaf_hash(b"a"),
        crate::log::leaf_hash(b"b"),
        crate::log::leaf_hash(b"c"),
        crate::log::leaf_hash(b"d"),
    ];
    let root2 = crate::log::root(&leaves[..2]);
    let root4 = crate::log::root(&leaves);
    let path: Vec<String> = crate::log::consistency_path(2, &leaves)
        .iter()
        .map(hex::encode)
        .collect();

    // Nothing pinned yet: the first call just establishes the pin.
    let first = registry
        .routes()
        .ok("/log/checkpoint", registry.checkpoint_body(2, &root2))
        .serve();
    let checkpoint = verify_log_consistency(&first.url, &paths).expect("first pin");
    assert_eq!(checkpoint.size, 2);
    assert_eq!(
        local::read_checkpoint(&paths).unwrap(),
        Some((2, hex::encode(root2)))
    );

    // A genuine extension 2 -> 4 with a valid proof advances the pin.
    let extended = registry
        .routes()
        .ok("/log/checkpoint", registry.checkpoint_body(4, &root4))
        .ok(
            "/log/consistency",
            serde_json::json!({"from": 2, "to": 4, "path": path}).to_string(),
        )
        .serve();
    let checkpoint = verify_log_consistency(&extended.url, &paths).expect("extension verifies");
    assert_eq!(checkpoint.size, 4);
    assert_eq!(
        local::read_checkpoint(&paths).unwrap(),
        Some((4, hex::encode(root4)))
    );
    assert!(
        target_of(&extended.request_to("/log/consistency")).contains("from=2&to=4"),
        "the proof must be requested between the pinned and candidate sizes"
    );

    // Rewind the pin and serve the same head with an empty proof: the
    // candidate must be rejected *and* the pin left where it was.
    local::write_checkpoint(&paths, 2, &hex::encode(root2)).unwrap();
    let unproven = registry
        .routes()
        .ok("/log/checkpoint", registry.checkpoint_body(4, &root4))
        .ok(
            "/log/consistency",
            serde_json::json!({"from": 2, "to": 4, "path": []}).to_string(),
        )
        .serve();
    assert!(verify_log_consistency(&unproven.url, &paths).is_err());
    assert_eq!(
        local::read_checkpoint(&paths).unwrap(),
        Some((2, hex::encode(root2))),
        "an unproven head must not overwrite the pin it would be caught by"
    );
}

/// bug-582: a validly signed *larger* head is not evidence of an extension.
///
/// `fetch_checkpoint`'s only growth check was monotonicity, so any head with
/// `size > pinned_size` was written straight over the pin without asking for
/// a consistency proof. That is the same defect bug-276 R2 fixed for
/// `verify_log_consistency`, left standing in the sibling primitive — and it
/// is reachable, because `verify_publish_inclusion` (the whole of
/// `pkg install --proof`'s log contact) advances the pin through it.
///
/// The two halves matter separately: refusing an *absent* proof is easy, and
/// refusing a proof that is genuine **for the fork's own history** is what
/// proves the check is anchored to the pinned root rather than to the
/// registry's arithmetic.
#[test]
fn fetch_checkpoint_rejects_an_unproven_larger_fork() {
    let registry = Registry::new();
    let (_temp, paths) = temp_paths();
    let leaf = crate::log::leaf_hash;
    let honest = [
        leaf(b"a"),
        leaf(b"b"),
        leaf(b"c"),
        leaf(b"d"),
        leaf(b"e"),
        leaf(b"f"),
    ];
    let root4 = crate::log::root(&honest[..4]);
    let root6 = crate::log::root(&honest);
    // A fork: it grows past the pin but rewrites leaf 3, so it is not an
    // extension of the history this client already accepted.
    let fork = [leaf(b"a"), leaf(b"b"), leaf(b"c"), leaf(b"X"), leaf(b"e")];
    let root5 = crate::log::root(&fork);
    let hexpath = |path: Vec<[u8; 32]>| -> Vec<String> { path.iter().map(hex::encode).collect() };

    // Trust on first use: no predecessor, so size 4 pins.
    let first = registry
        .routes()
        .ok("/log/checkpoint", registry.checkpoint_body(4, &root4))
        .serve();
    assert_eq!(fetch_checkpoint(&first.url, &paths).unwrap().size, 4);
    assert_eq!(
        local::read_checkpoint(&paths).unwrap(),
        Some((4, hex::encode(root4)))
    );

    // A bigger fork with no proof offered at all.
    let silent = registry
        .routes()
        .ok("/log/checkpoint", registry.checkpoint_body(5, &root5))
        .serve();
    assert!(
        fetch_checkpoint(&silent.url, &paths).is_err(),
        "a larger head with no consistency proof must not be accepted"
    );
    assert_eq!(
        local::read_checkpoint(&paths).unwrap(),
        Some((4, hex::encode(root4))),
        "a refused head must never overwrite the pin it would be caught by"
    );

    // A bigger fork with a proof that is perfectly valid inside the fork's
    // own history — 4 -> 5 over the forked leaves. It cannot reproduce the
    // pinned root, and that is the only thing the client may trust.
    let forged = registry
        .routes()
        .ok("/log/checkpoint", registry.checkpoint_body(5, &root5))
        .ok(
            "/log/consistency",
            serde_json::json!({
                "from": 4, "to": 5,
                "path": hexpath(crate::log::consistency_path(4, &fork)),
            })
            .to_string(),
        )
        .serve();
    let err = fetch_checkpoint(&forged.url, &paths).unwrap_err();
    // Which of the two roots the walk fails to reproduce depends on where
    // the forked leaf sits, so the assertion is on the anchor rather than
    // on the side: a proof that is internally valid is still refused
    // because it cannot tie the candidate to the pinned history.
    assert!(
        err.contains("consistency proof does not reproduce"),
        "the proof must be anchored to the PINNED root, got: {err}"
    );
    assert_eq!(
        local::read_checkpoint(&paths).unwrap(),
        Some((4, hex::encode(root4)))
    );

    // POSITIVE: a genuine 4 -> 6 extension still advances the pin. Without
    // this the fix could be "refuse every advance" and still look green.
    let extended = registry
        .routes()
        .ok("/log/checkpoint", registry.checkpoint_body(6, &root6))
        .ok(
            "/log/consistency",
            serde_json::json!({
                "from": 4, "to": 6,
                "path": hexpath(crate::log::consistency_path(4, &honest)),
            })
            .to_string(),
        )
        .serve();
    assert_eq!(fetch_checkpoint(&extended.url, &paths).unwrap().size, 6);
    assert_eq!(
        local::read_checkpoint(&paths).unwrap(),
        Some((6, hex::encode(root6)))
    );
    assert!(
        target_of(&extended.request_to("/log/consistency")).contains("from=4&to=6"),
        "the proof is requested between the pinned and candidate sizes"
    );

    // POSITIVE: re-reading the head the client is already pinned to is not
    // an advance. `fetch_checkpoint_unpinned` has already refused every
    // other root at this size, so there is nothing left to prove and no
    // consistency request to make.
    let steady = registry
        .routes()
        .ok("/log/checkpoint", registry.checkpoint_body(6, &root6))
        .serve();
    assert_eq!(fetch_checkpoint(&steady.url, &paths).unwrap().size, 6);
    assert_eq!(
        local::read_checkpoint(&paths).unwrap(),
        Some((6, hex::encode(root6)))
    );
    assert!(
        !steady
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|request| path_of(request) == "/log/consistency"),
        "an unchanged head must not need a proof of itself"
    );
}

/// bug-582: `pkg install --proof` reaches the log only through
/// `verify_publish_inclusion`, and nothing on that path ever called
/// `verify_log_consistency` first — so the larger-fork attack landed on the
/// pin there with no consistency check anywhere in the flow.
#[test]
fn verify_publish_inclusion_rejects_an_unproven_larger_fork() {
    let registry = Registry::new();
    let (_temp, paths) = temp_paths();
    let leaf = crate::log::leaf_hash;
    let publish = crate::log::leaf_hash(
        publish_leaf_payload("alice#pkg", "1.0.0", &"a".repeat(64)).as_bytes(),
    );
    let honest = [publish, leaf(b"b"), leaf(b"c"), leaf(b"d")];
    let root2 = crate::log::root(&honest[..2]);
    let root4 = crate::log::root(&honest);
    // A fork of the same size-4 history that rewrites leaf 1.
    let fork = [publish, leaf(b"X"), leaf(b"c"), leaf(b"d")];
    let root4f = crate::log::root(&fork);

    let log_routes = |routes: Routes, leaves: &[[u8; 32]]| -> Routes {
        let path: Vec<String> = crate::log::inclusion_path(0, leaves)
            .iter()
            .map(hex::encode)
            .collect();
        routes
            .ok(
                "/log/publish",
                serde_json::json!({"index": 0, "leafHash": hex::encode(publish)}).to_string(),
            )
            .ok(
                "/log/proof/",
                serde_json::json!({
                    "index": 0,
                    "size": leaves.len(),
                    "leafHash": hex::encode(publish),
                    "path": path,
                })
                .to_string(),
            )
    };

    // Pin at size 2 by trust on first use.
    let first = registry
        .routes()
        .ok("/log/checkpoint", registry.checkpoint_body(2, &root2))
        .serve();
    assert_eq!(fetch_checkpoint(&first.url, &paths).unwrap().size, 2);

    // A larger fork, offering no consistency proof.
    let attacker = log_routes(
        registry
            .routes()
            .ok("/log/checkpoint", registry.checkpoint_body(4, &root4f)),
        &fork,
    )
    .serve();
    assert!(
        verify_publish_inclusion(&attacker.url, &paths, "alice#pkg", "1.0.0", &"a".repeat(64))
            .is_err(),
        "inclusion must not be reported against an unproven fork"
    );
    assert_eq!(
        local::read_checkpoint(&paths).unwrap(),
        Some((2, hex::encode(root2))),
        "the fork must not have replaced the pin"
    );

    // POSITIVE: the honest 2 -> 4 extension, with its proof, still verifies
    // the publish and advances the pin.
    let good = log_routes(
        registry
            .routes()
            .ok("/log/checkpoint", registry.checkpoint_body(4, &root4))
            .ok(
                "/log/consistency",
                serde_json::json!({
                    "from": 2, "to": 4,
                    "path": crate::log::consistency_path(2, &honest)
                        .iter()
                        .map(hex::encode)
                        .collect::<Vec<String>>(),
                })
                .to_string(),
            ),
        &honest,
    )
    .serve();
    let (entry, checkpoint) =
        verify_publish_inclusion(&good.url, &paths, "alice#pkg", "1.0.0", &"a".repeat(64))
            .expect("an honest publish under a proven extension still verifies");
    assert_eq!(entry.index, 0);
    assert_eq!(checkpoint.size, 4);
    assert_eq!(
        local::read_checkpoint(&paths).unwrap(),
        Some((4, hex::encode(root4)))
    );
}

/// The inclusion proof must describe the entry that was asked for.
#[test]
fn verify_publish_inclusion_rejects_a_mismatched_proof_envelope() {
    let registry = Registry::new();
    let leaf = crate::log::leaf_hash(publish_leaf_payload("alice#pkg", "1.0.0", "hh").as_bytes());
    let other = crate::log::leaf_hash(b"other");
    let leaves = [leaf, other];
    let root = crate::log::root(&leaves);

    let serve = |proof: serde_json::Value| {
        registry
            .routes()
            .ok("/log/checkpoint", registry.checkpoint_body(2, &root))
            .ok(
                "/log/publish",
                serde_json::json!({"index": 0, "leafHash": hex::encode(leaf)}).to_string(),
            )
            .ok("/log/proof/", proof.to_string())
            .serve()
    };

    // A proof for a different index than the entry served.
    let (_temp, paths) = temp_paths();
    let stub = serve(serde_json::json!({
        "index": 1,
        "size": 2,
        "leafHash": hex::encode(leaf),
        "path": [hex::encode(other)],
    }));
    let err = verify_publish_inclusion(&stub.url, &paths, "alice#pkg", "1.0.0", "hh").unwrap_err();
    assert_eq!(err, "inclusion proof does not match the requested entry");

    // A proof whose leaf is not the entry's leaf.
    let (_temp2, paths2) = temp_paths();
    let stub2 = serve(serde_json::json!({
        "index": 0,
        "size": 2,
        "leafHash": hex::encode(other),
        "path": [hex::encode(other)],
    }));
    let err2 =
        verify_publish_inclusion(&stub2.url, &paths2, "alice#pkg", "1.0.0", "hh").unwrap_err();
    assert_eq!(
        err2,
        "inclusion proof leaf does not match the publish entry"
    );
}

/// `mfb repo trust` pins the registry id and root fingerprint only when the
/// chain actually delegates the server key this machine pinned.
#[test]
fn trust_registry_pins_only_a_root_that_delegates_the_pinned_server_key() {
    let registry = Registry::new();
    let authority = MetadataAuthority::new();
    let future = crate::store::now_unix() + 3600;

    let (_temp, paths) = temp_paths();
    let good = build_metadata_delegating(&authority, "reg-1", 5, future, &registry.public);
    let stub = with_metadata_routes(registry.routes(), &good).serve();
    let version = trust_registry(&stub.url, &paths, "reg-1", &good.root_fingerprint)
        .expect("a delegating chain is trusted");
    assert_eq!(version, 5);
    assert_eq!(
        local::read_root_pin(&paths).unwrap(),
        Some(("reg-1".to_string(), good.root_fingerprint.clone()))
    );
    assert_eq!(local::read_snapshot_version(&paths).unwrap(), Some(5));

    // Same registry, but the root delegates some *other* attestation key:
    // trusting it would accept attestations the pinned key never made.
    let (_temp2, paths2) = temp_paths();
    let (stranger, _) = crypto::generate_keypair();
    let bad = build_metadata_delegating(&authority, "reg-1", 5, future, &stranger);
    let stub2 = with_metadata_routes(registry.routes(), &bad).serve();
    let err = trust_registry(&stub2.url, &paths2, "reg-1", &bad.root_fingerprint).unwrap_err();
    assert!(err.contains("not delegated by the pinned root"), "{err}");
    assert_eq!(local::read_root_pin(&paths2).unwrap(), None);
}

/// Once a root is pinned, every online flow re-verifies the chain and
/// advances the snapshot version; a rollback is refused and does not move
/// the pinned version.
#[test]
fn verify_pinned_metadata_advances_the_version_and_refuses_a_rollback() {
    let registry = Registry::new();
    let authority = MetadataAuthority::new();
    let future = crate::store::now_unix() + 3600;
    let (_temp, paths) = temp_paths();

    let fresh = build_metadata_delegating(&authority, "reg-1", 9, future, &registry.public);
    let stub = with_metadata_routes(registry.routes(), &fresh).serve();
    trust_registry(&stub.url, &paths, "reg-1", &fresh.root_fingerprint).unwrap();
    assert_eq!(local::read_snapshot_version(&paths).unwrap(), Some(9));

    // A newer snapshot from the same root advances the pinned version.
    let newer = build_metadata_delegating(&authority, "reg-1", 12, future, &registry.public);
    let stub2 = with_metadata_routes(registry.routes(), &newer).serve();
    verify_pinned_metadata(&stub2.url, &paths).expect("a newer snapshot is accepted");
    assert_eq!(local::read_snapshot_version(&paths).unwrap(), Some(12));

    // An older one — correctly signed by the same, pinned root — is a
    // rollback and must not move the pin back.
    let stale = build_metadata_delegating(&authority, "reg-1", 4, future, &registry.public);
    let stub3 = with_metadata_routes(registry.routes(), &stale).serve();
    let err = verify_pinned_metadata(&stub3.url, &paths).unwrap_err();
    assert!(err.contains("metadata ROLLBACK"), "{err}");
    assert_eq!(local::read_snapshot_version(&paths).unwrap(), Some(12));

    // A chain that no longer delegates the pinned server key is refused
    // even though the root is right.
    let (stranger, _) = crypto::generate_keypair();
    let swapped = build_metadata_delegating(&authority, "reg-1", 13, future, &stranger);
    let stub4 = with_metadata_routes(registry.routes(), &swapped).serve();
    let err = verify_pinned_metadata(&stub4.url, &paths).unwrap_err();
    assert!(err.contains("not delegated by the pinned root"), "{err}");
    assert_eq!(local::read_snapshot_version(&paths).unwrap(), Some(12));
    assert_eq!(authority.fingerprint(), fresh.root_fingerprint);
}

/// The chain must be re-fetched with the *pinned* root fingerprint, so a
/// registry that re-anchors to a new root is refused rather than followed.
#[test]
fn verify_pinned_metadata_refuses_a_re_anchored_root() {
    let registry = Registry::new();
    let future = crate::store::now_unix() + 3600;
    let (_temp, paths) = temp_paths();

    let original = build_metadata_delegating(
        &MetadataAuthority::new(),
        "reg-1",
        3,
        future,
        &registry.public,
    );
    let stub = with_metadata_routes(registry.routes(), &original).serve();
    trust_registry(&stub.url, &paths, "reg-1", &original.root_fingerprint).unwrap();

    // A brand-new root key, otherwise a perfectly valid chain.
    let reanchored = build_metadata_delegating(
        &MetadataAuthority::new(),
        "reg-1",
        4,
        future,
        &registry.public,
    );
    let stub2 = with_metadata_routes(registry.routes(), &reanchored).serve();
    let err = verify_pinned_metadata(&stub2.url, &paths).unwrap_err();
    assert!(
        err.contains("does not match the pinned root fingerprint"),
        "{err}"
    );
    assert_eq!(local::read_snapshot_version(&paths).unwrap(), Some(3));
}

/// bug-584: a root RENEWAL (same anchor, higher `root.json` version)
/// advances the pinned root version, and the retired root — still correctly
/// signed by the pinned key, still unexpired — cannot be replayed back over
/// it. Without this floor, a renewal performed to retire a compromised
/// online snapshot/timestamp key would be undone by serving the old
/// root.json that still delegates it.
#[test]
fn verify_pinned_metadata_follows_a_renewal_and_refuses_the_retired_root() {
    let registry = Registry::new();
    let authority = MetadataAuthority::new();
    let future = crate::store::now_unix() + 3600;
    let (_temp, paths) = temp_paths();

    let first = build_metadata_delegating_at_root_version(
        &authority,
        "reg-1",
        1,
        5,
        future,
        &registry.public,
    );
    let stub = with_metadata_routes(registry.routes(), &first).serve();
    trust_registry(&stub.url, &paths, "reg-1", &first.root_fingerprint).unwrap();
    assert_eq!(local::read_root_version(&paths).unwrap(), Some(1));

    // The operator renews: same anchor, root version 2. The pinned client
    // follows it with no out-of-band step.
    let renewed = build_metadata_delegating_at_root_version(
        &authority,
        "reg-1",
        2,
        6,
        future,
        &registry.public,
    );
    let stub2 = with_metadata_routes(registry.routes(), &renewed).serve();
    verify_pinned_metadata(&stub2.url, &paths).expect("a renewed root is followed");
    assert_eq!(local::read_root_version(&paths).unwrap(), Some(2));
    // Re-verifying the SAME root version is not a rollback.
    verify_pinned_metadata(&stub2.url, &paths).expect("the same root version still verifies");

    // The retired root, replayed with a newer snapshot, is refused.
    let replayed = build_metadata_delegating_at_root_version(
        &authority,
        "reg-1",
        1,
        7,
        future,
        &registry.public,
    );
    let stub3 = with_metadata_routes(registry.routes(), &replayed).serve();
    let err = verify_pinned_metadata(&stub3.url, &paths).unwrap_err();
    assert!(
        err.contains("root version 1 is below the pinned version 2"),
        "{err}"
    );
    assert_eq!(local::read_root_version(&paths).unwrap(), Some(2));
    assert_eq!(local::read_snapshot_version(&paths).unwrap(), Some(6));
}
