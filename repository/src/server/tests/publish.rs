//! plan-176-I: `server::tests` — the publish tests. The shared harnesses (owners, transfers,\n//! MFPC builders, signed requests) live in `server/tests/mod.rs`.

use super::*;

struct CraftArgs<'a> {
    ident_key_public: &'a [u8],
    proof_signer: &'a [u8],
    attestation: String,
    attestation_sig: Vec<u8>,
    version: &'a str,
}

/// Craft a container v1.0 package for `alice#toolbox` with a fresh one-off
/// signing key, a proof signed by `proof_signer`, and the given
/// attestation. Returns the artifact plus the wire request describing it.
fn craft_package(args: CraftArgs<'_>, session_token: &str) -> PackageArtifactRequest {
    let (signing_public, signing_private) = crypto::generate_keypair();
    let ident_fingerprint = crypto::fingerprint(args.ident_key_public);
    let signing_fingerprint = crypto::fingerprint(&signing_public);
    let proof = format!(
        "{{\"owner\":\"alice\",\"ident\":\"alice#toolbox\",\"version\":\"{}\",\"identFingerprint\":\"{}\",\"signingFingerprint\":\"{}\",\"issued\":1}}",
        args.version, ident_fingerprint, signing_fingerprint,
    );
    let proof_sig = crypto::sign(
        args.proof_signer,
        &crypto::proof_signing_input(proof.as_bytes()),
    )
    .unwrap();
    let artifact = package::test_support::serialize(
        &package::test_support::TestPackage {
            name: "toolbox".to_string(),
            ident: "alice#toolbox".to_string(),
            version: args.version.to_string(),
            author: "alice".to_string(),
            url: String::new(),
            payload: b"MFPCtestpayload".to_vec(),
            ident_key: format!("ed25519:{}", crypto::encode_bytes(args.ident_key_public)),
            signing_key: format!("ed25519:{}", crypto::encode_bytes(&signing_public)),
            proof,
            proof_sig,
            attestation: args.attestation,
            attestation_sig: args.attestation_sig,
        },
        &signing_private,
    );
    let parsed = package::parse_mfp_package(&artifact).expect("crafted package parses");
    PackageArtifactRequest {
        ident: parsed.ident.clone(),
        version: parsed.version.clone(),
        artifact: crypto::encode_bytes(&artifact),
        content_hash: parsed.content_hash_hex(),
        ident_fingerprint: parsed.ident_fingerprint().unwrap(),
        signing_fingerprint: parsed.signing_fingerprint().unwrap(),
        session_token: session_token.to_string(),
    }
}

#[tokio::test]
async fn publish_chain_enforces_two_credentials_and_pinning() {
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

    // The crafted flow needs the attestation to pin the one-off key, and
    // the one-off key is generated inside craft_package. Pre-generate the
    // signing fingerprint by crafting once with a placeholder attestation
    // just to learn the fingerprint... instead, mirror the real client:
    // generate the one-off key first. craft_package generates its own, so
    // for the VALID case we call /signing with the fingerprint the craft
    // will use — achieved by crafting with a fixed rng is impossible, so
    // the valid-case craft is done inline here.
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
            proof,
            proof_sig,
            attestation,
            attestation_sig,
        },
        &signing_private,
    );
    let parsed = package::parse_mfp_package(&artifact).unwrap();
    let valid_request = PackageArtifactRequest {
        ident: parsed.ident.clone(),
        version: parsed.version.clone(),
        artifact: crypto::encode_bytes(&artifact),
        content_hash: parsed.content_hash_hex(),
        ident_fingerprint: parsed.ident_fingerprint().unwrap(),
        signing_fingerprint: parsed.signing_fingerprint().unwrap(),
        session_token: token.clone(),
    };
    let report =
        validate_package_request(&state, &valid_request, "validate", VALIDATE_PER_OWNER_MAX)
            .await
            .unwrap();
    assert!(
        report.valid,
        "fully chained package must validate: {:?}",
        report.diagnostics
    );

    // Two-credential negative 1 — ident-only forgery: the attacker holds
    // the ident private key (valid proof) but no session, so no genuine
    // attestation exists; a self-minted one signed by the attacker's own
    // "server" key must be refused (§3.4 step 2).
    let (fake_server_public, fake_server_private) = crypto::generate_keypair();
    let fake_attestation = attestation_json(
        &crypto::fingerprint(&fake_server_public),
        "alice",
        "alice#toolbox",
        "1.0.0",
        &ident_fingerprint,
        "0000000000000000000000000000000000000000000000000000000000000000",
        1,
    );
    let fake_attestation_sig = crypto::sign(
        &fake_server_private,
        &crypto::attestation_signing_input(fake_attestation.as_bytes()),
    )
    .unwrap();
    let forged = craft_package(
        CraftArgs {
            ident_key_public: &keys.ident_public,
            proof_signer: &keys.ident_private,
            attestation: fake_attestation,
            attestation_sig: fake_attestation_sig,
            version: "1.0.0",
        },
        &token,
    );
    let report = validate_package_request(&state, &forged, "validate", VALIDATE_PER_OWNER_MAX)
        .await
        .unwrap();
    assert!(!report.valid);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains("attestation verification failed")),
        "{:?}",
        report.diagnostics
    );

    // Two-credential negative 2 — auth-only forgery: the attacker has a
    // live session (real attestation) but not the ident private key, so
    // the proof cannot verify under the registered identKey (§3.4 step 6).
    let (_attacker_public, attacker_private) = crypto::generate_keypair();
    // The attestation must pin the forged package's one-off key, which
    // craft_package generates internally — so the attestation check will
    // also fail. To isolate the PROOF failure, attest the crafted
    // package's own fingerprint by crafting first with a throwaway
    // attestation, reading the fingerprint, then re-crafting is not
    // possible (fresh key each craft). Instead assert on the proof
    // diagnostic which fires regardless of the attestation result.
    let throwaway = real_attestation(&state, &token, "1.0.0", &signing_fingerprint).await;
    let forged = craft_package(
        CraftArgs {
            ident_key_public: &keys.ident_public,
            proof_signer: &attacker_private,
            attestation: throwaway.0,
            attestation_sig: throwaway.1,
            version: "1.0.0",
        },
        &token,
    );
    let report = validate_package_request(&state, &forged, "validate", VALIDATE_PER_OWNER_MAX)
        .await
        .unwrap();
    assert!(!report.valid);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains("proof verification failed")),
        "{:?}",
        report.diagnostics
    );

    // Attestation reuse — a genuine attestation for 1.0.0 cannot publish
    // version 2.0.0 (§3.4 step 3 pinning).
    let (reuse_attestation, reuse_sig) =
        real_attestation(&state, &token, "1.0.0", &signing_fingerprint).await;
    let reused = craft_package(
        CraftArgs {
            ident_key_public: &keys.ident_public,
            proof_signer: &keys.ident_private,
            attestation: reuse_attestation,
            attestation_sig: reuse_sig,
            version: "2.0.0",
        },
        &token,
    );
    let report = validate_package_request(&state, &reused, "validate", VALIDATE_PER_OWNER_MAX)
        .await
        .unwrap();
    assert!(!report.valid);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains("attestation")),
        "{:?}",
        report.diagnostics
    );

    // Wrong container version — hard v1.0 (index §10.2).
    let mut wrong_version = artifact.clone();
    wrong_version[10] = 9; // containerMinor = 9
    let request = PackageArtifactRequest {
        ident: valid_request.ident.clone(),
        version: valid_request.version.clone(),
        artifact: crypto::encode_bytes(&wrong_version),
        content_hash: valid_request.content_hash.clone(),
        ident_fingerprint: valid_request.ident_fingerprint.clone(),
        signing_fingerprint: valid_request.signing_fingerprint.clone(),
        session_token: token.clone(),
    };
    let report = validate_package_request(&state, &request, "validate", VALIDATE_PER_OWNER_MAX)
        .await
        .unwrap();
    assert!(!report.valid);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains("unsupported MFP container version")),
        "{:?}",
        report.diagnostics
    );
}

/// plan-48-A §4.2/§4.3: the native-blob write half. A blob PUTs, HEADs, and
/// GETs back byte-identically under the `.bin` namespace; a hash-mismatched
/// body stores nothing; a re-PUT is a cheap success; an unauthenticated or
/// bogus-token PUT is refused.
#[tokio::test]
async fn native_blob_put_head_get_roundtrip_and_auth() {
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

    let body = b"\x7fELF vendored native library payload".to_vec();
    let hash = hex::encode(crypto::sha256(&body));
    let bearer = |token: &str| {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {token}").parse().unwrap(),
        );
        headers
    };

    // Nothing stored yet.
    assert_eq!(
        head_blob(State(state.clone()), axum::extract::Path(hash.clone())).await,
        StatusCode::NOT_FOUND
    );

    // No Authorization header at all → 401, and nothing is stored.
    let no_auth = put_blob(
        State(state.clone()),
        axum::extract::Path(hash.clone()),
        axum::http::HeaderMap::new(),
        axum::body::Bytes::from(body.clone()),
    )
    .await;
    assert_eq!(no_auth.err().unwrap().0, StatusCode::UNAUTHORIZED);

    // A syntactically valid but unknown token → 401.
    let bad_token = put_blob(
        State(state.clone()),
        axum::extract::Path(hash.clone()),
        bearer("not.a.session"),
        axum::body::Bytes::from(body.clone()),
    )
    .await;
    assert_eq!(bad_token.err().unwrap().0, StatusCode::UNAUTHORIZED);
    assert!(!state
        .blob_store
        .exists(&hash, BlobKind::Native)
        .await
        .unwrap());

    // A real session stores the blob.
    let created = put_blob(
        State(state.clone()),
        axum::extract::Path(hash.clone()),
        bearer(&token),
        axum::body::Bytes::from(body.clone()),
    )
    .await
    .expect("authenticated put stores the blob");
    assert_eq!(created, StatusCode::CREATED);

    // HEAD now reports it, and it lives under the `.bin` namespace — never
    // `<hash>.mfp`, which would lie about what the file is.
    assert_eq!(
        head_blob(State(state.clone()), axum::extract::Path(hash.clone())).await,
        StatusCode::OK
    );
    assert!(opened.packages_dir.join(format!("{hash}.bin")).exists());
    assert!(!opened.packages_dir.join(format!("{hash}.mfp")).exists());

    // GET serves the exact bytes back.
    let response = package_blob(State(state.clone()), axum::extract::Path(hash.clone()))
        .await
        .expect("native blob served");
    let served = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(served.as_ref(), body.as_slice());

    // Re-uploading the same blob is a cheap success, not an error, and does
    // not duplicate anything.
    let again = put_blob(
        State(state.clone()),
        axum::extract::Path(hash.clone()),
        bearer(&token),
        axum::body::Bytes::from(body.clone()),
    )
    .await
    .expect("re-put is idempotent");
    assert_eq!(again, StatusCode::OK);

    // A body that does not hash to the path is refused and stores NOTHING.
    let other_hash = hex::encode(crypto::sha256(b"entirely different bytes"));
    let mismatch = put_blob(
        State(state.clone()),
        axum::extract::Path(other_hash.clone()),
        bearer(&token),
        axum::body::Bytes::from(body.clone()),
    )
    .await;
    assert_eq!(mismatch.err().unwrap().0, StatusCode::BAD_REQUEST);
    assert!(!state
        .blob_store
        .exists(&other_hash, BlobKind::Native)
        .await
        .unwrap());
    assert_eq!(
        head_blob(State(state.clone()), axum::extract::Path(other_hash)).await,
        StatusCode::NOT_FOUND
    );

    // A non-hex path is a 400 on both verbs.
    assert_eq!(
        head_blob(
            State(state.clone()),
            axum::extract::Path("nothex".to_string())
        )
        .await,
        StatusCode::BAD_REQUEST
    );
    let malformed = put_blob(
        State(state.clone()),
        axum::extract::Path("nothex".to_string()),
        bearer(&token),
        axum::body::Bytes::from(body.clone()),
    )
    .await;
    assert_eq!(malformed.err().unwrap().0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn install_path_serves_blob_and_index() {
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

    let (artifact, hash) = publish_valid_package(&state, &keys, &token, "1.0.0").await;

    // The blob round-trips byte-for-byte.
    let response = package_blob(State(state.clone()), axum::extract::Path(hash.clone()))
        .await
        .expect("blob served");
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(body.as_ref(), artifact.as_slice());

    // An unknown hash is a 404; a malformed hash is a 400.
    let missing = package_blob(State(state.clone()), axum::extract::Path("0".repeat(64))).await;
    assert_eq!(missing.err().unwrap().0, StatusCode::NOT_FOUND);
    let malformed = package_blob(
        State(state.clone()),
        axum::extract::Path("nothex".to_string()),
    )
    .await;
    assert_eq!(malformed.err().unwrap().0, StatusCode::BAD_REQUEST);

    // A corrupted stored blob is refused (blob-store corruption defense).
    std::fs::write(opened.packages_dir.join(format!("{hash}.mfp")), b"corrupt").unwrap();
    let corrupted = package_blob(State(state.clone()), axum::extract::Path(hash.clone())).await;
    assert_eq!(
        corrupted.err().unwrap().0,
        StatusCode::INTERNAL_SERVER_ERROR
    );

    // The index lists the version with its publish time, state, and a
    // name binding that verifies under the server key.
    let index = package_index(
        State(state.clone()),
        axum::extract::Path("alice#toolbox".to_string()),
    )
    .await
    .expect("index served")
    .0;
    assert_eq!(index.versions.len(), 1);
    assert_eq!(index.versions[0].version, "1.0.0");
    assert_eq!(index.versions[0].hash, hash);
    assert_eq!(index.versions[0].state, "available");
    assert!(index.versions[0].published_at > 0);
    assert!(index.versions[0].log_entry.is_some());
    let (server_public, _private) = store.server_keypair().unwrap();
    let ident_public = crypto::decode_bytes(
        index.ident_key.strip_prefix("ed25519:").unwrap(),
        "identKey",
    )
    .unwrap();
    assert_eq!(ident_public, keys.ident_public);
    crypto::verify(
        &server_public,
        &crypto::name_binding_message(&index.owner, &index.ident_fingerprint),
        &crypto::decode_bytes(&index.name_binding_signature, "sig").unwrap(),
    )
    .expect("name binding verifies");

    // An unknown owner/package is a 400.
    let unknown = package_index(
        State(state.clone()),
        axum::extract::Path("bob#toolbox".to_string()),
    )
    .await;
    assert_eq!(unknown.err().unwrap().0, StatusCode::BAD_REQUEST);
}

/// plan-61-A §4: the header copy of `author`/`url` is a plaintext fast-scan
/// convenience; only the MANIFEST copy is covered by the signature. When
/// the two disagree the artifact is malformed or tampered, and the publish
/// is refused naming both values — never silently resolved in favour of
/// either, which is how a transparency registry ends up rendering a string
/// nobody signed.
#[tokio::test]
async fn a_header_manifest_metadata_mismatch_is_refused() {
    let h = harness();
    let keys = register_owner_with_all_keys(&h.store, "alice");
    let token = open_session(&h.store, "alice", &keys.auth_private);

    // The signed manifest says "mallory"; the plaintext header says "alice".
    let strings = mfpc_string_pool(&["", "mallory"]);
    let payload = mfpc_container(&[(1, mfpc_manifest_section(1, 0)), (2, strings)]);
    let (_artifact, request) =
        signed_request_with_header_metadata(&h.state, &keys, &token, "1.0.0", payload, "alice", "")
            .await;

    let (status, body) = publish_package(State(h.state.clone()), Json(request))
        .await
        .expect_err("a metadata mismatch must not publish");
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body.error.contains("header and signed manifest disagree")
            && body.error.contains("alice")
            && body.error.contains("mallory"),
        "the error must name both values: {}",
        body.error,
    );
    // Nothing was persisted: the refusal happens before the version row.
    assert!(h
        .store
        .list_package_versions("alice#toolbox")
        .unwrap()
        .is_empty());
}

/// plan-61-A Phase 2: `POST /validate` is a **dry run**. It reads section-10
/// locators — that is how it reports missing vendor blobs before the
/// publisher uploads anything — but it holds no transaction and no
/// `package_version_id`, and it must never write a target row. Persisting
/// there would invert its documented non-mutating contract, so the guard is
/// a test rather than a comment.
#[tokio::test]
async fn validate_writes_no_target_rows_but_publish_does() {
    let h = harness();
    let keys = register_owner_with_all_keys(&h.store, "alice");
    let token = open_session(&h.store, "alice", &keys.auth_private);

    let vendor_bytes = b"\x7fELF vendored archive".to_vec();
    let vendor_hash = crypto::sha256(&vendor_bytes);
    let vendor_hex = hex::encode(vendor_hash);
    let strings = mfpc_string_pool(&["snd", "libsnd.a", "linux", "aarch64"]);
    let payload = mfpc_container(&[
        (2, strings),
        (10, mfpc_vendor_table(1, &vendor_hash)),
        (15, mfpc_abi_section(&[(0, [0xab; 32])])),
    ]);

    // Upload the blob first, so validation passes and the only reason the
    // table could stay empty is that `/validate` does not write to it.
    put_blob(
        State(h.state.clone()),
        axum::extract::Path(vendor_hex),
        bearer_headers(&token),
        axum::body::Bytes::from(vendor_bytes),
    )
    .await
    .expect("vendor blob uploaded");

    let (_artifact, request) =
        signed_request(&h.state, &keys, &token, "1.0.0", payload.clone()).await;
    let report = validate_package(State(h.state.clone()), Json(request))
        .await
        .expect("validation ran")
        .0;
    assert!(report.valid, "{:?}", report.diagnostics);

    assert!(
        h.store.target_rows_for_test().is_empty(),
        "a dry run must not persist targets",
    );

    // The same artifact through `/publish` does write them — otherwise the
    // assertion above would pass for the wrong reason.
    let (_artifact, request) = signed_request(&h.state, &keys, &token, "1.0.0", payload).await;
    let _ = publish_package(State(h.state.clone()), Json(request))
        .await
        .expect("publish succeeds");

    let rows = h.store.target_rows_for_test();
    assert_eq!(rows.len(), 1);
    let (os, arch, libc, lib_type, source) = &rows[0];
    assert_eq!(os, "linux");
    assert_eq!(arch.as_deref(), Some("aarch64"));
    assert_eq!(*libc, None, "the fixture locator declares no libc");
    assert_eq!(lib_type, "vendor");
    assert_eq!(source, "libsnd.a");
}

// === plan-126-E Phase 2: section 17 captured at publish ================

/// A publishable payload with an optional section 17 (`doc`). Same shape
/// `validate_writes_no_target_rows_but_publish_does` publishes -- a string
/// pool and an ABI index -- minus the section-10 vendor table, which would
/// need an uploaded blob.
fn payload_with_doc_section(doc_section: Option<Vec<u8>>) -> Vec<u8> {
    let mut sections = vec![
        (2, mfpc_string_pool(&["greet"])),
        (15, mfpc_abi_section(&[(0, [0xab; 32])])),
    ];
    if let Some(section) = doc_section {
        sections.push((17, section));
    }
    mfpc_container(&sections)
}

/// A well-formed section-17 body, produced by the one shared encoder.
fn sample_doc_section() -> Vec<u8> {
    mfb_wire::docs::encode_doc_table(&mfb_wire::docs::PackageDocs {
        package: Some(mfb_wire::docs::PackageDocEntry {
            name: "toolbox".to_string(),
            desc: vec![(0, "A toolbox.".to_string())],
            deprecated: None,
        }),
        decls: vec![mfb_wire::docs::DeclDocEntry {
            kind: "func".to_string(),
            name: "greet".to_string(),
            signature: "EXPORT FUNC greet() AS String".to_string(),
            group: String::new(),
            desc: vec![(0, "Greets.".to_string())],
            args: Vec::new(),
            props: Vec::new(),
            ret: "a greeting".to_string(),
            errors: Vec::new(),
            example: String::new(),
            internal: false,
            deprecated: None,
        }],
    })
}

/// Publish `payload` as `alice#toolbox@1.0.0` through the real handler.
async fn publish_payload(payload: Vec<u8>) -> Harness {
    let h = harness();
    let keys = register_owner_with_all_keys(&h.store, "alice");
    let token = open_session(&h.store, "alice", &keys.auth_private);
    let (_artifact, request) = signed_request(&h.state, &keys, &token, "1.0.0", payload).await;
    let _ = publish_package(State(h.state.clone()), Json(request))
        .await
        .expect("publish succeeds");
    h
}

/// A documented package's section 17 is stored, and reads back byte for
/// byte -- the raw bytes, not a re-encoding.
#[tokio::test]
async fn publishing_a_documented_package_stores_its_doc_section_byte_for_byte() {
    let section = sample_doc_section();
    let h = publish_payload(payload_with_doc_section(Some(section.clone()))).await;
    assert_eq!(
        h.store.latest_active_version_docs("alice#toolbox").unwrap(),
        Some(("1.0.0".to_string(), section)),
    );
}

/// No section 17 is the normal undocumented case: publish succeeds, no row.
#[tokio::test]
async fn publishing_an_undocumented_package_succeeds_and_stores_no_doc_row() {
    let h = publish_payload(payload_with_doc_section(None)).await;
    assert_eq!(
        h.store.latest_active_version_docs("alice#toolbox").unwrap(),
        None
    );
}

/// **plan-126-E Phase 2's important negative.** A truncated section 17 must
/// not reject a signed package -- documentation "does not affect execution
/// or the ABI" -- and must not be recorded either.
#[tokio::test]
async fn a_truncated_doc_section_still_publishes_and_records_nothing() {
    let mut truncated = sample_doc_section();
    truncated.truncate(truncated.len() - 3);
    assert!(
        mfb_wire::docs::read_doc_table(&truncated).is_err(),
        "the fixture must genuinely fail to decode, or this test proves nothing"
    );
    let h = publish_payload(payload_with_doc_section(Some(truncated))).await;
    assert_eq!(
        h.store.latest_active_version_docs("alice#toolbox").unwrap(),
        None
    );
    // The version itself landed: a doc-table defect did not block the publish.
    assert_eq!(
        h.store
            .latest_active_version("alice#toolbox")
            .unwrap()
            .map(|(version, _)| version),
        Some("1.0.0".to_string()),
    );
}

#[tokio::test]
async fn publish_refuses_invalid_packages_and_tolerates_a_pre_staged_blob() {
    let h = harness();
    let keys = register_owner_with_all_keys(&h.store, "alice");
    let token = open_session(&h.store, "alice", &keys.auth_private);

    // A package that fails validation never reaches the store.
    let (_artifact, mut broken) = signed_request(
        &h.state,
        &keys,
        &token,
        "1.0.0",
        b"MFPCtestpayload".to_vec(),
    )
    .await;
    broken.content_hash = "0".repeat(64);
    let (status, message) = err_of(publish_package(State(h.state.clone()), Json(broken)).await);
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        message.starts_with("package validation failed: "),
        "{message}"
    );
    assert!(h
        .store
        .list_package_versions("alice#toolbox")
        .unwrap()
        .is_empty());

    // A blob left behind by an earlier, half-finished publish is reused
    // rather than re-staged: the version row still commits, and the
    // response reports that no new blob was written.
    let (artifact, request) = signed_request(
        &h.state,
        &keys,
        &token,
        "1.0.0",
        b"MFPCtestpayload".to_vec(),
    )
    .await;
    let hash = request.content_hash.clone();
    let staged = h
        .state
        .blob_store
        .stage(&hash, BlobKind::Package, artifact.clone())
        .await
        .unwrap();
    h.state.blob_store.promote(staged).await.unwrap();
    let published = publish_package(State(h.state.clone()), Json(request))
        .await
        .expect("publish succeeds over an existing blob")
        .0;
    assert!(!published.blob_stored);
    assert_eq!(published.hash, hash);
    assert_eq!(published.state, "available");
    assert!(published.warnings.is_empty());
    // ...and the bytes are still exactly the artifact.
    let response = package_blob(State(h.state.clone()), axum::extract::Path(hash.clone()))
        .await
        .expect("blob served");
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(body.as_ref(), artifact.as_slice());

    // Re-uploading that same content hash as a *native* blob is a no-op:
    // the row already serves it as a package, so promoting a `.bin` would
    // leave a second, unreferenced copy behind (bug-276 R5).
    let status = put_blob(
        State(h.state.clone()),
        axum::extract::Path(hash.clone()),
        bearer_headers(&token),
        axum::body::Bytes::from(artifact),
    )
    .await
    .expect("the no-op upload reports success");
    assert_eq!(status, StatusCode::OK);
    assert!(h.packages_dir.join(format!("{hash}.mfp")).exists());
    assert!(
        !h.packages_dir.join(format!("{hash}.bin")).exists(),
        "a package blob must not gain a duplicate .bin copy",
    );
    assert_eq!(
        h.store.blob_kind(&hash).unwrap().as_deref(),
        Some("package")
    );
}

#[tokio::test]
async fn publish_aborts_the_staged_blob_when_the_ident_moved_owner() {
    let h = harness();
    let alice = register_owner_with_all_keys(&h.store, "alice");
    let bob = register_owner_with_all_keys(&h.store, "bob");
    let alice_token = open_session(&h.store, "alice", &alice.auth_private);
    let bob_token = open_session(&h.store, "bob", &bob.auth_private);
    publish_valid_package(&h.state, &alice, &alice_token, "1.0.0").await;
    // Alice obtains a genuine attestation for 2.0.0 while she still owns
    // the package. After the transfer `/signing` refuses her (bug-494), so
    // this is the stale-attestation shape: attested before, published after.
    let (_artifact, request) = signed_request(
        &h.state,
        &alice,
        &alice_token,
        "2.0.0",
        b"MFPCtestpayload".to_vec(),
    )
    .await;

    // Hand the package to bob.
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

    // Alice still holds the ident named in the header and a genuine
    // attestation, but she no longer owns the package: `/validate` refuses
    // her (plan-168-B; before it, only the store's write did), so nothing
    // is staged and no orphan reaches the datapath. The store-refusal abort
    // path itself is covered by
    // `publish_aborts_the_staged_blob_when_the_store_refuses_the_write`.
    let hash = request.content_hash.clone();
    let (status, message) = err_of(publish_package(State(h.state.clone()), Json(request)).await);
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        message.contains("session owner is not the package's current owner"),
        "{message}"
    );
    assert!(
        !h.packages_dir.join(format!("{hash}.mfp")).exists(),
        "a refused publish must not leave a servable blob behind",
    );
    assert!(!h
        .state
        .blob_store
        .exists(&hash, BlobKind::Package)
        .await
        .unwrap());
    assert!(!h
        .store
        .package_version_exists("alice#toolbox", "2.0.0")
        .unwrap());
}

/// The blob-ordering contract (plan-10-A §2.6) when the STORE refuses a
/// publish that validated: the staged blob is aborted, leaving no servable
/// orphan and no version row. A lost history table stands in for any write
/// failure after validation (an ownership race, a full disk).
#[tokio::test]
async fn publish_aborts_the_staged_blob_when_the_store_refuses_the_write() {
    let h = harness();
    let alice = register_owner_with_all_keys(&h.store, "alice");
    let alice_token = open_session(&h.store, "alice", &alice.auth_private);
    let (_artifact, request) = signed_request(
        &h.state,
        &alice,
        &alice_token,
        "1.0.0",
        b"MFPCtestpayload".to_vec(),
    )
    .await;
    let report = validate_package(State(h.state.clone()), Json(clone_request(&request)))
        .await
        .unwrap()
        .0;
    assert!(report.valid, "{:?}", report.diagnostics);
    crate::store::tests::drop_tables(&h.store, &["transfer_history"]);

    let hash = request.content_hash.clone();
    let (_status, message) = err_of(publish_package(State(h.state.clone()), Json(request)).await);
    assert!(message.contains("transfer history"), "{message}");
    assert!(
        !h.packages_dir.join(format!("{hash}.mfp")).exists(),
        "a refused publish must not leave a servable blob behind",
    );
    assert!(!h
        .state
        .blob_store
        .exists(&hash, BlobKind::Package)
        .await
        .unwrap());
    assert!(!h
        .store
        .package_version_exists("alice#toolbox", "1.0.0")
        .unwrap());
}

/// bug-578, request level: an MFPC payload whose string pool declares more
/// entries than the format ceiling allows is rejected inside the parser,
/// before `/validate` builds a single `String` for it.
///
/// The payload here is ~4 MiB — a sixteenth of `MAX_BODY_BYTES` — and
/// declares 1,048,577 zero-length pool entries. Pre-fix the parser accepted
/// the count and constructed every entry, so those 4 MiB of body became a
/// ~25 MiB `Vec<String>`; scaled to the full 64 MiB body limit the same
/// shape reached ~288 MiB on a 512 MiB server, and `validate_package_request`
/// pays it once per parser it calls. The request must come back with a
/// bounded diagnostic and an empty `abiIndex` instead.
#[tokio::test]
async fn oversized_string_pool_is_rejected_at_the_request_boundary() {
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

    // A minimal MFPC container: an oversized string pool (section 2), a
    // one-entry native library table (section 10) so `parse_vendor_blobs`
    // has to resolve the pool, and an ABI index (section 15) so
    // `abi_index_json` does too.
    let hostile_payload = {
        let entries: u32 = 1_048_577; // MAX_STRING_POOL_ENTRIES + 1
        let mut pool = Vec::with_capacity(4 + entries as usize * 4);
        pool.extend_from_slice(&entries.to_le_bytes());
        for _ in 0..entries {
            pool.extend_from_slice(&0u32.to_le_bytes());
        }
        let mut table = Vec::new();
        table.extend_from_slice(&1u32.to_le_bytes()); // one entry
        table.extend_from_slice(&0u32.to_le_bytes()); // logical -> string 0
        table.extend_from_slice(&0u32.to_le_bytes()); // zero locators
        let mut abi = Vec::new();
        abi.extend_from_slice(&1u16.to_le_bytes()); // format version
        abi.extend_from_slice(&0u16.to_le_bytes()); // reserved
        abi.extend_from_slice(&0u32.to_le_bytes()); // zero exports

        let sections: [(u16, Vec<u8>); 3] = [(2, pool), (10, table), (15, abi)];
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"MFPC");
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&(sections.len() as u32).to_le_bytes());
        let mut data_offset = 16 + sections.len() * 24;
        for (id, data) in &sections {
            bytes.extend_from_slice(&id.to_le_bytes());
            bytes.extend_from_slice(&0u16.to_le_bytes());
            bytes.extend_from_slice(&0u32.to_le_bytes());
            bytes.extend_from_slice(&(data_offset as u64).to_le_bytes());
            bytes.extend_from_slice(&(data.len() as u64).to_le_bytes());
            data_offset += data.len();
        }
        for (_id, data) in &sections {
            bytes.extend_from_slice(data);
        }
        bytes
    };
    assert!(
        hostile_payload.len() < MAX_BODY_BYTES,
        "the whole point is that this fits inside the body limit"
    );

    let artifact = package::test_support::serialize(
        &package::test_support::TestPackage {
            name: "toolbox".to_string(),
            ident: "alice#toolbox".to_string(),
            version: "1.0.0".to_string(),
            author: "alice".to_string(),
            url: String::new(),
            payload: hostile_payload,
            ident_key: format!("ed25519:{}", crypto::encode_bytes(&keys.ident_public)),
            signing_key: format!("ed25519:{}", crypto::encode_bytes(&signing_public)),
            proof,
            proof_sig,
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
    assert!(!report.valid, "an over-ceiling pool must not validate");
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains("string pool declares 1048577 entries")),
        "expected a bounded string-pool diagnostic, got: {:?}",
        report.diagnostics
    );
    // The diagnostic is a single bounded line, not a dump of the payload.
    for diagnostic in &report.diagnostics {
        assert!(
            diagnostic.len() < 512,
            "diagnostics must stay bounded: {diagnostic}"
        );
    }
    // The best-effort ABI reader reports the empty object rather than
    // building the pool behind the caller's back.
    assert_eq!(report.abi_index, serde_json::json!({}));
}
