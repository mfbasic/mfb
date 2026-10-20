//! Package-blob garbage collection (plan-49).
//!
//! The registry has never deleted a package blob. That was defensible while a
//! blob was a small `.mfp` of compiled IR that was always referenced by the
//! `package_versions` row created in the same transaction. plan-48 broke all
//! three properties at once: vendored native libraries are megabytes, there are
//! up to seven per binding, and `PUT /blob` accepts a blob **before** anything
//! references it — so a publisher who uploads and then abandons (network
//! failure, failed validation, `^C`) leaves bytes nothing will ever name.
//!
//! The mechanism is deliberately boring: **mark** the reachable set from truth
//! (`package_versions` ∪ `package_version_blobs`), **sweep** what is left after
//! a grace period, and report — deleting only when the operator says
//! `--delete`.
//!
//! Three properties carry the safety of the whole design:
//!
//!   * **The grace period is the concurrency design** (§3.1). There is no lock
//!     between `PUT /blob` and `POST /publish`, so a publisher's five uploaded
//!     blobs genuinely *are* unreachable until the publish lands. A 24h default
//!     window on `package_blobs.created_at` means a blob is only ever collected
//!     long after any plausible publish has finished or died — no lock, no
//!     lease, no two-phase protocol. `--grace-hours 0` is refused because it
//!     removes the only protection there is.
//!   * **Store before DB** (§4.4). The inverse of the publish path's ordering,
//!     on purpose: a crash mid-delete leaves a row pointing at a missing object,
//!     which the next run re-collects idempotently. The other order leaves an
//!     object with no row — invisible to every future run, unreclaimable
//!     forever. Neither is atomic; pick the failure that self-heals.
//!   * **Recompute, never refcount** (§3.4). A drifted refcount either leaks
//!     forever or deletes a live blob. A scan from truth is self-correcting.
//!
//! The risk here is entirely one-sided: collecting a live blob is catastrophic
//! and silent, while failing to collect garbage costs disk. Every default leans
//! that way on purpose.

use crate::blobstore::{BlobKind, BlobStore};
use crate::store::Store;

/// Grace period default (§3.1) — comfortably longer than any plausible publish.
pub const DEFAULT_GRACE_HOURS: i64 = 24;

const SECONDS_PER_HOUR: i64 = 3600;

/// How a `gc` run should behave. Constructed by the CLI parser, which is where
/// `--grace-hours` is range-checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GcOptions {
    pub grace_hours: i64,
    /// Perform the deletion. Default `false`: a `gc` an operator runs to *look*
    /// must never be one that acts.
    pub delete: bool,
    /// Emit machine-readable JSON, and — because the caller has asked for a
    /// full accounting — stat the reachable blobs too, so the report can say
    /// what fraction of the store is garbage. Text mode deliberately skips
    /// that: it is one backend round trip per *reachable* blob, which on a
    /// large registry dwarfs the candidate scan and answers a question the
    /// operator did not ask.
    pub json: bool,
}

impl Default for GcOptions {
    fn default() -> Self {
        GcOptions {
            grace_hours: DEFAULT_GRACE_HOURS,
            delete: false,
            json: false,
        }
    }
}

/// Validate an operator-supplied `--grace-hours` (§4.5).
///
/// `0` is **refused**, not clamped. It removes the only concurrency protection
/// the design has, and a sweep of a quiesced registry is a distinct decision
/// that deserves a distinct, explicitly-named flag rather than falling out of a
/// zero. Negative and overflowing values are refused for the same reason a
/// negative `--expires-days` is (bug-276 R10): operator input reaching integer
/// arithmetic unchecked.
pub fn grace_seconds(grace_hours: i64) -> Result<i64, String> {
    if grace_hours <= 0 {
        return Err(format!(
            "--grace-hours must be a positive number of hours (got {grace_hours}); \
             the grace period is what makes the sweep safe against an in-flight \
             publish, so 0 is refused"
        ));
    }
    grace_hours
        .checked_mul(SECONDS_PER_HOUR)
        .ok_or_else(|| format!("--grace-hours {grace_hours} is too large"))
}

/// One unreachable blob in a `gc` report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GcBlob {
    pub hash: String,
    pub kind: String,
    /// Where the bytes live, recomputed from the *live* backend rather than
    /// read from the stale `package_blobs.path` column.
    pub blob_ref: String,
    /// Stored size, or `None` when the backing object is already gone — the
    /// signature of an interrupted earlier delete (§4.4).
    pub size: Option<u64>,
    pub age_seconds: i64,
    /// Whether this run actually removed it (`--delete` only).
    pub deleted: bool,
}

/// The outcome of a `gc` run.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GcReport {
    pub grace_hours: i64,
    /// Whether this was a `--delete` run or a dry run.
    pub deleting: bool,
    pub unreachable: Vec<GcBlob>,
    /// Bytes held by unreachable blobs whose objects still exist.
    pub unreachable_bytes: u64,
    pub deleted_count: usize,
    pub deleted_bytes: u64,
    pub reachable_count: usize,
    /// Only computed in `--json` mode (see [`GcOptions::json`]).
    pub reachable_bytes: Option<u64>,
    /// Per-blob failures. A single unreadable or undeletable object must not
    /// abandon the rest of the sweep, so failures are collected and reported
    /// rather than propagated — but they make the run exit nonzero.
    pub errors: Vec<String>,
}

impl GcReport {
    pub fn failed(&self) -> bool {
        !self.errors.is_empty()
    }
}

/// Run the collector. `now` is passed in rather than read from the clock so the
/// candidate set at a given instant is reproducible and testable.
pub async fn run(
    store: &Store,
    blobs: &BlobStore,
    options: &GcOptions,
    now: i64,
) -> Result<GcReport, String> {
    let grace = grace_seconds(options.grace_hours)?;
    let candidates = store.unreachable_blobs(now, grace)?;
    let reachable = store.reachable_blobs()?;

    let mut report = GcReport {
        grace_hours: options.grace_hours,
        deleting: options.delete,
        reachable_count: reachable.len(),
        ..GcReport::default()
    };

    for row in candidates {
        let kind = match BlobKind::from_db_str(&row.kind) {
            Ok(kind) => kind,
            // An unparseable kind means we cannot name the backing object. Do
            // not guess and do not drop the row: deleting metadata we cannot
            // match to bytes would strand the object forever (§4.4's "DB then
            // store" failure, arrived at by another route).
            Err(err) => {
                report
                    .errors
                    .push(format!("skipped blob {}: {err}", row.hash));
                continue;
            }
        };
        let blob_ref = blobs.blob_ref(&row.hash, kind);
        let size = match blobs.size(&row.hash, kind).await {
            Ok(size) => size,
            Err(err) => {
                report
                    .errors
                    .push(format!("failed to stat blob {}: {err}", row.hash));
                None
            }
        };
        if let Some(size) = size {
            report.unreachable_bytes = report.unreachable_bytes.saturating_add(size);
        }

        let mut deleted = false;
        // Re-ask reachability immediately before deleting, rather than trusting
        // the scan this run started with. A publish that lands mid-sweep can
        // reference a blob that was a legitimate candidate a moment ago — the
        // grace period covers an in-flight publish, not a slow publisher whose
        // upload predates it by more than the window.
        let still_unreachable = if options.delete {
            match store.blob_is_reachable(&row.hash) {
                Ok(false) => true,
                Ok(true) => {
                    report.errors.push(format!(
                        "skipped blob {}: a publish referenced it during this sweep",
                        row.hash
                    ));
                    false
                }
                // Never delete on a failed check. Leaking a blob costs disk;
                // deleting a live one is unrecoverable.
                Err(err) => {
                    report.errors.push(format!(
                        "skipped blob {}: could not re-check reachability: {err}",
                        row.hash
                    ));
                    false
                }
            }
        } else {
            false
        };
        if still_unreachable {
            // Store first, then the DB row (§4.4).
            match blobs.delete(&row.hash, kind).await {
                Ok(()) => match store.forget_blob(&row.hash) {
                    Ok(_) => {
                        deleted = true;
                        report.deleted_count += 1;
                        report.deleted_bytes = report
                            .deleted_bytes
                            .saturating_add(size.unwrap_or_default());
                    }
                    Err(err) => report.errors.push(format!(
                        "deleted object for {} but failed to remove its row: {err} \
                         (the next gc run will re-collect it)",
                        row.hash
                    )),
                },
                Err(err) => report
                    .errors
                    .push(format!("failed to delete blob {}: {err}", row.hash)),
            }
        }

        report.unreachable.push(GcBlob {
            hash: row.hash,
            kind: row.kind,
            blob_ref,
            size,
            age_seconds: now.saturating_sub(row.created_at),
            deleted,
        });
    }

    if options.json {
        let mut total = 0u64;
        for row in &reachable {
            let Ok(kind) = BlobKind::from_db_str(&row.kind) else {
                report
                    .errors
                    .push(format!("unknown kind on reachable blob {}", row.hash));
                continue;
            };
            match blobs.size(&row.hash, kind).await {
                Ok(Some(size)) => total = total.saturating_add(size),
                // A reachable blob with no object is a real integrity problem —
                // a live version whose download 404s — and worth surfacing even
                // though gc is not what repairs it.
                Ok(None) => report
                    .errors
                    .push(format!("reachable blob {} has no backing object", row.hash)),
                Err(err) => report
                    .errors
                    .push(format!("failed to stat reachable blob {}: {err}", row.hash)),
            }
        }
        report.reachable_bytes = Some(total);
    }

    Ok(report)
}

/// Human-readable report.
pub fn render_text(report: &GcReport) -> String {
    let mut out = String::new();
    let mode = if report.deleting {
        "deleting"
    } else {
        "dry run"
    };
    out.push_str(&format!(
        "mfb-repo gc — grace period {}h ({mode})\n",
        report.grace_hours
    ));
    if report.unreachable.is_empty() {
        out.push_str(&format!(
            "\nNo unreachable blobs older than {}h.\n",
            report.grace_hours
        ));
    } else {
        out.push('\n');
        for blob in &report.unreachable {
            out.push_str(&format!(
                "  {}  {:<7}  {:>10}  {:>7}  {}\n",
                blob.hash,
                blob.kind,
                match blob.size {
                    Some(size) => format_bytes(size),
                    None => "missing".to_string(),
                },
                format_age(blob.age_seconds),
                blob.blob_ref,
            ));
        }
        out.push('\n');
        out.push_str(&format!(
            "{} unreachable blob{}, {} reclaimable.\n",
            report.unreachable.len(),
            if report.unreachable.len() == 1 {
                ""
            } else {
                "s"
            },
            format_bytes(report.unreachable_bytes),
        ));
    }
    out.push_str(&format!(
        "Reachable: {} blob{} (never collected, including yanked versions).\n",
        report.reachable_count,
        if report.reachable_count == 1 { "" } else { "s" },
    ));
    if let Some(bytes) = report.reachable_bytes {
        out.push_str(&format!("Reachable bytes: {}\n", format_bytes(bytes)));
    }
    if report.deleting {
        out.push_str(&format!(
            "Deleted {} blob{}, freed {}.\n",
            report.deleted_count,
            if report.deleted_count == 1 { "" } else { "s" },
            format_bytes(report.deleted_bytes),
        ));
    } else if !report.unreachable.is_empty() {
        out.push_str("Run again with --delete to reclaim them.\n");
    }
    for err in &report.errors {
        out.push_str(&format!("error: {err}\n"));
    }
    out
}

/// Machine-readable report, for operators who script the sweep.
pub fn render_json(report: &GcReport) -> String {
    let blobs: Vec<serde_json::Value> = report
        .unreachable
        .iter()
        .map(|blob| {
            serde_json::json!({
                "hash": blob.hash,
                "kind": blob.kind,
                "blobRef": blob.blob_ref,
                "size": blob.size,
                "ageSeconds": blob.age_seconds,
                "deleted": blob.deleted,
            })
        })
        .collect();
    serde_json::to_string_pretty(&serde_json::json!({
        "graceHours": report.grace_hours,
        "deleting": report.deleting,
        "unreachable": blobs,
        "unreachableCount": report.unreachable.len(),
        "unreachableBytes": report.unreachable_bytes,
        "deletedCount": report.deleted_count,
        "deletedBytes": report.deleted_bytes,
        "reachableCount": report.reachable_count,
        "reachableBytes": report.reachable_bytes,
        "errors": report.errors,
    }))
    .expect("JSON report encoding cannot fail")
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn format_age(seconds: i64) -> String {
    let seconds = seconds.max(0);
    if seconds < 3600 {
        format!("{}m", seconds / 60)
    } else if seconds < 86_400 {
        format!("{}h", seconds / 3600)
    } else {
        format!("{}d", seconds / 86_400)
    }
}

#[cfg(test)]
mod tests;
