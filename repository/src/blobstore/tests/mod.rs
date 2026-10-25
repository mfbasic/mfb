use super::*;

#[test]
fn parse_local_datapath() {
    assert_eq!(
        BlobBackend::parse("/var/lib/mfb/data", None).unwrap(),
        BlobBackend::Local(PathBuf::from("/var/lib/mfb/data"))
    );
}

#[test]
fn parse_s3_datapath_variants_normalize_prefix() {
    assert_eq!(
        BlobBackend::parse("s3://my-bucket", None).unwrap(),
        BlobBackend::S3 {
            bucket: "my-bucket".to_string(),
            prefix: String::new(),
            endpoint: None,
        }
    );
    assert_eq!(
        BlobBackend::parse("s3://my-bucket/", None).unwrap(),
        BlobBackend::S3 {
            bucket: "my-bucket".to_string(),
            prefix: String::new(),
            endpoint: None,
        }
    );
    assert_eq!(
        BlobBackend::parse("s3://my-bucket/packages", None).unwrap(),
        BlobBackend::S3 {
            bucket: "my-bucket".to_string(),
            prefix: "packages/".to_string(),
            endpoint: None,
        }
    );
    assert_eq!(
        BlobBackend::parse("s3://my-bucket/a/b/", None).unwrap(),
        BlobBackend::S3 {
            bucket: "my-bucket".to_string(),
            prefix: "a/b/".to_string(),
            endpoint: None,
        }
    );
}

#[test]
fn parse_s3_carries_endpoint() {
    assert_eq!(
        BlobBackend::parse(
            "s3://bucket/pkgs",
            Some("https://minio.example:9000".to_string())
        )
        .unwrap(),
        BlobBackend::S3 {
            bucket: "bucket".to_string(),
            prefix: "pkgs/".to_string(),
            endpoint: Some("https://minio.example:9000".to_string()),
        }
    );
}

#[test]
fn parse_rejects_endpoint_without_s3() {
    let err =
        BlobBackend::parse("/local/dir", Some("https://minio.example".to_string())).unwrap_err();
    assert!(err.contains("--s3-endpoint requires an s3://"));
}

#[test]
fn parse_rejects_bucketless_s3() {
    assert!(BlobBackend::parse("s3://", None)
        .unwrap_err()
        .contains("must name a bucket"));
    assert!(BlobBackend::parse("s3:///prefix", None)
        .unwrap_err()
        .contains("must name a bucket"));
}

#[test]
fn blob_ref_and_name_are_content_addressed() {
    let store = BlobStore::local("/data");
    // The contract is the content-addressed *name* and that a local
    // `blob_ref` is that name inside the store directory. The separator is
    // NOT part of it: for the local backend `blob_ref` is a host filesystem
    // path (documented in `store.rs` as informational — the collector
    // addresses objects by hash + kind), so it is `\` on Windows, and a
    // Windows deployment rooted at `C:\data` wants exactly that. Asserting a
    // literal `/data/abc123.mfp` failed there on `/data\abc123.mfp`.
    let expected = |name: &str| {
        std::path::Path::new("/data")
            .join(name)
            .to_string_lossy()
            .into_owned()
    };
    // Package blobs keep the historical `.mfp` name — byte-for-byte
    // unchanged, so existing blobs need no migration.
    assert_eq!(
        store.blob_ref("abc123", BlobKind::Package),
        expected("abc123.mfp")
    );
    assert_eq!(blob_name("abc123", BlobKind::Package), "abc123.mfp");
    // Native library blobs land in a new `.bin` namespace.
    assert_eq!(
        store.blob_ref("abc123", BlobKind::Native),
        expected("abc123.bin")
    );
    assert_eq!(blob_name("abc123", BlobKind::Native), "abc123.bin");
}

#[test]
fn blob_kind_db_roundtrip() {
    assert_eq!(BlobKind::Package.db_str(), "package");
    assert_eq!(BlobKind::Native.db_str(), "native");
    assert_eq!(BlobKind::from_db_str("package").unwrap(), BlobKind::Package);
    assert_eq!(BlobKind::from_db_str("native").unwrap(), BlobKind::Native);
    assert!(BlobKind::from_db_str("bogus").is_err());
}

#[tokio::test]
async fn local_stage_promote_get_roundtrip() {
    let temp = tempfile::tempdir().unwrap();
    let backend = BlobBackend::Local(temp.path().join("data"));
    let store = backend.into_store().await.unwrap();
    let hash = "d".repeat(64);

    assert!(!store.exists(&hash, BlobKind::Package).await.unwrap());
    assert!(store.get(&hash, BlobKind::Package).await.unwrap().is_none());

    let staged = store
        .stage(&hash, BlobKind::Package, b"payload".to_vec())
        .await
        .unwrap();
    // Not servable until promoted.
    assert!(!store.exists(&hash, BlobKind::Package).await.unwrap());
    store.promote(staged).await.unwrap();

    assert!(store.exists(&hash, BlobKind::Package).await.unwrap());
    match store.get(&hash, BlobKind::Package).await.unwrap() {
        Some(BlobFetch::Bytes(bytes)) => assert_eq!(bytes, b"payload"),
        #[cfg(feature = "s3")]
        Some(BlobFetch::Redirect(_)) => panic!("local backend must serve inline bytes"),
        None => panic!("promoted blob should be servable"),
    }
}

#[tokio::test]
async fn local_native_blob_roundtrip_uses_bin_suffix() {
    let temp = tempfile::tempdir().unwrap();
    let store = BlobStore::local(temp.path());
    let hash = "f".repeat(64);

    let staged = store
        .stage(&hash, BlobKind::Native, b"\x7fELFnative".to_vec())
        .await
        .unwrap();
    store.promote(staged).await.unwrap();

    assert!(store.exists(&hash, BlobKind::Native).await.unwrap());
    // The `.bin` blob is not visible under the `.mfp` (package) name.
    assert!(!store.exists(&hash, BlobKind::Package).await.unwrap());
    assert!(temp.path().join(format!("{hash}.bin")).exists());
    match store.get(&hash, BlobKind::Native).await.unwrap() {
        Some(BlobFetch::Bytes(bytes)) => assert_eq!(bytes, b"\x7fELFnative"),
        _ => panic!("native blob should serve inline bytes"),
    }
}

#[tokio::test]
async fn local_abort_removes_staged_blob() {
    let temp = tempfile::tempdir().unwrap();
    let store = BlobStore::local(temp.path());
    let hash = "e".repeat(64);
    let staged = store
        .stage(&hash, BlobKind::Package, b"payload".to_vec())
        .await
        .unwrap();
    store.abort(staged).await;
    assert!(!store.exists(&hash, BlobKind::Package).await.unwrap());
    // The temp file is gone too — no orphan left behind.
    let leftovers: Vec<_> = std::fs::read_dir(temp.path())
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    assert!(leftovers.is_empty(), "staged temp file should be removed");
}

#[tokio::test]
async fn into_store_rejects_datapath_that_is_a_file() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("not-a-dir");
    std::fs::write(&file, b"i am a regular file").unwrap();

    let err = expect_err(BlobBackend::Local(file.clone()).into_store().await);
    assert!(
        err.contains("exists but is not a directory"),
        "unexpected error: {err}"
    );
    assert!(
        err.contains(&file.display().to_string()),
        "error should name the offending path: {err}"
    );
    // The pre-existing file must be left exactly as it was, not clobbered
    // into a directory.
    assert_eq!(std::fs::read(&file).unwrap(), b"i am a regular file");
}

#[tokio::test]
async fn into_store_reports_undirectory_creation_failure() {
    // A regular file in the middle of the path makes `create_dir_all` fail
    // with ENOTDIR, which is the only honest way to exercise the
    // "failed to create data directory" arm without faking an OS error.
    let temp = tempfile::tempdir().unwrap();
    let blocker = temp.path().join("blocker");
    std::fs::write(&blocker, b"x").unwrap();
    let dir = blocker.join("data");
    assert!(!dir.exists(), "fixture precondition: target must not exist");

    let err = expect_err(BlobBackend::Local(dir.clone()).into_store().await);
    assert!(
        err.contains("failed to create data directory"),
        "unexpected error: {err}"
    );
    assert!(
        err.contains(&dir.display().to_string()),
        "error should name the directory it could not create: {err}"
    );
}

#[tokio::test]
async fn into_store_creates_missing_local_directory() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("nested/data/dir");
    let store = BlobBackend::Local(dir.clone()).into_store().await.unwrap();
    assert!(dir.is_dir(), "into_store should create the data directory");
    // And the created directory is the one the store actually uses.
    assert_eq!(
        store.blob_ref("abc", BlobKind::Package),
        dir.join("abc.mfp").to_string_lossy()
    );
}

/// Without the `s3` feature an `s3://` datapath must fail at store
/// construction with an actionable message — never silently fall back to a
/// local directory literally named `s3:`.
#[cfg(not(feature = "s3"))]
#[tokio::test]
async fn into_store_rejects_s3_backend_when_feature_is_off() {
    let backend = BlobBackend::parse("s3://my-bucket/pkgs", None).unwrap();
    let err = expect_err(backend.into_store().await);
    assert!(
        err.contains("built without S3 support"),
        "unexpected error: {err}"
    );
    assert!(
        err.contains("--features s3"),
        "error should say how to rebuild: {err}"
    );
}

#[tokio::test]
async fn stage_reports_write_failure() {
    // The store's directory does not exist, so the staging write fails.
    // `stage` must surface that as an error rather than returning a
    // `StagedBlob` that `promote` would later act on.
    let temp = tempfile::tempdir().unwrap();
    let store = BlobStore::local(temp.path().join("missing-dir"));
    let hash = "a".repeat(64);
    let err = expect_err(
        store
            .stage(&hash, BlobKind::Package, b"payload".to_vec())
            .await,
    );
    assert!(
        err.contains("failed to stage package blob"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn promote_reports_rename_failure_and_leaves_nothing_servable() {
    let temp = tempfile::tempdir().unwrap();
    let store = BlobStore::local(temp.path());
    let hash = "b".repeat(64);
    // A staged temp file that no longer exists (e.g. already aborted):
    // renaming it must fail loudly instead of reporting a successful
    // publish for a blob that was never written.
    let staged = StagedBlob::Local {
        temp: temp.path().join("vanished.tmp"),
        final_path: temp.path().join(blob_name(&hash, BlobKind::Package)),
    };
    let err = expect_err(store.promote(staged).await);
    assert!(
        err.contains("failed to persist package blob"),
        "unexpected error: {err}"
    );
    assert!(
        !store.exists(&hash, BlobKind::Package).await.unwrap(),
        "a failed promote must not leave a servable blob"
    );
}

#[tokio::test]
async fn size_reports_missing_absent_and_present_blobs() {
    let temp = tempfile::tempdir().unwrap();
    let store = BlobStore::local(temp.path());
    let hash = "c".repeat(64);

    // Absent blob is `None`, not an error.
    assert_eq!(store.size(&hash, BlobKind::Package).await.unwrap(), None);

    let staged = store
        .stage(&hash, BlobKind::Package, vec![7u8; 1234])
        .await
        .unwrap();
    store.promote(staged).await.unwrap();
    assert_eq!(
        store.size(&hash, BlobKind::Package).await.unwrap(),
        Some(1234)
    );
    // Size is per-kind: the `.bin` sibling still does not exist.
    assert_eq!(store.size(&hash, BlobKind::Native).await.unwrap(), None);
}

#[tokio::test]
async fn size_surfaces_non_not_found_stat_errors() {
    // An unusable root makes `<root>/<hash>.mfp` stat with `InvalidInput`,
    // which is not `NotFound` and so must be reported as an error rather
    // than being mistaken for "no such blob".
    let store = BlobStore::local(unusable_root());
    let err = expect_err(store.size(&"a".repeat(64), BlobKind::Package).await);
    assert!(
        err.contains("failed to stat package blob"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn delete_removes_blob_and_is_idempotent() {
    let temp = tempfile::tempdir().unwrap();
    let store = BlobStore::local(temp.path());
    let hash = "1".repeat(64);
    let staged = store
        .stage(&hash, BlobKind::Package, b"payload".to_vec())
        .await
        .unwrap();
    store.promote(staged).await.unwrap();

    store.delete(&hash, BlobKind::Package).await.unwrap();
    assert!(!store.exists(&hash, BlobKind::Package).await.unwrap());
    // Re-collection of an already-deleted blob is success (plan-49 §4.4).
    store.delete(&hash, BlobKind::Package).await.unwrap();
}

#[tokio::test]
async fn delete_surfaces_non_not_found_errors() {
    let store = BlobStore::local(unusable_root());
    let err = expect_err(store.delete(&"a".repeat(64), BlobKind::Package).await);
    assert!(
        err.contains("failed to delete package blob"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn get_surfaces_non_not_found_read_errors() {
    let store = BlobStore::local(unusable_root());
    let err = expect_err(store.get(&"a".repeat(64), BlobKind::Package).await);
    assert!(
        err.contains("failed to read package blob"),
        "unexpected error: {err}"
    );
}

/// Unwrap the error side of a `Result` whose `Ok` type is not `Debug`
/// (`BlobStore`, `StagedBlob`, and `BlobFetch` deliberately are not).
fn expect_err<T>(result: Result<T, String>) -> String {
    match result {
        Ok(_) => panic!("expected an error, got Ok"),
        Err(err) => err,
    }
}

/// A store root whose path contains an interior NUL, so every
/// `<root>/<name>` operation fails with `InvalidInput` — an io error that is
/// *not* `NotFound`, which is what distinguishes the "real failure" arms of
/// `size`/`delete`/`get` from their "blob is absent" arms.
///
/// This replaces rooting the store at a regular file (POSIX `ENOTDIR`).
/// That is not portable: Windows reports a path whose interior component is
/// a file as `ERROR_PATH_NOT_FOUND`, which Rust maps to `NotFound` — so the
/// three operations took their "absent" arm and returned `Ok`, and the tests
/// failed with "expected an error, got Ok". Measured alternatives that also
/// do not work: a directory standing in for the blob fails `read`/
/// `remove_file` on both platforms but `metadata` *succeeds* on both, and a
/// share-locked file still stats fine on Windows. The interior NUL is
/// rejected before the path reaches the OS at all, so it is identical on
/// every platform and needs no filesystem setup.
fn unusable_root() -> PathBuf {
    PathBuf::from("blob\u{0}root")
}
