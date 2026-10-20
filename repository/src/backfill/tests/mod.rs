use super::*;
use crate::store::tests::register_keys;

// === plan-126-E Phase 3: doc sections ===================================

/// `payload_for`, plus a section 17 carrying `doc_section`.
fn payload_with_docs(author: &str, url: &str, doc_section: Vec<u8>) -> Vec<u8> {
    let strings = string_pool(&["", author, url, "snd", "linux", "x86_64", "libsnd.a"]);
    let author_id = if author.is_empty() { 0 } else { 1 };
    let url_id = if url.is_empty() { 0 } else { 2 };
    container(&[
        (1, manifest_section(author_id, url_id)),
        (2, strings),
        (10, vendor_table(&[0x11; 32])),
        (17, doc_section),
    ])
}

/// A well-formed section-17 body from the one shared encoder.
fn sample_doc_section() -> Vec<u8> {
    mfb_wire::docs::encode_doc_table(&mfb_wire::docs::PackageDocs {
        package: None,
        decls: vec![mfb_wire::docs::DeclDocEntry {
            kind: "func".to_string(),
            name: "greet".to_string(),
            signature: "EXPORT FUNC greet() AS String".to_string(),
            group: String::new(),
            desc: vec![(0, "Greets.".to_string())],
            args: Vec::new(),
            props: Vec::new(),
            ret: String::new(),
            errors: Vec::new(),
            example: String::new(),
            internal: false,
            deprecated: None,
        }],
    })
}

/// Store `artifact` as a blob and publish it as `alice#toolbox@version` the
/// way a server predating plan-126-E would have: with no docs row.
fn publish_stored(
    store: &Store,
    dir: &std::path::Path,
    owner_id: i64,
    version: &str,
    artifact: &[u8],
) {
    let hash = hex::encode(crate::crypto::sha256(artifact));
    std::fs::write(dir.join(format!("{hash}.mfp")), artifact).unwrap();
    store
        .publish_package_version(
            owner_id,
            "alice#toolbox",
            version,
            &hash,
            &format!("data/{hash}.mfp"),
            "{}",
            &[],
            &PublishMetadata::default(),
        )
        .unwrap();
}

/// The sweep records a doc section from a stored blob, leaves an
/// undocumented blob quietly alone, and on a second run records nothing --
/// the property that lets an operator re-run it safely.
#[tokio::test]
async fn backfill_fills_doc_sections_and_is_idempotent() {
    let (_temp, store, blob_store, dir) = harness().await;
    register_keys(&store, "alice");
    let alice_id = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
    let section = sample_doc_section();
    // The documented release is the newest, so the latest-active accessor
    // reads it back.
    publish_stored(&store, &dir, alice_id, "1.0.0", &package_bytes("alice", ""));
    publish_stored(
        &store,
        &dir,
        alice_id,
        "2.0.0",
        &serialize("alice", "", payload_with_docs("alice", "", section.clone())),
    );
    assert_eq!(
        store.latest_active_version_docs("alice#toolbox").unwrap(),
        None,
        "published without docs rows, as a server predating plan-126-E would",
    );

    let report = run(&store, &blob_store).await.unwrap();
    assert_eq!(report.updated, 2);
    assert_eq!(
        report.docs_filled, 1,
        "only the documented blob fills a row; the undocumented one is left quietly alone",
    );
    assert_eq!(report.docs_unparseable, 0);
    assert!(!report.skipped(), "{:?}", report.skips);
    assert_eq!(
        store.latest_active_version_docs("alice#toolbox").unwrap(),
        Some(("2.0.0".to_string(), section.clone())),
    );
    assert!(
        render_text(&report).contains("doc sections: 1 recorded, 0 undecodable"),
        "{}",
        render_text(&report),
    );

    let again = run(&store, &blob_store).await.unwrap();
    assert_eq!(again.docs_filled, 0, "a second sweep records nothing new");
    assert_eq!(
        store.latest_active_version_docs("alice#toolbox").unwrap(),
        Some(("2.0.0".to_string(), section)),
    );
    assert!(render_text(&again).contains("doc sections: 0 recorded, 0 undecodable"));
}

/// A stored blob whose section 17 does not decode is counted separately,
/// makes the run report itself as skipped, and writes no row -- but does not
/// stop the rest of that version's metadata from being backfilled.
#[tokio::test]
async fn a_malformed_doc_section_is_counted_skipped_and_not_recorded() {
    let (_temp, store, blob_store, dir) = harness().await;
    register_keys(&store, "alice");
    let alice_id = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
    let mut truncated = sample_doc_section();
    truncated.truncate(truncated.len() - 3);
    assert!(
        mfb_wire::docs::read_doc_table(&truncated).is_err(),
        "the fixture must genuinely fail to decode, or this test proves nothing",
    );
    publish_stored(
        &store,
        &dir,
        alice_id,
        "1.0.0",
        &serialize("alice", "", payload_with_docs("alice", "", truncated)),
    );

    let report = run(&store, &blob_store).await.unwrap();
    assert_eq!(
        report.updated, 1,
        "the doc defect does not block the rest of the metadata"
    );
    assert_eq!(report.docs_filled, 0);
    assert_eq!(report.docs_unparseable, 1);
    assert!(
        report.skipped(),
        "an undecodable doc section in a stored, signed blob is a finding",
    );
    assert!(
        report
            .skips
            .iter()
            .any(|skip| skip.contains("doc section does not decode")),
        "{:?}",
        report.skips,
    );
    assert_eq!(
        store.latest_active_version_docs("alice#toolbox").unwrap(),
        None
    );
    assert!(render_text(&report).contains("doc sections: 0 recorded, 1 undecodable"));
}

/// A sweep over a database whose versions were published before the
/// metadata columns existed fills them in, and a second run changes
/// nothing — the property that lets an operator re-run it safely.
#[tokio::test]
async fn backfill_populates_metadata_and_is_idempotent() {
    let (_temp, store, blob_store, dir) = harness().await;
    register_keys(&store, "alice");
    let alice_id = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;

    let artifact = crate::backfill::tests::package_bytes("alice", "https://example.invalid");
    let hash = hex::encode(crate::crypto::sha256(&artifact));
    std::fs::write(dir.join(format!("{hash}.mfp")), &artifact).unwrap();

    // Publish with the columns stubbed out, the way a pre-plan-61 server
    // would have left them.
    store
        .publish_package_version(
            alice_id,
            "alice#toolbox",
            "1.0.0",
            &hash,
            &format!("data/{hash}.mfp"),
            "{}",
            &[],
            &PublishMetadata::default(),
        )
        .unwrap();
    assert_eq!(
        store.version_metadata_for_test("alice#toolbox", "1.0.0"),
        (None, None),
    );

    let report = run(&store, &blob_store).await.unwrap();
    assert_eq!(report.updated, 1);
    assert!(!report.skipped());
    assert_eq!(
        store.version_metadata_for_test("alice#toolbox", "1.0.0"),
        (
            Some("alice".to_string()),
            Some("https://example.invalid".to_string())
        ),
    );
    let after_first = store.target_rows_for_test();
    assert_eq!(after_first.len(), 1, "the section-10 locator was captured");

    // Idempotent: the second run rewrites the same rows, it does not add a
    // second copy of every target.
    let report = run(&store, &blob_store).await.unwrap();
    assert_eq!(report.updated, 1);
    assert_eq!(store.target_rows_for_test(), after_first);
}

/// A blob whose header and signed manifest disagree is skipped, counted
/// apart from an unparseable blob, and leaves its columns NULL. Backfill
/// walks artifacts published before the Phase-3 check existed, so this is
/// reachable in a way a fresh publish is not.
#[tokio::test]
async fn a_metadata_mismatch_is_skipped_counted_and_left_untouched() {
    let (_temp, store, blob_store, dir) = harness().await;
    register_keys(&store, "alice");
    let alice_id = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;

    // Header says "alice", the signed manifest says "mallory".
    let artifact = mismatched_package_bytes();
    let hash = hex::encode(crate::crypto::sha256(&artifact));
    std::fs::write(dir.join(format!("{hash}.mfp")), &artifact).unwrap();

    // An unparseable blob alongside it, to prove the two counters are not
    // the same counter.
    std::fs::write(dir.join("deadbeef.mfp"), b"not a package").unwrap();

    for (version, hash) in [("1.0.0", hash.as_str()), ("2.0.0", "deadbeef")] {
        store
            .publish_package_version(
                alice_id,
                "alice#toolbox",
                version,
                hash,
                &format!("data/{hash}.mfp"),
                "{}",
                &[],
                &PublishMetadata::default(),
            )
            .unwrap();
    }

    let report = run(&store, &blob_store).await.unwrap();
    assert_eq!(report.updated, 0);
    assert_eq!(report.mismatched, 1, "the mismatch has its own counter");
    assert_eq!(report.unparseable, 1, "and is not lumped in with this one");
    assert!(report.skipped());
    assert!(
        report.skips.iter().any(|skip| skip.contains("mallory")
            && skip.contains("disagree")
            && skip.contains("left untouched")),
        "{:?}",
        report.skips,
    );
    // Neither version was rewritten: a disagreement is never resolved by
    // silently picking a copy.
    assert_eq!(
        store.version_metadata_for_test("alice#toolbox", "1.0.0"),
        (None, None),
    );
    assert!(store.target_rows_for_test().is_empty());
    assert!(render_text(&report).contains("1 mismatched"));
}

/// plan-61-E: backfill fills `description` from section 18, and a blob
/// **without** section 18 — every package built before plan-61-D — leaves it
/// NULL without being counted or logged. That absence is the expected state
/// of an existing registry, not a finding; counting it would bury the real
/// findings under noise.
#[tokio::test]
async fn backfill_fills_descriptions_and_stays_quiet_about_packages_that_have_none() {
    let (_temp, store, blob_store, dir) = harness().await;
    register_keys(&store, "alice");
    let alice_id = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;

    // One artifact with a section 18, one without.
    let described = serialize(
        "alice",
        "",
        payload_with_description("alice", "", "A described package."),
    );
    let plain = package_bytes("alice", "");
    for artifact in [&described, &plain] {
        let hash = hex::encode(crate::crypto::sha256(artifact));
        std::fs::write(dir.join(format!("{hash}.mfp")), artifact).unwrap();
    }
    for (version, artifact) in [("1.0.0", &described), ("2.0.0", &plain)] {
        let hash = hex::encode(crate::crypto::sha256(artifact));
        store
            .publish_package_version(
                alice_id,
                "alice#toolbox",
                version,
                &hash,
                &format!("data/{hash}.mfp"),
                "{}",
                &[],
                &PublishMetadata::default(),
            )
            .unwrap();
    }

    let report = run(&store, &blob_store).await.unwrap();
    assert_eq!(report.updated, 2);
    assert!(
        !report.skipped(),
        "a package with no section 18 is not a skip: {:?}",
        report.skips,
    );
    assert!(report.skips.is_empty(), "{:?}", report.skips);

    assert_eq!(
        store.version_description_for_test("alice#toolbox", "1.0.0"),
        Some("A described package.".to_string()),
    );
    assert_eq!(
        store.version_description_for_test("alice#toolbox", "2.0.0"),
        None,
        "a pre-plan-61-D package stays NULL",
    );
}

/// A version whose blob is gone is counted as missing rather than aborting
/// the sweep, so one hole does not hide every other version's metadata.
#[tokio::test]
async fn a_missing_blob_is_counted_and_the_sweep_continues() {
    let (_temp, store, blob_store, dir) = harness().await;
    register_keys(&store, "alice");
    let alice_id = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;

    let artifact = package_bytes("alice", "");
    let good_hash = hex::encode(crate::crypto::sha256(&artifact));
    std::fs::write(dir.join(format!("{good_hash}.mfp")), &artifact).unwrap();

    // The missing one is published first, so a sweep that aborted on it
    // would never reach the good one.
    for (version, hash) in [("1.0.0", "absent"), ("2.0.0", good_hash.as_str())] {
        store
            .publish_package_version(
                alice_id,
                "alice#toolbox",
                version,
                hash,
                &format!("data/{hash}.mfp"),
                "{}",
                &[],
                &PublishMetadata::default(),
            )
            .unwrap();
    }

    let report = run(&store, &blob_store).await.unwrap();
    assert_eq!(report.missing, 1);
    assert_eq!(report.updated, 1, "the sweep continued past the hole");
    assert_eq!(
        store.version_metadata_for_test("alice#toolbox", "2.0.0").0,
        Some("alice".to_string()),
    );
    // An empty url stays NULL rather than becoming "".
    assert_eq!(
        store.version_metadata_for_test("alice#toolbox", "2.0.0").1,
        None,
    );
}

/// A blob that parses as a package but whose section-1 MANIFEST is present
/// yet too short to read `author`/`url`, so `parse_manifest_metadata` errors.
/// It is counted `unparseable` and skipped, never rewritten.
#[tokio::test]
async fn a_malformed_manifest_section_is_unparseable() {
    let (_temp, store, blob_store, dir) = harness().await;
    register_keys(&store, "alice");
    let alice_id = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;

    // Section 1 present but only 8 bytes — the author lives at offset 24.
    let payload = container(&[(1, vec![0u8; 8]), (2, string_pool(&["", "alice", ""]))]);
    let artifact = serialize("alice", "", payload);
    let hash = hex::encode(crate::crypto::sha256(&artifact));
    std::fs::write(dir.join(format!("{hash}.mfp")), &artifact).unwrap();
    store
        .publish_package_version(
            alice_id,
            "alice#toolbox",
            "1.0.0",
            &hash,
            &format!("data/{hash}.mfp"),
            "{}",
            &[],
            &PublishMetadata::default(),
        )
        .unwrap();

    let report = run(&store, &blob_store).await.unwrap();
    assert_eq!(report.updated, 0);
    assert_eq!(report.unparseable, 1);
    assert!(report.skipped());
    assert_eq!(report.skips.len(), 1, "{:?}", report.skips);
    assert_eq!(
        store.version_metadata_for_test("alice#toolbox", "1.0.0"),
        (None, None),
    );
}

/// A blob whose MANIFEST is fine but whose section-10 native-library table is
/// truncated errors in `parse_vendor_blobs`, again counted `unparseable`, with
/// no target rows written.
#[tokio::test]
async fn a_malformed_vendor_table_is_unparseable() {
    let (_temp, store, blob_store, dir) = harness().await;
    register_keys(&store, "alice");
    let alice_id = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;

    // Section 10 declares one entry but carries no data for it.
    let payload = container(&[
        (1, manifest_section(1, 0)),
        (
            2,
            string_pool(&["", "alice", "", "snd", "linux", "x86_64", "libsnd.a"]),
        ),
        (10, vec![1u8, 0, 0, 0]),
    ]);
    let artifact = serialize("alice", "", payload);
    let hash = hex::encode(crate::crypto::sha256(&artifact));
    std::fs::write(dir.join(format!("{hash}.mfp")), &artifact).unwrap();
    store
        .publish_package_version(
            alice_id,
            "alice#toolbox",
            "1.0.0",
            &hash,
            &format!("data/{hash}.mfp"),
            "{}",
            &[],
            &PublishMetadata::default(),
        )
        .unwrap();

    let report = run(&store, &blob_store).await.unwrap();
    assert_eq!(report.updated, 0);
    assert_eq!(report.unparseable, 1, "{:?}", report.skips);
    assert!(store.target_rows_for_test().is_empty());
}

/// A blob carrying no section-1 MANIFEST yields `Ok(None)` from
/// `parse_manifest_metadata`, so the `None` metadata arm runs: author/url stay
/// NULL (the header is not trusted), yet `description` from section 18 is still
/// captured and the version is counted `updated`.
#[tokio::test]
async fn no_manifest_section_still_updates_description_with_null_author() {
    let (_temp, store, blob_store, dir) = harness().await;
    register_keys(&store, "alice");
    let alice_id = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;

    // Description field (section 18), a valid vendor table, but NO section 1.
    let mut meta = Vec::new();
    put_u32(&mut meta, 1);
    put_u16(&mut meta, 1); // fieldId = description
    let text = "Described but unsigned author.";
    put_u32(&mut meta, text.len() as u32);
    meta.extend_from_slice(text.as_bytes());
    let payload = container(&[
        (
            2,
            string_pool(&["", "alice", "", "snd", "linux", "x86_64", "libsnd.a"]),
        ),
        (10, vendor_table(&[0x22; 32])),
        (18, meta),
    ]);
    // Header claims an author even though there is no signed manifest.
    let artifact = serialize("alice", "https://header.invalid", payload);
    let hash = hex::encode(crate::crypto::sha256(&artifact));
    std::fs::write(dir.join(format!("{hash}.mfp")), &artifact).unwrap();
    store
        .publish_package_version(
            alice_id,
            "alice#toolbox",
            "1.0.0",
            &hash,
            &format!("data/{hash}.mfp"),
            "{}",
            &[],
            &PublishMetadata::default(),
        )
        .unwrap();

    let report = run(&store, &blob_store).await.unwrap();
    assert_eq!(report.updated, 1);
    assert!(!report.skipped(), "{:?}", report.skips);
    // The unsigned header author/url are NOT trusted.
    assert_eq!(
        store.version_metadata_for_test("alice#toolbox", "1.0.0"),
        (None, None),
    );
    // But the description from section 18 is captured.
    assert_eq!(
        store.version_description_for_test("alice#toolbox", "1.0.0"),
        Some("Described but unsigned author.".to_string()),
    );
}

async fn harness() -> (tempfile::TempDir, Store, BlobStore, std::path::PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let db_path = temp.path().join("meta.db");
    let data_path = temp.path().join("data");
    let opened = Store::open_repository(&db_path, &data_path).unwrap();
    let blob_store = crate::blobstore::BlobBackend::Local(data_path.clone())
        .into_store()
        .await
        .unwrap();
    (temp, opened.store, blob_store, data_path)
}

fn put_u16(dst: &mut Vec<u8>, value: u16) {
    dst.extend_from_slice(&value.to_le_bytes());
}
fn put_u32(dst: &mut Vec<u8>, value: u32) {
    dst.extend_from_slice(&value.to_le_bytes());
}

fn string_pool(strings: &[&str]) -> Vec<u8> {
    let mut bytes = Vec::new();
    put_u32(&mut bytes, strings.len() as u32);
    for value in strings {
        put_u32(&mut bytes, value.len() as u32);
        bytes.extend_from_slice(value.as_bytes());
    }
    bytes
}

fn manifest_section(author_id: u32, url_id: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    for _ in 0..6 {
        put_u32(&mut bytes, 0);
    }
    put_u32(&mut bytes, author_id);
    put_u32(&mut bytes, url_id);
    for _ in 0..6 {
        put_u16(&mut bytes, 0);
    }
    for _ in 0..5 {
        put_u32(&mut bytes, 0);
    }
    bytes
}

/// One vendor locator so the sweep has a target row to capture.
fn vendor_table(hash: &[u8; 32]) -> Vec<u8> {
    let mut bytes = Vec::new();
    put_u32(&mut bytes, 1); // one entry
    put_u32(&mut bytes, 3); // logical -> "snd"
    put_u32(&mut bytes, 1); // one locator
    put_u32(&mut bytes, 4); // os -> "linux"
    put_u32(&mut bytes, 5); // arch -> "x86_64"
    bytes.push(1); // libc = glibc
    bytes.push(1); // lib_type = vendor
    put_u32(&mut bytes, 6); // source -> "libsnd.a"
    bytes.extend_from_slice(hash);
    bytes
}

fn container(sections: &[(u16, Vec<u8>)]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"MFPC");
    put_u16(&mut bytes, 2);
    put_u16(&mut bytes, 0);
    put_u32(&mut bytes, 0);
    put_u32(&mut bytes, sections.len() as u32);
    let mut data_offset = 16 + sections.len() * 24;
    for (id, data) in sections {
        put_u16(&mut bytes, *id);
        put_u16(&mut bytes, 0);
        put_u32(&mut bytes, 0);
        bytes.extend_from_slice(&(data_offset as u64).to_le_bytes());
        bytes.extend_from_slice(&(data.len() as u64).to_le_bytes());
        data_offset += data.len();
    }
    for (_id, data) in sections {
        bytes.extend_from_slice(data);
    }
    bytes
}

/// A payload that additionally carries MFPC section 18.
fn payload_with_description(author: &str, url: &str, description: &str) -> Vec<u8> {
    let strings = string_pool(&["", author, url, "snd", "linux", "x86_64", "libsnd.a"]);
    let author_id = if author.is_empty() { 0 } else { 1 };
    let url_id = if url.is_empty() { 0 } else { 2 };
    let mut meta = Vec::new();
    put_u32(&mut meta, 1);
    put_u16(&mut meta, 1); // fieldId = description
    put_u32(&mut meta, description.len() as u32);
    meta.extend_from_slice(description.as_bytes());
    container(&[
        (1, manifest_section(author_id, url_id)),
        (2, strings),
        (10, vendor_table(&[0x11; 32])),
        (18, meta),
    ])
}

fn payload_for(author: &str, url: &str) -> Vec<u8> {
    // strings: 0="", 1=author, 2=url, 3="snd", 4="linux", 5="x86_64", 6="libsnd.a"
    let strings = string_pool(&["", author, url, "snd", "linux", "x86_64", "libsnd.a"]);
    let author_id = if author.is_empty() { 0 } else { 1 };
    let url_id = if url.is_empty() { 0 } else { 2 };
    container(&[
        (1, manifest_section(author_id, url_id)),
        (2, strings),
        (10, vendor_table(&[0x11; 32])),
    ])
}

/// A `.mfp` whose header and manifest agree.
pub(super) fn package_bytes(author: &str, url: &str) -> Vec<u8> {
    serialize(author, url, payload_for(author, url))
}

/// A `.mfp` whose header says "alice" and whose signed manifest says
/// "mallory" — the shape publish now refuses but old blobs may carry.
fn mismatched_package_bytes() -> Vec<u8> {
    serialize("alice", "", payload_for("mallory", ""))
}

/// A structurally complete signed `.mfp`. The sweep only ever *parses*
/// these — it re-reads stored artifacts and never re-verifies a signature,
/// which the registry already did at publish — but the parser still
/// requires the identity fields to be present.
fn serialize(header_author: &str, header_url: &str, payload: Vec<u8>) -> Vec<u8> {
    let (ident_public, _ident_private) = crate::crypto::generate_keypair();
    let (signing_public, signing_private) = crate::crypto::generate_keypair();
    crate::package::test_support::serialize(
        &crate::package::test_support::TestPackage {
            name: "toolbox".to_string(),
            ident: "alice#toolbox".to_string(),
            version: "1.0.0".to_string(),
            author: header_author.to_string(),
            url: header_url.to_string(),
            payload,
            ident_key: format!("ed25519:{}", crate::crypto::encode_bytes(&ident_public)),
            signing_key: format!("ed25519:{}", crate::crypto::encode_bytes(&signing_public)),
            proof: "{}".to_string(),
            proof_sig: vec![0; 64],
            attestation: "{}".to_string(),
            attestation_sig: vec![0; 64],
        },
        &signing_private,
    )
}
