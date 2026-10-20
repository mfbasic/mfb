use super::*;
use crate::store::now_unix;

const DAY: i64 = 86_400;

/// A blob hash whose derived object path (`<data>/<hash>.bin`) cannot be
/// stat'd on any platform: the interior NUL is rejected before the path
/// reaches the OS, so `metadata` returns `InvalidInput`. That is neither
/// success nor `NotFound`, which is exactly the third "broken reachable
/// blob" kind — the collector must report it and still list the candidate.
///
/// This replaces planting a regular file where the path needed a directory
/// (POSIX `ENOTDIR`). Windows reports that as `ERROR_PATH_NOT_FOUND` →
/// `NotFound`, so the blob looked merely absent and the stat-failure arm
/// never ran. No portable filesystem shape produces a failing `metadata`:
/// a directory standing in for the blob stats fine on both platforms, as
/// does a share-locked file on Windows.
const UNSTATTABLE_HASH: &str = "unstattable\u{0}blob";

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
            &crate::store::PublishMetadata::default(),
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
    // `_temp` is the tempdir guard: the store and blob backend live inside
    // it, so it must stay bound (not `_`) for the life of the test. The
    // injection below no longer needs its path.
    let (_temp, store, blobs) = fixture();
    // A hash whose derived object path is rejected before it reaches the
    // OS, so `stat`ing it fails with `InvalidInput` — neither success nor
    // "no such object". See `UNSTATTABLE_HASH`.
    store
        .record_native_blob(UNSTATTABLE_HASH, "data/unstattable.bin")
        .unwrap();

    let report = run(&store, &blobs, &GcOptions::default(), now_unix() + 2 * DAY)
        .await
        .unwrap();

    assert!(report.failed());
    assert_eq!(report.errors.len(), 1, "{:?}", report.errors);
    assert!(
        report.errors[0].starts_with(&format!("failed to stat blob {UNSTATTABLE_HASH}:")),
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
    assert!(store.blob_kind(UNSTATTABLE_HASH).unwrap().is_some());
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
        report.errors[0].contains("skipped blob bbb: a publish referenced it during this sweep"),
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
    // A referenced blob the backend cannot stat at all.
    store
        .record_native_blob(UNSTATTABLE_HASH, "data/unstattable.bin")
        .unwrap();

    publish(
        &store,
        owner,
        "1.0.0",
        "livehash",
        &[
            "weirdhash".to_string(),
            "gonehash".to_string(),
            UNSTATTABLE_HASH.to_string(),
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
        errors.contains(&format!(
            "failed to stat reachable blob {UNSTATTABLE_HASH}:"
        )),
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
