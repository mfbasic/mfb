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
mod tests {
    use super::*;
    use crate::store::now_unix;

    const DAY: i64 = 86_400;

    /// A store plus a local blob backend sharing one temp dir.
    fn fixture() -> (tempfile::TempDir, Store, BlobStore) {
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("data");
        let opened = Store::open_repository(&temp.path().join("meta.db"), &data).unwrap();
        (temp, opened.store, BlobStore::local(data))
    }

    /// Put real bytes behind a hash so the size/delete paths are exercised
    /// against a live backend rather than a bookkeeping-only fake.
    async fn put_object(blobs: &BlobStore, hash: &str, kind: BlobKind, bytes: &[u8]) {
        let staged = blobs.stage(hash, kind, bytes.to_vec()).await.unwrap();
        blobs.promote(staged).await.unwrap();
    }

    fn owner_id(store: &Store) -> i64 {
        crate::store::tests::register_keys(store, "alice");
        store.owner_with_ident_key("alice").unwrap().unwrap().0.id
    }

    /// A second connection onto the fixture's SQLite file. Used only to stage
    /// states the public API deliberately cannot produce — a `kind` column this
    /// build cannot parse, or a DB that refuses a row delete — so the
    /// collector's damage-control arms can be exercised against a real store.
    fn raw_conn(temp: &tempfile::TempDir) -> rusqlite::Connection {
        rusqlite::Connection::open(temp.path().join("meta.db")).unwrap()
    }

    /// Insert a `package_blobs` row with an arbitrary `kind`, bypassing the
    /// store API (which only ever writes `package`/`native`).
    fn raw_blob_row(temp: &tempfile::TempDir, hash: &str, kind: &str, created_at: i64) {
        raw_conn(temp)
            .execute(
                "INSERT INTO package_blobs (hash, path, kind, created_at)
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![hash, format!("data/{hash}.xyz"), kind, created_at],
            )
            .unwrap();
    }

    fn publish(store: &Store, owner: i64, version: &str, hash: &str, vendor: &[String]) {
        // These tests care only about the version→blob edges reachability is
        // computed from, so the platform axis is filled in with a placeholder.
        let vendor: Vec<_> = vendor
            .iter()
            .map(|hash| crate::abi::vendor_ref_for_hash(hash))
            .collect();
        store
            .publish_package_version(
                owner,
                "alice#toolbox",
                version,
                hash,
                &format!("data/{hash}.mfp"),
                "{}",
                &vendor,
            )
            .unwrap();
    }

    /// The dry run names exactly the orphan, leaves it on disk, and reports its
    /// real size — the plan-49 Phase-1 acceptance in miniature.
    #[tokio::test]
    async fn dry_run_reports_only_the_orphan_and_deletes_nothing() {
        let (_temp, store, blobs) = fixture();
        let owner = owner_id(&store);
        publish(&store, owner, "1.0.0", "livehash", &[]);
        put_object(&blobs, "livehash", BlobKind::Package, b"live package").await;

        store
            .record_native_blob("orphanhash", "data/orphanhash.bin")
            .unwrap();
        put_object(&blobs, "orphanhash", BlobKind::Native, b"orphaned bytes").await;

        let report = run(&store, &blobs, &GcOptions::default(), now_unix() + 2 * DAY)
            .await
            .unwrap();

        assert_eq!(report.unreachable.len(), 1);
        assert_eq!(report.unreachable[0].hash, "orphanhash");
        assert_eq!(report.unreachable[0].kind, "native");
        assert_eq!(report.unreachable[0].size, Some(14));
        assert_eq!(report.unreachable_bytes, 14);
        assert!(!report.unreachable[0].deleted);
        assert_eq!(report.deleted_count, 0);
        assert_eq!(report.reachable_count, 1);
        assert!(!report.failed(), "{:?}", report.errors);
        // Nothing was touched.
        assert!(blobs.exists("orphanhash", BlobKind::Native).await.unwrap());
        assert!(blobs.exists("livehash", BlobKind::Package).await.unwrap());
        assert!(store.blob_kind("orphanhash").unwrap().is_some());

        let text = render_text(&report);
        assert!(text.contains("orphanhash"), "{text}");
        assert!(text.contains("dry run"), "{text}");
        assert!(text.contains("--delete"), "{text}");
        assert!(!text.contains("livehash"), "{text}");
    }

    /// `--delete` removes exactly the candidate set, frees the backing bytes,
    /// and leaves every reachable blob downloadable.
    #[tokio::test]
    async fn delete_reclaims_the_orphan_and_spares_live_blobs() {
        let (_temp, store, blobs) = fixture();
        let owner = owner_id(&store);
        store
            .record_native_blob("vendorhash", "data/vendorhash.bin")
            .unwrap();
        put_object(&blobs, "vendorhash", BlobKind::Native, b"vendored lib").await;
        publish(
            &store,
            owner,
            "1.0.0",
            "livehash",
            &["vendorhash".to_string()],
        );
        put_object(&blobs, "livehash", BlobKind::Package, b"live package").await;

        store
            .record_native_blob("orphanhash", "data/orphanhash.bin")
            .unwrap();
        put_object(&blobs, "orphanhash", BlobKind::Native, b"orphaned bytes").await;

        let options = GcOptions {
            delete: true,
            ..GcOptions::default()
        };
        let report = run(&store, &blobs, &options, now_unix() + 2 * DAY)
            .await
            .unwrap();

        assert!(!report.failed(), "{:?}", report.errors);
        assert_eq!(report.deleted_count, 1);
        assert_eq!(report.deleted_bytes, 14);
        assert!(report.unreachable[0].deleted);
        // Object and row are both gone.
        assert!(!blobs.exists("orphanhash", BlobKind::Native).await.unwrap());
        assert!(store.blob_kind("orphanhash").unwrap().is_none());
        // Every reachable blob still downloads — the `.mfp` and its vendor blob.
        assert!(blobs
            .get("livehash", BlobKind::Package)
            .await
            .unwrap()
            .is_some());
        assert!(blobs
            .get("vendorhash", BlobKind::Native)
            .await
            .unwrap()
            .is_some());
        assert_eq!(report.reachable_count, 2);

        // Running it again is a clean no-op: nothing left to collect.
        let second = run(&store, &blobs, &options, now_unix() + 2 * DAY)
            .await
            .unwrap();
        assert!(second.unreachable.is_empty());
        assert_eq!(second.deleted_count, 0);
        assert!(!second.failed(), "{:?}", second.errors);
    }

    /// An interrupted delete — object gone, row still present (§4.4's chosen
    /// failure) — is re-collected cleanly rather than wedging the sweep.
    #[tokio::test]
    async fn interrupted_delete_is_recollected() {
        let (_temp, store, blobs) = fixture();
        store
            .record_native_blob("halfhash", "data/halfhash.bin")
            .unwrap();
        // Deliberately no object: this is the crash-in-between state.
        let options = GcOptions {
            delete: true,
            ..GcOptions::default()
        };
        let report = run(&store, &blobs, &options, now_unix() + 2 * DAY)
            .await
            .unwrap();

        assert!(!report.failed(), "{:?}", report.errors);
        assert_eq!(report.unreachable.len(), 1);
        assert_eq!(
            report.unreachable[0].size, None,
            "a missing object reports as missing, not as zero bytes"
        );
        assert!(report.unreachable[0].deleted);
        assert_eq!(report.deleted_count, 1);
        assert_eq!(report.deleted_bytes, 0);
        assert!(store.blob_kind("halfhash").unwrap().is_none());
    }

    /// `--json` accounts for the reachable side too, so an operator can see what
    /// fraction of the store is garbage.
    #[tokio::test]
    async fn json_report_totals_reachable_bytes() {
        let (_temp, store, blobs) = fixture();
        let owner = owner_id(&store);
        publish(&store, owner, "1.0.0", "livehash", &[]);
        put_object(&blobs, "livehash", BlobKind::Package, b"0123456789").await;
        store
            .record_native_blob("orphanhash", "data/orphanhash.bin")
            .unwrap();
        put_object(&blobs, "orphanhash", BlobKind::Native, b"012").await;

        let options = GcOptions {
            json: true,
            ..GcOptions::default()
        };
        let report = run(&store, &blobs, &options, now_unix() + 2 * DAY)
            .await
            .unwrap();
        assert_eq!(report.reachable_bytes, Some(10));
        assert_eq!(report.unreachable_bytes, 3);
        assert!(!report.failed(), "{:?}", report.errors);

        let json: serde_json::Value = serde_json::from_str(&render_json(&report)).unwrap();
        assert_eq!(json["unreachableCount"], 1);
        assert_eq!(json["unreachableBytes"], 3);
        assert_eq!(json["reachableBytes"], 10);
        assert_eq!(json["unreachable"][0]["hash"], "orphanhash");
        assert_eq!(json["unreachable"][0]["deleted"], false);
        assert_eq!(json["graceHours"], DEFAULT_GRACE_HOURS);
    }

    /// A blob inside the grace window is never a candidate, however unreachable
    /// it is — this is the only thing standing between `gc` and an in-flight
    /// publish (§3.1).
    #[tokio::test]
    async fn grace_window_shields_a_fresh_orphan() {
        let (_temp, store, blobs) = fixture();
        store
            .record_native_blob("freshhash", "data/freshhash.bin")
            .unwrap();
        put_object(&blobs, "freshhash", BlobKind::Native, b"just uploaded").await;

        let options = GcOptions {
            delete: true,
            ..GcOptions::default()
        };
        // One hour after upload: well inside the 24h window.
        let report = run(&store, &blobs, &options, now_unix() + 3600)
            .await
            .unwrap();
        assert!(report.unreachable.is_empty());
        assert!(blobs.exists("freshhash", BlobKind::Native).await.unwrap());

        // Two days later the same blob is collectable.
        let report = run(&store, &blobs, &options, now_unix() + 2 * DAY)
            .await
            .unwrap();
        assert_eq!(report.deleted_count, 1);
        assert!(!blobs.exists("freshhash", BlobKind::Native).await.unwrap());
    }

    /// `--grace-hours 0` is refused, not clamped (§4.5).
    #[test]
    fn zero_and_negative_grace_are_refused() {
        assert!(grace_seconds(0).unwrap_err().contains("must be a positive"));
        assert!(grace_seconds(-1)
            .unwrap_err()
            .contains("must be a positive"));
        assert!(grace_seconds(i64::MAX).unwrap_err().contains("too large"));
        assert_eq!(grace_seconds(24).unwrap(), 86_400);
        assert_eq!(grace_seconds(DEFAULT_GRACE_HOURS).unwrap(), DAY);
    }

    /// `run` refuses a zero grace period *before* it looks at a single blob.
    /// The window is the only thing protecting an in-flight publish (§3.1), so
    /// a sweep without one must not start at all — not start and find nothing.
    #[tokio::test]
    async fn run_refuses_a_zero_grace_period_before_touching_anything() {
        let (_temp, store, blobs) = fixture();
        store
            .record_native_blob("orphanhash", "data/orphanhash.bin")
            .unwrap();
        put_object(&blobs, "orphanhash", BlobKind::Native, b"orphaned bytes").await;

        let options = GcOptions {
            grace_hours: 0,
            delete: true,
            ..GcOptions::default()
        };
        let err = run(&store, &blobs, &options, now_unix() + 2 * DAY)
            .await
            .unwrap_err();
        assert!(err.contains("must be a positive"), "{err}");
        // A blob that a 0h window would have collected is untouched.
        assert!(blobs.exists("orphanhash", BlobKind::Native).await.unwrap());
        assert!(store.blob_kind("orphanhash").unwrap().is_some());
    }

    /// A grace period that underflows the clock is refused by the scan rather
    /// than wrapping into the future and making every blob in the registry a
    /// candidate.
    #[tokio::test]
    async fn run_refuses_a_grace_period_that_underflows_the_clock() {
        let (_temp, store, blobs) = fixture();
        store
            .record_native_blob("orphanhash", "data/orphanhash.bin")
            .unwrap();
        put_object(&blobs, "orphanhash", BlobKind::Native, b"orphaned bytes").await;

        let options = GcOptions {
            delete: true,
            ..GcOptions::default()
        };
        let err = run(&store, &blobs, &options, i64::MIN).await.unwrap_err();
        assert!(err.contains("overflows the clock"), "{err}");
        assert!(blobs.exists("orphanhash", BlobKind::Native).await.unwrap());
        assert!(store.blob_kind("orphanhash").unwrap().is_some());
    }

    /// A `package_blobs` row whose `kind` this build cannot parse is reported
    /// and *kept*. We cannot name the backing object from an unknown kind, so
    /// dropping the row would strand the bytes with nothing left to name them
    /// (§4.4's failure, reached by another route).
    #[tokio::test]
    async fn unparseable_candidate_kind_is_reported_and_never_collected() {
        let (temp, store, blobs) = fixture();
        raw_blob_row(&temp, "weirdhash", "quux", now_unix());

        let options = GcOptions {
            delete: true,
            ..GcOptions::default()
        };
        let report = run(&store, &blobs, &options, now_unix() + 2 * DAY)
            .await
            .unwrap();

        assert!(report.failed());
        assert_eq!(report.errors.len(), 1, "{:?}", report.errors);
        assert!(
            report.errors[0].contains("skipped blob weirdhash"),
            "{:?}",
            report.errors
        );
        assert!(
            report.errors[0].contains("unknown blob kind 'quux'"),
            "{:?}",
            report.errors
        );
        // Not listed as reclaimable, and above all not reclaimed.
        assert!(report.unreachable.is_empty());
        assert_eq!(report.deleted_count, 0);
        assert_eq!(
            store.blob_kind("weirdhash").unwrap().as_deref(),
            Some("quux"),
            "the row we could not interpret must survive verbatim"
        );
    }

    /// A candidate whose object cannot be stat'ed is reported and still listed.
    /// A stat failure is not evidence of anything: it must neither drop the
    /// candidate nor be counted as reclaimable bytes.
    #[tokio::test]
    async fn a_stat_failure_is_reported_and_the_candidate_is_still_listed() {
        let (temp, store, blobs) = fixture();
        // A *file* standing where the object's path needs a directory, so
        // `stat`ing `data/wall/blocked.bin` fails with ENOTDIR — neither
        // success nor "no such object".
        std::fs::write(temp.path().join("data").join("wall"), b"not a directory").unwrap();
        store
            .record_native_blob("wall/blocked", "data/wall/blocked.bin")
            .unwrap();

        let report = run(&store, &blobs, &GcOptions::default(), now_unix() + 2 * DAY)
            .await
            .unwrap();

        assert!(report.failed());
        assert_eq!(report.errors.len(), 1, "{:?}", report.errors);
        assert!(
            report.errors[0].starts_with("failed to stat blob wall/blocked:"),
            "{:?}",
            report.errors
        );
        assert_eq!(report.unreachable.len(), 1);
        assert_eq!(report.unreachable[0].size, None);
        assert_eq!(
            report.unreachable_bytes, 0,
            "a blob we could not stat contributes no reclaimable bytes"
        );
        assert!(!report.unreachable[0].deleted);
        assert!(store.blob_kind("wall/blocked").unwrap().is_some());
    }

    /// When the backing object cannot be removed, the DB row must stay put.
    /// §4.4's ordering only self-heals in one direction: a row without an
    /// object is re-collected next run, an object without a row is
    /// unreclaimable forever.
    #[tokio::test]
    async fn a_failed_object_delete_leaves_the_row_in_place() {
        let (temp, store, blobs) = fixture();
        // A non-empty *directory* exactly where the object would be: `metadata`
        // succeeds (so the stat path is not what fails), `remove_file` cannot.
        let occupied = temp.path().join("data").join("dirhash.bin");
        std::fs::create_dir(&occupied).unwrap();
        std::fs::write(occupied.join("child"), b"x").unwrap();
        store
            .record_native_blob("dirhash", "data/dirhash.bin")
            .unwrap();

        let options = GcOptions {
            delete: true,
            ..GcOptions::default()
        };
        let report = run(&store, &blobs, &options, now_unix() + 2 * DAY)
            .await
            .unwrap();

        assert!(report.failed());
        assert_eq!(report.errors.len(), 1, "{:?}", report.errors);
        assert!(
            report.errors[0].starts_with("failed to delete blob dirhash:"),
            "{:?}",
            report.errors
        );
        assert_eq!(report.deleted_count, 0);
        assert_eq!(report.deleted_bytes, 0);
        assert_eq!(report.unreachable.len(), 1);
        assert!(!report.unreachable[0].deleted);
        // Row and bytes both survive, so the next run tries again.
        assert_eq!(
            store.blob_kind("dirhash").unwrap().as_deref(),
            Some("native")
        );
        assert!(occupied.join("child").exists());
    }

    /// The DB refusing the metadata delete *after* the object is gone is
    /// §4.4's deliberately-chosen failure. It is reported rather than
    /// swallowed, is not counted as a delete, and the surviving row makes the
    /// next run finish the job.
    ///
    /// The refusal is staged with a trigger because the real cause —
    /// `package_version_blobs`' foreign key firing on a publish that lands
    /// between the reachability re-check and the row delete — is a race with
    /// no deterministic handle.
    #[tokio::test]
    async fn a_failed_row_delete_is_reported_and_the_next_run_self_heals() {
        let (temp, store, blobs) = fixture();
        store
            .record_native_blob("wedgedhash", "data/wedgedhash.bin")
            .unwrap();
        put_object(&blobs, "wedgedhash", BlobKind::Native, b"seven!!").await;

        let conn = raw_conn(&temp);
        conn.execute_batch(
            "CREATE TRIGGER gc_test_wedge BEFORE DELETE ON package_blobs
             WHEN OLD.hash = 'wedgedhash'
             BEGIN SELECT RAISE(ABORT, 'blob is still referenced'); END;",
        )
        .unwrap();

        let options = GcOptions {
            delete: true,
            ..GcOptions::default()
        };
        let report = run(&store, &blobs, &options, now_unix() + 2 * DAY)
            .await
            .unwrap();

        assert!(report.failed());
        assert_eq!(report.errors.len(), 1, "{:?}", report.errors);
        assert!(
            report.errors[0].contains("deleted object for wedgedhash but failed to remove its row"),
            "{:?}",
            report.errors
        );
        assert!(
            report.errors[0].contains("next gc run will re-collect it"),
            "{:?}",
            report.errors
        );
        assert_eq!(
            report.deleted_count, 0,
            "a half-finished delete is not a delete"
        );
        assert_eq!(report.deleted_bytes, 0);
        assert!(!report.unreachable[0].deleted);
        // Object gone, row still present: exactly the state §4.4 picks.
        assert!(!blobs.exists("wedgedhash", BlobKind::Native).await.unwrap());
        assert!(store.blob_kind("wedgedhash").unwrap().is_some());

        // Once the DB stops refusing, a rerun completes it with no error.
        conn.execute_batch("DROP TRIGGER gc_test_wedge;").unwrap();
        let second = run(&store, &blobs, &options, now_unix() + 2 * DAY)
            .await
            .unwrap();
        assert!(!second.failed(), "{:?}", second.errors);
        assert_eq!(second.deleted_count, 1);
        assert_eq!(
            second.deleted_bytes, 0,
            "the object was already gone, so nothing new was freed"
        );
        assert!(store.blob_kind("wedgedhash").unwrap().is_none());
    }

    /// The pre-delete reachability re-check is load-bearing: a blob that the
    /// opening scan listed as garbage, but which a publish referenced *while
    /// the sweep was running*, is skipped rather than deleted (§4.2).
    ///
    /// The mid-sweep publish is staged as an `AFTER DELETE` trigger on the
    /// preceding candidate's row, which is the only deterministic way to land a
    /// new reference between `unreachable_blobs` and `blob_is_reachable`.
    #[tokio::test]
    async fn a_publish_landing_mid_sweep_saves_a_listed_candidate() {
        let (temp, store, blobs) = fixture();
        let owner = owner_id(&store);
        publish(&store, owner, "1.0.0", "livehash", &[]);
        put_object(&blobs, "livehash", BlobKind::Package, b"live package").await;

        // Two candidates, ordered `created_at, hash` — `aaa` is swept first.
        let old = now_unix() - 10 * DAY;
        raw_blob_row(&temp, "aaa", "native", old);
        raw_blob_row(&temp, "bbb", "native", old + 1);
        put_object(&blobs, "aaa", BlobKind::Native, b"first").await;
        put_object(&blobs, "bbb", BlobKind::Native, b"second").await;

        let conn = raw_conn(&temp);
        let version_id: i64 = conn
            .query_row("SELECT id FROM package_versions", [], |row| row.get(0))
            .unwrap();
        conn.execute(
            &format!(
                "CREATE TRIGGER gc_test_racy_publish AFTER DELETE ON package_blobs
                 WHEN OLD.hash = 'aaa'
                 BEGIN
                   INSERT INTO package_version_blobs (package_version_id, hash)
                   VALUES ({version_id}, 'bbb');
                 END;"
            ),
            [],
        )
        .unwrap();

        let options = GcOptions {
            delete: true,
            ..GcOptions::default()
        };
        let report = run(&store, &blobs, &options, now_unix() + 2 * DAY)
            .await
            .unwrap();

        // `aaa` really was garbage and really was collected.
        assert_eq!(report.deleted_count, 1);
        assert_eq!(report.deleted_bytes, 5);
        assert!(!blobs.exists("aaa", BlobKind::Native).await.unwrap());
        assert!(store.blob_kind("aaa").unwrap().is_none());

        // `bbb` became live mid-sweep and survived both halves of the delete.
        assert!(report.failed());
        assert_eq!(report.errors.len(), 1, "{:?}", report.errors);
        assert!(
            report.errors[0]
                .contains("skipped blob bbb: a publish referenced it during this sweep"),
            "{:?}",
            report.errors
        );
        assert!(blobs.exists("bbb", BlobKind::Native).await.unwrap());
        assert!(store.blob_kind("bbb").unwrap().is_some());
        assert!(store.blob_is_reachable("bbb").unwrap());

        let listed: Vec<(&str, bool)> = report
            .unreachable
            .iter()
            .map(|blob| (blob.hash.as_str(), blob.deleted))
            .collect();
        assert_eq!(listed, vec![("aaa", true), ("bbb", false)]);
    }

    /// `--json` totals the reachable side, and every way that accounting can
    /// fail is surfaced rather than folded silently into the total. A live
    /// version whose object is missing is a real integrity problem — a download
    /// that 404s — and gc is the run that notices even though it is not the
    /// thing that repairs it.
    #[tokio::test]
    async fn json_mode_surfaces_every_kind_of_broken_reachable_blob() {
        let (temp, store, blobs) = fixture();
        let owner = owner_id(&store);
        // A referenced blob with a kind this build cannot parse.
        raw_blob_row(&temp, "weirdhash", "quux", now_unix());
        // A referenced blob whose object never reached the backend.
        store
            .record_native_blob("gonehash", "data/gonehash.bin")
            .unwrap();
        // A referenced blob the backend cannot stat at all (ENOTDIR).
        std::fs::write(temp.path().join("data").join("wall"), b"not a directory").unwrap();
        store
            .record_native_blob("wall/blocked", "data/wall/blocked.bin")
            .unwrap();

        publish(
            &store,
            owner,
            "1.0.0",
            "livehash",
            &[
                "weirdhash".to_string(),
                "gonehash".to_string(),
                "wall/blocked".to_string(),
            ],
        );
        put_object(&blobs, "livehash", BlobKind::Package, b"0123456789").await;

        let options = GcOptions {
            json: true,
            ..GcOptions::default()
        };
        let report = run(&store, &blobs, &options, now_unix() + 2 * DAY)
            .await
            .unwrap();

        assert_eq!(report.reachable_count, 4);
        assert!(
            report.unreachable.is_empty(),
            "every blob here is referenced: {:?}",
            report.unreachable
        );
        // Only the one blob that could actually be stat'ed contributes bytes;
        // the broken three are not guessed at.
        assert_eq!(report.reachable_bytes, Some(10));

        assert!(report.failed());
        assert_eq!(report.errors.len(), 3, "{:?}", report.errors);
        let errors = report.errors.join("\n");
        assert!(
            errors.contains("unknown kind on reachable blob weirdhash"),
            "{errors}"
        );
        assert!(
            errors.contains("reachable blob gonehash has no backing object"),
            "{errors}"
        );
        assert!(
            errors.contains("failed to stat reachable blob wall/blocked:"),
            "{errors}"
        );

        // Nothing was collected: this run only reported.
        assert_eq!(report.deleted_count, 0);
        assert!(store.blob_kind("gonehash").unwrap().is_some());

        let json: serde_json::Value = serde_json::from_str(&render_json(&report)).unwrap();
        assert_eq!(json["reachableCount"], 4);
        assert_eq!(json["reachableBytes"], 10);
        assert_eq!(json["errors"].as_array().unwrap().len(), 3);
    }

    /// The text report shapes a single-blob dry run never produces: a plural
    /// count, an already-missing object, the `--json` reachable-bytes line, a
    /// delete summary, and the error tail.
    #[test]
    fn text_report_renders_a_plural_delete_run_with_failures() {
        let report = GcReport {
            grace_hours: 48,
            deleting: true,
            unreachable: vec![
                GcBlob {
                    hash: "aaa".to_string(),
                    kind: "native".to_string(),
                    blob_ref: "/data/aaa.bin".to_string(),
                    size: Some(2048),
                    age_seconds: 3 * DAY,
                    deleted: true,
                },
                GcBlob {
                    hash: "bbb".to_string(),
                    kind: "package".to_string(),
                    blob_ref: "/data/bbb.mfp".to_string(),
                    size: None,
                    age_seconds: 7200,
                    deleted: false,
                },
            ],
            unreachable_bytes: 2048,
            deleted_count: 1,
            deleted_bytes: 2048,
            reachable_count: 3,
            reachable_bytes: Some(1_572_864),
            errors: vec!["failed to delete blob bbb: boom".to_string()],
        };

        let text = render_text(&report);
        assert!(text.contains("grace period 48h (deleting)"), "{text}");
        // An object that is already gone reads as "missing", never as "0 B" —
        // the two mean opposite things to an operator.
        assert!(text.contains("missing"), "{text}");
        assert!(!text.contains("0 B"), "{text}");
        assert!(text.contains("2.0 KiB"), "{text}");
        assert!(text.contains("3d"), "{text}");
        assert!(text.contains("2h"), "{text}");
        assert!(
            text.contains("2 unreachable blobs, 2.0 KiB reclaimable."),
            "{text}"
        );
        assert!(text.contains("Reachable: 3 blobs"), "{text}");
        assert!(text.contains("Reachable bytes: 1.5 MiB"), "{text}");
        assert!(text.contains("Deleted 1 blob, freed 2.0 KiB."), "{text}");
        // A run that already deleted must not invite the operator to run again.
        assert!(!text.contains("--delete"), "{text}");
        assert!(
            text.contains("error: failed to delete blob bbb: boom"),
            "{text}"
        );
    }

    /// The opposite shapes: an empty sweep says so and offers no call to
    /// action, a text-mode report omits the reachable-bytes line it never
    /// computed, and the delete summary pluralises the other way.
    #[test]
    fn text_report_renders_an_empty_sweep_and_a_plural_delete_count() {
        let empty = GcReport {
            grace_hours: 24,
            reachable_count: 1,
            ..GcReport::default()
        };
        let text = render_text(&empty);
        assert!(text.contains("(dry run)"), "{text}");
        assert!(
            text.contains("No unreachable blobs older than 24h."),
            "{text}"
        );
        assert!(
            text.contains("Reachable: 1 blob (never collected"),
            "{text}"
        );
        assert!(
            !text.contains("--delete"),
            "there is nothing to reclaim, so no call to action: {text}"
        );
        assert!(
            !text.contains("Reachable bytes"),
            "text mode does not stat the reachable set: {text}"
        );
        assert!(!text.contains("Deleted"), "{text}");

        let plural = GcReport {
            grace_hours: 24,
            deleting: true,
            deleted_count: 2,
            reachable_count: 0,
            ..GcReport::default()
        };
        let text = render_text(&plural);
        assert!(text.contains("Deleted 2 blobs, freed 0 B."), "{text}");
        assert!(text.contains("Reachable: 0 blobs"), "{text}");
    }

    #[test]
    fn byte_and_age_formatting() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1536), "1.5 KiB");
        assert_eq!(format_bytes(3 * 1024 * 1024), "3.0 MiB");
        assert_eq!(format_age(0), "0m");
        assert_eq!(format_age(90), "1m");
        assert_eq!(format_age(7200), "2h");
        assert_eq!(format_age(3 * DAY), "3d");
        // A blob stamped in the future (clock skew) reads as brand new, not as
        // a negative age.
        assert_eq!(format_age(-10), "0m");
    }
}
