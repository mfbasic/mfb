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
    /// Versions whose doc section (MFPC section 17) was recorded by this run
    /// (plan-126-E). Counts real inserts only, so a second run reports zero.
    pub docs_filled: usize,
    /// Versions whose blob carries a section 17 that does not decode. Counted
    /// apart, and never silently treated as "no documentation": an undecodable
    /// doc section in a stored, signed blob is a finding an operator must see.
    pub docs_unparseable: usize,
    /// One line per skipped version, in sweep order.
    pub skips: Vec<String>,
}

impl BackfillReport {
    /// Whether anything was skipped. The caller exits non-zero on this so a
    /// scripted sweep cannot read a partial run as a clean one.
    pub fn skipped(&self) -> bool {
        self.missing > 0 || self.unparseable > 0 || self.mismatched > 0 || self.docs_unparseable > 0
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
            #[cfg(feature = "s3")]
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
                docs: None,
                // `replace_version_metadata` rewrites author/url/description
                // only; the publisher recorded at publish time stays as it is.
                publisher_ident_fingerprint: None,
            },
            None => PublishMetadata {
                description,
                ..Default::default()
            },
        };
        store.replace_version_metadata(version_id, &metadata, &vendor_blobs)?;
        report.updated += 1;

        // plan-126-E: fill the version's doc section (MFPC section 17) from the
        // blob it was published as. This runs only after every check above has
        // passed, so a mismatched or unparseable blob -- which this sweep leaves
        // untouched -- never gains a docs row either.
        //
        // Three outcomes, kept distinct on purpose. A payload that is not a
        // container, or one with no section 17, is the normal undocumented case
        // and stays quiet, exactly as a missing section 10 or 18 does above. A
        // section 17 that is present but does not decode is a finding: counted,
        // logged, and not recorded. Only a section that decodes is stored, and
        // it is stored raw so `mfb_wire::docs` stays the only decoder.
        if let Ok(sections) = mfb_wire::mfpc::read_section_table(&parsed.payload) {
            if let Some(section) = sections.get(&mfb_wire::mfpc::SECTION_DOC_TABLE) {
                match mfb_wire::docs::read_doc_table(section) {
                    Ok(_) => {
                        if store.put_version_docs(version_id, section)? {
                            report.docs_filled += 1;
                        }
                    }
                    Err(err) => {
                        report.docs_unparseable += 1;
                        report.skips.push(format!(
                            "{label}: doc section does not decode ({err}) -- not recorded"
                        ));
                    }
                }
            }
        }
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
    // A separate line, and "undecodable" rather than "unparseable", so the doc
    // counters can never be confused with the blob counters above.
    out.push_str(&format!(
        "doc sections: {} recorded, {} undecodable\n",
        report.docs_filled, report.docs_unparseable,
    ));
    out
}

#[cfg(test)]
mod tests;
