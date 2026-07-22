//! `mfb-repo backfill-metadata` — populate the plan-61-A metadata for versions
//! published before the server captured it.
//!
//! Every field this writes is already present in bytes the registry already
//! stores: `author` and `url` in the signed MANIFEST section, and the native
//! target matrix in section 10. Nothing is republished and no publisher action
//! is required — the sweep re-parses each stored package blob and fills in what
//! the old publish path discarded.
//!
//! Two rules shape the whole sweep, both about not lying to the operator:
//!
//! - **One bad blob does not abandon the run.** An old blob that no longer
//!   parses is exactly the kind of thing this surfaces; aborting on the first
//!   one would leave the rest unmeasured and unfixed.
//! - **A header/MANIFEST mismatch is skipped and counted separately, never
//!   silently resolved.** Publish refuses such a package (plan-61-A Phase 3),
//!   but backfill walks blobs published *before* that check existed, so they
//!   are parseable-and-invalid — a case the skip-unparseable rule does not
//!   cover. Two stored copies of `author` that disagree is a transparency
//!   finding an operator must see, and this sweep is the only thing that will
//!   ever look. The version row is left alone: not rewritten, not deleted.

use crate::abi;
use crate::blobstore::{BlobFetch, BlobKind, BlobStore};
use crate::package;
use crate::store::{PublishMetadata, Store};

/// What one `backfill-metadata` run did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BackfillReport {
    /// Versions whose metadata was rewritten.
    pub updated: usize,
    /// Versions whose blob is absent from the blob store.
    pub missing: usize,
    /// Versions whose blob could not be parsed at all.
    pub unparseable: usize,
    /// Versions whose header and signed MANIFEST disagree — deliberately
    /// counted apart from `unparseable`, because the blob parsed fine and the
    /// disagreement is the finding.
    pub mismatched: usize,
    /// One line per skipped version, in sweep order.
    pub skips: Vec<String>,
}

impl BackfillReport {
    /// Whether anything was skipped. The caller exits non-zero on this so a
    /// scripted sweep cannot read a partial run as a clean one.
    pub fn skipped(&self) -> bool {
        self.missing > 0 || self.unparseable > 0 || self.mismatched > 0
    }
}

/// Re-parse every stored package blob and rewrite its captured metadata.
///
/// Idempotent: `replace_version_metadata` clears a version's target rows before
/// inserting, so a second run produces byte-identical state rather than
/// doubling every row.
pub async fn run(store: &Store, blob_store: &BlobStore) -> Result<BackfillReport, String> {
    let mut report = BackfillReport::default();

    for (version_id, ident, version, hash) in store.all_package_versions()? {
        let label = format!("{ident}@{version}");

        let bytes = match blob_store.get(&hash, BlobKind::Package).await? {
            Some(BlobFetch::Bytes(bytes)) => bytes,
            // An S3 backend answers a fetch with a presigned redirect rather
            // than bytes. Downloading through it would make an operator
            // ceremony depend on the blob store's public URL being reachable
            // from wherever the sweep runs, so it is reported rather than
            // guessed at.
            Some(BlobFetch::Redirect(_)) => {
                report.missing += 1;
                report
                    .skips
                    .push(format!("{label}: blob is only available by redirect"));
                continue;
            }
            None => {
                report.missing += 1;
                report.skips.push(format!(
                    "{label}: blob {hash} is missing from the blob store"
                ));
                continue;
            }
        };

        let parsed = match package::parse_mfp_package(&bytes) {
            Ok(parsed) => parsed,
            Err(err) => {
                report.unparseable += 1;
                report.skips.push(format!("{label}: {err}"));
                continue;
            }
        };

        let signed = match abi::parse_manifest_metadata(&parsed.payload) {
            Ok(signed) => signed,
            Err(err) => {
                report.unparseable += 1;
                report.skips.push(format!("{label}: {err}"));
                continue;
            }
        };
        if let Some(signed) = signed.as_ref() {
            if signed.author != parsed.author || signed.url != parsed.url {
                report.mismatched += 1;
                report.skips.push(format!(
                    "{label}: header and signed manifest disagree \
                     (header author {:?} url {:?}, manifest author {:?} url {:?}) \
                     — left untouched",
                    parsed.author, parsed.url, signed.author, signed.url,
                ));
                continue;
            }
        }

        let vendor_blobs = match abi::parse_vendor_blobs(&parsed.payload) {
            Ok(refs) => refs,
            Err(err) => {
                report.unparseable += 1;
                report.skips.push(format!("{label}: {err}"));
                continue;
            }
        };

        // plan-61-E: a blob with no section 18 was built before plan-61-D. That
        // is the expected state for every package already on a live registry,
        // so it leaves `description` NULL and is deliberately **not** counted
        // as a skip — logging it would bury the real findings under noise.
        let description = abi::parse_package_description(&parsed.payload)
            .ok()
            .flatten()
            .filter(|value| !value.is_empty());
        let metadata = match signed {
            Some(signed) => PublishMetadata {
                author: Some(signed.author).filter(|value| !value.is_empty()),
                url: Some(signed.url).filter(|value| !value.is_empty()),
                description,
            },
            None => PublishMetadata {
                description,
                ..Default::default()
            },
        };
        store.replace_version_metadata(version_id, &metadata, &vendor_blobs)?;
        report.updated += 1;
    }

    Ok(report)
}

/// Render a report for an operator's terminal.
pub fn render_text(report: &BackfillReport) -> String {
    let mut out = String::new();
    for skip in &report.skips {
        out.push_str(&format!("skipped {skip}\n"));
    }
    out.push_str(&format!(
        "backfilled {} version(s); skipped {} missing, {} unparseable, {} mismatched\n",
        report.updated, report.missing, report.unparseable, report.mismatched,
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::tests::register_keys;

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
}
