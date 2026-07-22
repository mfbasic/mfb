//! Pluggable blob storage for content-addressed `.mfp` package artifacts.
//!
//! The repository serves immutable package blobs keyed purely by their SHA-256
//! hash as `<hash>.mfp`. Two backends share one interface:
//!
//!   * [`BlobStore::Local`] — files under a data directory (the historical
//!     default; `--datapath /some/dir`).
//!   * `BlobStore::S3` — objects in an S3 or S3-compatible bucket
//!     (`--datapath s3://<bucket>/<prefix>`). Downloads are served as a `302`
//!     redirect to a short-lived presigned URL, so blob bytes never transit the
//!     app server and the bucket can stay private. Publishing PUTs directly to
//!     the content-addressed key. Gated behind the `s3` cargo feature so the
//!     heavy AWS SDK is only compiled into `mfb-repo` builds that need it.
//!
//! ### Publish protocol (plan-10-A §2.6)
//! Publishing is three-phase — `stage` → commit the DB version row →
//! `promote` (with `abort` on DB failure) — so a failed publish never leaves a
//! servable orphan. The local backend stages to a temp file and promotes with
//! an atomic rename. The S3 backend PUTs the immutable content-addressed object
//! directly and treats `promote` as a no-op; `abort` deletes the object. This
//! is safe because the object key is the content hash: it is unknowable (and so
//! unreachable) until the committed index row exposes it, and a failed publish
//! cleans it up.

use std::path::PathBuf;
use uuid::Uuid;

/// How a blob download should be fulfilled.
pub enum BlobFetch {
    /// Serve these bytes inline (local backend). The caller re-verifies the
    /// hash as a blob-store corruption defense before sending them.
    Bytes(Vec<u8>),
    /// Redirect the client to this presigned URL (S3 backend). The client
    /// re-hashes the downloaded bytes, so the server need not proxy them.
    Redirect(String),
}

/// A staged-but-not-yet-servable blob returned by [`BlobStore::stage`] and
/// consumed by [`BlobStore::promote`] or [`BlobStore::abort`].
pub enum StagedBlob {
    Local {
        temp: PathBuf,
        final_path: PathBuf,
    },
    #[cfg(feature = "s3")]
    S3 {
        key: String,
    },
}

/// Parsed `--datapath` backend selection, produced before any async setup so
/// argument validation stays synchronous and unit-testable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlobBackend {
    Local(PathBuf),
    S3 {
        bucket: String,
        /// Key prefix, normalized to end with `/` (or empty for bucket root).
        prefix: String,
        /// Optional custom endpoint for S3-compatible stores (MinIO, R2, Ceph).
        endpoint: Option<String>,
    },
}

impl BlobBackend {
    /// Parse `--datapath` (and the optional `--s3-endpoint`) into a backend.
    ///
    /// A bare path selects [`BlobBackend::Local`]. An `s3://<bucket>/<prefix>`
    /// URL selects `BlobBackend::S3`. `--s3-endpoint` is only meaningful in S3
    /// mode: supplying it with a local `--datapath` is a hard error (they go
    /// together). A real-AWS bucket needs no endpoint — the region is resolved
    /// from the environment.
    pub fn parse(datapath: &str, endpoint: Option<String>) -> Result<Self, String> {
        if let Some(rest) = datapath.strip_prefix("s3://") {
            let (bucket, prefix) = match rest.split_once('/') {
                Some((bucket, prefix)) => (bucket, prefix),
                None => (rest, ""),
            };
            if bucket.is_empty() {
                return Err(
                    "s3 --datapath must name a bucket, e.g. s3://my-bucket/prefix".to_string(),
                );
            }
            let prefix = normalize_prefix(prefix);
            Ok(BlobBackend::S3 {
                bucket: bucket.to_string(),
                prefix,
                endpoint,
            })
        } else if endpoint.is_some() {
            Err("--s3-endpoint requires an s3:// --datapath".to_string())
        } else {
            Ok(BlobBackend::Local(PathBuf::from(datapath)))
        }
    }

    /// Build the live [`BlobStore`], performing any async backend setup. For
    /// the local backend this creates the data directory; for S3 it constructs
    /// the SDK client from the ambient AWS credential/region chain.
    pub async fn into_store(self) -> Result<BlobStore, String> {
        match self {
            BlobBackend::Local(dir) => {
                if dir.exists() && !dir.is_dir() {
                    return Err(format!(
                        "data path '{}' exists but is not a directory",
                        dir.display()
                    ));
                }
                std::fs::create_dir_all(&dir).map_err(|err| {
                    format!("failed to create data directory '{}': {err}", dir.display())
                })?;
                Ok(BlobStore::Local(LocalBlobStore { dir }))
            }
            #[cfg(feature = "s3")]
            BlobBackend::S3 {
                bucket,
                prefix,
                endpoint,
            } => s3_impl::build(bucket, prefix, endpoint).await,
            #[cfg(not(feature = "s3"))]
            BlobBackend::S3 { .. } => Err(
                "this mfb-repo was built without S3 support; rebuild with `--features s3` \
                 to use an s3:// --datapath"
                    .to_string(),
            ),
        }
    }
}

/// Normalize a key prefix to end with `/`, or be empty for the bucket root.
fn normalize_prefix(prefix: &str) -> String {
    let trimmed = prefix.trim_matches('/');
    if trimmed.is_empty() {
        String::new()
    } else {
        format!("{trimmed}/")
    }
}

/// What a stored blob *is* — a package `.mfp` artifact or a vendored native
/// library file (plan-48-A §4.1). The kind selects the on-disk/S3 filename
/// suffix so an operator listing the datapath sees honest names, and it is
/// persisted in `package_blobs.kind` so `GET /blob/<hash>` can learn a blob's
/// kind from a primary-key lookup before touching the backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlobKind {
    Package,
    Native,
}

impl BlobKind {
    /// The content-addressed filename suffix for this kind. `Package` keeps the
    /// historical `.mfp` (so existing blobs need no migration); native library
    /// blobs use `.bin`.
    fn suffix(self) -> &'static str {
        match self {
            BlobKind::Package => "mfp",
            BlobKind::Native => "bin",
        }
    }

    /// The `package_blobs.kind` column value.
    pub fn db_str(self) -> &'static str {
        match self {
            BlobKind::Package => "package",
            BlobKind::Native => "native",
        }
    }

    /// Parse a `package_blobs.kind` column value back into a `BlobKind`. An
    /// unknown value (never written by this code) is rejected rather than
    /// silently coerced.
    pub fn from_db_str(value: &str) -> Result<Self, String> {
        match value {
            "package" => Ok(BlobKind::Package),
            "native" => Ok(BlobKind::Native),
            other => Err(format!("unknown blob kind '{other}'")),
        }
    }
}

/// The content-addressed object/file name for a blob hash and kind.
fn blob_name(hash: &str, kind: BlobKind) -> String {
    format!("{hash}.{}", kind.suffix())
}

/// Live blob storage backend. Cheap to clone (the S3 client is internally
/// reference-counted), so it can live in the shared `AppState`.
#[derive(Clone)]
pub enum BlobStore {
    Local(LocalBlobStore),
    #[cfg(feature = "s3")]
    S3(s3_impl::S3BlobStore),
}

#[derive(Clone)]
pub struct LocalBlobStore {
    dir: PathBuf,
}

impl BlobStore {
    /// Construct a local backend rooted at `dir` (used by tests and callers
    /// that already hold a directory path).
    pub fn local(dir: impl Into<PathBuf>) -> Self {
        BlobStore::Local(LocalBlobStore { dir: dir.into() })
    }

    /// A human-readable reference to where the blob lives, recorded in the
    /// `package_blobs.path` column. Informational only — serving is by hash.
    pub fn blob_ref(&self, hash: &str, kind: BlobKind) -> String {
        match self {
            BlobStore::Local(local) => local
                .dir
                .join(blob_name(hash, kind))
                .to_string_lossy()
                .into_owned(),
            #[cfg(feature = "s3")]
            BlobStore::S3(s3) => {
                format!("s3://{}/{}{}", s3.bucket, s3.prefix, blob_name(hash, kind))
            }
        }
    }

    /// Whether a servable blob already exists for `hash`.
    pub async fn exists(&self, hash: &str, kind: BlobKind) -> Result<bool, String> {
        match self {
            BlobStore::Local(local) => Ok(local.dir.join(blob_name(hash, kind)).exists()),
            #[cfg(feature = "s3")]
            BlobStore::S3(s3) => s3.exists(hash, kind).await,
        }
    }

    /// Stage `bytes` for `hash`. The staged blob is not yet servable (local
    /// backend) or is written to its final immutable key (S3 backend); either
    /// way it is not committed until [`BlobStore::promote`].
    pub async fn stage(
        &self,
        hash: &str,
        kind: BlobKind,
        bytes: Vec<u8>,
    ) -> Result<StagedBlob, String> {
        match self {
            BlobStore::Local(local) => {
                let final_path = local.dir.join(blob_name(hash, kind));
                let temp =
                    local
                        .dir
                        .join(format!("{}.tmp-{}", blob_name(hash, kind), Uuid::new_v4()));
                std::fs::write(&temp, &bytes)
                    .map_err(|err| format!("failed to stage package blob: {err}"))?;
                Ok(StagedBlob::Local { temp, final_path })
            }
            #[cfg(feature = "s3")]
            BlobStore::S3(s3) => s3.stage(hash, kind, bytes).await,
        }
    }

    /// Commit a staged blob so it becomes servable.
    pub async fn promote(&self, staged: StagedBlob) -> Result<(), String> {
        match (self, staged) {
            (BlobStore::Local(_), StagedBlob::Local { temp, final_path }) => {
                std::fs::rename(&temp, &final_path).map_err(|err| {
                    let _ = std::fs::remove_file(&temp);
                    format!("failed to persist package blob: {err}")
                })
            }
            #[cfg(feature = "s3")]
            (BlobStore::S3(_), StagedBlob::S3 { .. }) => Ok(()),
            // Mismatched backend/staged pair: unreachable in practice (each
            // stage/promote pair shares one store), but fail loudly if it ever
            // happens rather than silently dropping the blob.
            #[allow(unreachable_patterns)]
            _ => Err("blob store backend mismatch on promote".to_string()),
        }
    }

    /// Discard a staged blob after a failed publish, leaving no servable orphan.
    ///
    /// Local staging uses a per-request UUID temp file, so removing it is always
    /// safe and always right.
    ///
    /// S3 is deliberately different: it stages and promotes to the *same*
    /// content-addressed key, so the staged key is not this request's private
    /// object — it is the object. Two concurrent publishes of identical bytes
    /// share it, and the one that loses the unique-constraint race used to
    /// `delete` it on abort, destroying the blob the winner had just committed
    /// (the winner's `promote` is a no-op) and leaving a `package_versions` row
    /// whose blob 404s (bug-276 R4).
    ///
    /// There is no safe narrowing here: S3's PUT does not report whether it
    /// created or replaced an object, and a HEAD-before-PUT is itself racy. So
    /// abort does not delete. The cost is a possible unreferenced object, which
    /// is byte-identical to what a legitimate publish stores and is what the
    /// `package_version_blobs` reachability edges exist to let a GC reclaim. The
    /// cost of the alternative is destroying live data.
    pub async fn abort(&self, staged: StagedBlob) {
        match staged {
            StagedBlob::Local { temp, .. } => {
                let _ = std::fs::remove_file(&temp);
            }
            #[cfg(feature = "s3")]
            StagedBlob::S3 { key } => {
                let _ = key;
            }
        }
    }

    /// The stored size of a blob in bytes, or `None` when no object exists.
    ///
    /// `package_blobs` has no size column (plan-49 §4.3): rather than add one
    /// and back-fill every pre-existing row as unknown, the collector stats the
    /// backing store on demand. That costs one `metadata`/`head_object` per
    /// candidate, which is fine for a command an operator runs rarely and buys
    /// a report of *real* reclaimed bytes.
    pub async fn size(&self, hash: &str, kind: BlobKind) -> Result<Option<u64>, String> {
        match self {
            BlobStore::Local(local) => {
                match std::fs::metadata(local.dir.join(blob_name(hash, kind))) {
                    Ok(meta) => Ok(Some(meta.len())),
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
                    Err(err) => Err(format!("failed to stat package blob: {err}")),
                }
            }
            #[cfg(feature = "s3")]
            BlobStore::S3(s3) => s3.size(hash, kind).await,
        }
    }

    /// Delete a blob's backing object. **Only the garbage collector calls this**
    /// — the publish path's failure cleanup is [`BlobStore::abort`], which is
    /// deliberately narrower (see its docs on why S3 must not delete there).
    ///
    /// An already-absent object is success, not an error: plan-49 §4.4 deletes
    /// the object *before* the DB row precisely so that a crash in between
    /// leaves a row the next `gc` re-lists, and that re-collection must be
    /// idempotent.
    pub async fn delete(&self, hash: &str, kind: BlobKind) -> Result<(), String> {
        match self {
            BlobStore::Local(local) => {
                match std::fs::remove_file(local.dir.join(blob_name(hash, kind))) {
                    Ok(()) => Ok(()),
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    Err(err) => Err(format!("failed to delete package blob: {err}")),
                }
            }
            #[cfg(feature = "s3")]
            BlobStore::S3(s3) => s3.delete(hash, kind).await,
        }
    }

    /// Fetch a blob for download. Returns `None` when no blob exists for `hash`.
    pub async fn get(&self, hash: &str, kind: BlobKind) -> Result<Option<BlobFetch>, String> {
        match self {
            BlobStore::Local(local) => {
                let path = local.dir.join(blob_name(hash, kind));
                match std::fs::read(&path) {
                    Ok(bytes) => Ok(Some(BlobFetch::Bytes(bytes))),
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
                    Err(err) => Err(format!("failed to read package blob: {err}")),
                }
            }
            #[cfg(feature = "s3")]
            BlobStore::S3(s3) => s3.get(hash, kind).await,
        }
    }
}

#[cfg(feature = "s3")]
mod s3_impl {
    use super::{blob_name, BlobFetch, BlobKind, BlobStore, StagedBlob};
    use aws_config::BehaviorVersion;
    use aws_sdk_s3::config::Region;
    use aws_sdk_s3::presigning::PresigningConfig;
    use aws_sdk_s3::primitives::ByteStream;
    use std::time::Duration;

    /// Presigned-download URLs live only long enough for the client to follow
    /// the redirect; the app remains the access-control gate that issues them.
    const PRESIGN_TTL: Duration = Duration::from_secs(300);

    #[derive(Clone)]
    pub struct S3BlobStore {
        pub(super) client: aws_sdk_s3::Client,
        pub(super) bucket: String,
        pub(super) prefix: String,
    }

    /// Build the S3 client from the ambient AWS credential/region chain,
    /// overriding the endpoint (and switching to path-style addressing) for
    /// S3-compatible stores such as MinIO, Cloudflare R2, and Ceph.
    pub(super) async fn build(
        bucket: String,
        prefix: String,
        endpoint: Option<String>,
    ) -> Result<BlobStore, String> {
        // A concrete region is required for request signing; real AWS resolves
        // it from the environment, and S3-compatible endpoints accept the
        // conventional `us-east-1` placeholder when none is configured.
        let region = aws_config::meta::region::RegionProviderChain::default_provider()
            .or_else(Region::new("us-east-1"));
        let shared = aws_config::defaults(BehaviorVersion::latest())
            .region(region)
            .load()
            .await;
        let mut builder = aws_sdk_s3::config::Builder::from(&shared);
        if let Some(endpoint) = endpoint {
            builder = builder.endpoint_url(endpoint).force_path_style(true);
        }
        let client = aws_sdk_s3::Client::from_conf(builder.build());
        Ok(BlobStore::S3(S3BlobStore {
            client,
            bucket,
            prefix,
        }))
    }

    impl S3BlobStore {
        fn key(&self, hash: &str, kind: BlobKind) -> String {
            format!("{}{}", self.prefix, blob_name(hash, kind))
        }

        pub(super) async fn exists(&self, hash: &str, kind: BlobKind) -> Result<bool, String> {
            match self
                .client
                .head_object()
                .bucket(&self.bucket)
                .key(self.key(hash, kind))
                .send()
                .await
            {
                Ok(_) => Ok(true),
                Err(err) => {
                    if err
                        .as_service_error()
                        .map(|e| e.is_not_found())
                        .unwrap_or(false)
                    {
                        Ok(false)
                    } else {
                        Err(format!(
                            "failed to query S3 blob: {}",
                            service_message(&err)
                        ))
                    }
                }
            }
        }

        pub(super) async fn stage(
            &self,
            hash: &str,
            kind: BlobKind,
            bytes: Vec<u8>,
        ) -> Result<StagedBlob, String> {
            let key = self.key(hash, kind);
            self.client
                .put_object()
                .bucket(&self.bucket)
                .key(&key)
                .body(ByteStream::from(bytes))
                .content_type("application/octet-stream")
                .send()
                .await
                .map_err(|err| {
                    format!("failed to upload package blob: {}", service_message(&err))
                })?;
            Ok(StagedBlob::S3 { key })
        }

        pub(super) async fn size(&self, hash: &str, kind: BlobKind) -> Result<Option<u64>, String> {
            match self
                .client
                .head_object()
                .bucket(&self.bucket)
                .key(self.key(hash, kind))
                .send()
                .await
            {
                // A missing `content_length` is not a missing object; report it
                // as zero rather than silently dropping the candidate.
                Ok(head) => Ok(Some(head.content_length().unwrap_or(0).max(0) as u64)),
                Err(err) => {
                    if err
                        .as_service_error()
                        .map(|e| e.is_not_found())
                        .unwrap_or(false)
                    {
                        Ok(None)
                    } else {
                        Err(format!("failed to stat S3 blob: {}", service_message(&err)))
                    }
                }
            }
        }

        /// Delete one blob object (plan-49 §4.4). S3 `DeleteObject` is already
        /// idempotent — deleting an absent key succeeds — which is exactly the
        /// "not found is success" contract the collector needs.
        pub(super) async fn delete(&self, hash: &str, kind: BlobKind) -> Result<(), String> {
            self.client
                .delete_object()
                .bucket(&self.bucket)
                .key(self.key(hash, kind))
                .send()
                .await
                .map_err(|err| format!("failed to delete S3 blob: {}", service_message(&err)))?;
            Ok(())
        }

        pub(super) async fn get(
            &self,
            hash: &str,
            kind: BlobKind,
        ) -> Result<Option<BlobFetch>, String> {
            // HEAD first so a missing blob yields our own 404 rather than
            // redirecting the client to an S3 error page. Presigning itself is a
            // local signing operation with no network round trip.
            if !self.exists(hash, kind).await? {
                return Ok(None);
            }
            let presigned = self
                .client
                .get_object()
                .bucket(&self.bucket)
                .key(self.key(hash, kind))
                .presigned(
                    PresigningConfig::expires_in(PRESIGN_TTL)
                        .map_err(|err| format!("failed to configure presigned URL: {err}"))?,
                )
                .await
                .map_err(|err| {
                    format!("failed to presign blob download: {}", service_message(&err))
                })?;
            Ok(Some(BlobFetch::Redirect(presigned.uri().to_string())))
        }
    }

    /// Extract the most useful message from an SDK error without leaking the
    /// full debug chain into HTTP responses.
    fn service_message<E, R>(err: &aws_sdk_s3::error::SdkError<E, R>) -> String
    where
        E: std::error::Error,
    {
        match err.as_service_error() {
            Some(service) => service.to_string(),
            None => err.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
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
        let err = BlobBackend::parse("/local/dir", Some("https://minio.example".to_string()))
            .unwrap_err();
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
        // Package blobs keep the historical `.mfp` name — byte-for-byte
        // unchanged, so existing blobs need no migration.
        assert_eq!(
            store.blob_ref("abc123", BlobKind::Package),
            "/data/abc123.mfp"
        );
        assert_eq!(blob_name("abc123", BlobKind::Package), "abc123.mfp");
        // Native library blobs land in a new `.bin` namespace.
        assert_eq!(
            store.blob_ref("abc123", BlobKind::Native),
            "/data/abc123.bin"
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
        // Rooting the store at a regular file makes `<file>/<hash>.mfp` stat
        // with ENOTDIR, which is not `NotFound` and so must be reported as an
        // error rather than being mistaken for "no such blob".
        let (_guard, root) = not_a_directory_root();
        let store = BlobStore::local(root);
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
        let (_guard, root) = not_a_directory_root();
        let store = BlobStore::local(root);
        let err = expect_err(store.delete(&"a".repeat(64), BlobKind::Package).await);
        assert!(
            err.contains("failed to delete package blob"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn get_surfaces_non_not_found_read_errors() {
        let (_guard, root) = not_a_directory_root();
        let store = BlobStore::local(root);
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

    /// A store root that is a regular file, so every `<root>/<name>` path
    /// operation fails with `ENOTDIR` — an io error that is *not* `NotFound`,
    /// which is what distinguishes the "real failure" arms of `size`/`delete`/
    /// `get` from their "blob is absent" arms.
    /// The returned `TempDir` guard must be held for the duration of the test.
    fn not_a_directory_root() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("root-is-a-file");
        std::fs::write(&file, b"x").unwrap();
        (dir, file)
    }
}
