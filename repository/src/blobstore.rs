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

/// The content-addressed object/file name for a blob hash.
fn blob_name(hash: &str) -> String {
    format!("{hash}.mfp")
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
    pub fn blob_ref(&self, hash: &str) -> String {
        match self {
            BlobStore::Local(local) => local
                .dir
                .join(blob_name(hash))
                .to_string_lossy()
                .into_owned(),
            #[cfg(feature = "s3")]
            BlobStore::S3(s3) => format!("s3://{}/{}{}", s3.bucket, s3.prefix, blob_name(hash)),
        }
    }

    /// Whether a servable blob already exists for `hash`.
    pub async fn exists(&self, hash: &str) -> Result<bool, String> {
        match self {
            BlobStore::Local(local) => Ok(local.dir.join(blob_name(hash)).exists()),
            #[cfg(feature = "s3")]
            BlobStore::S3(s3) => s3.exists(hash).await,
        }
    }

    /// Stage `bytes` for `hash`. The staged blob is not yet servable (local
    /// backend) or is written to its final immutable key (S3 backend); either
    /// way it is not committed until [`BlobStore::promote`].
    pub async fn stage(&self, hash: &str, bytes: Vec<u8>) -> Result<StagedBlob, String> {
        match self {
            BlobStore::Local(local) => {
                let final_path = local.dir.join(blob_name(hash));
                let temp = local
                    .dir
                    .join(format!("{}.tmp-{}", blob_name(hash), Uuid::new_v4()));
                std::fs::write(&temp, &bytes)
                    .map_err(|err| format!("failed to stage package blob: {err}"))?;
                Ok(StagedBlob::Local { temp, final_path })
            }
            #[cfg(feature = "s3")]
            BlobStore::S3(s3) => s3.stage(hash, bytes).await,
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
    pub async fn abort(&self, staged: StagedBlob) {
        match staged {
            StagedBlob::Local { temp, .. } => {
                let _ = std::fs::remove_file(&temp);
            }
            #[cfg(feature = "s3")]
            StagedBlob::S3 { key } => {
                if let BlobStore::S3(s3) = self {
                    s3.delete(&key).await;
                }
            }
        }
    }

    /// Fetch a blob for download. Returns `None` when no blob exists for `hash`.
    pub async fn get(&self, hash: &str) -> Result<Option<BlobFetch>, String> {
        match self {
            BlobStore::Local(local) => {
                let path = local.dir.join(blob_name(hash));
                match std::fs::read(&path) {
                    Ok(bytes) => Ok(Some(BlobFetch::Bytes(bytes))),
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
                    Err(err) => Err(format!("failed to read package blob: {err}")),
                }
            }
            #[cfg(feature = "s3")]
            BlobStore::S3(s3) => s3.get(hash).await,
        }
    }
}

#[cfg(feature = "s3")]
mod s3_impl {
    use super::{blob_name, BlobFetch, BlobStore, StagedBlob};
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
        fn key(&self, hash: &str) -> String {
            format!("{}{}", self.prefix, blob_name(hash))
        }

        pub(super) async fn exists(&self, hash: &str) -> Result<bool, String> {
            match self
                .client
                .head_object()
                .bucket(&self.bucket)
                .key(self.key(hash))
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

        pub(super) async fn stage(&self, hash: &str, bytes: Vec<u8>) -> Result<StagedBlob, String> {
            let key = self.key(hash);
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

        pub(super) async fn delete(&self, key: &str) {
            let _ = self
                .client
                .delete_object()
                .bucket(&self.bucket)
                .key(key)
                .send()
                .await;
        }

        pub(super) async fn get(&self, hash: &str) -> Result<Option<BlobFetch>, String> {
            // HEAD first so a missing blob yields our own 404 rather than
            // redirecting the client to an S3 error page. Presigning itself is a
            // local signing operation with no network round trip.
            if !self.exists(hash).await? {
                return Ok(None);
            }
            let presigned = self
                .client
                .get_object()
                .bucket(&self.bucket)
                .key(self.key(hash))
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
        assert_eq!(store.blob_ref("abc123"), "/data/abc123.mfp");
        assert_eq!(blob_name("abc123"), "abc123.mfp");
    }

    #[tokio::test]
    async fn local_stage_promote_get_roundtrip() {
        let temp = tempfile::tempdir().unwrap();
        let backend = BlobBackend::Local(temp.path().join("data"));
        let store = backend.into_store().await.unwrap();
        let hash = "d".repeat(64);

        assert!(!store.exists(&hash).await.unwrap());
        assert!(store.get(&hash).await.unwrap().is_none());

        let staged = store.stage(&hash, b"payload".to_vec()).await.unwrap();
        // Not servable until promoted.
        assert!(!store.exists(&hash).await.unwrap());
        store.promote(staged).await.unwrap();

        assert!(store.exists(&hash).await.unwrap());
        match store.get(&hash).await.unwrap() {
            Some(BlobFetch::Bytes(bytes)) => assert_eq!(bytes, b"payload"),
            Some(BlobFetch::Redirect(_)) => panic!("local backend must serve inline bytes"),
            None => panic!("promoted blob should be servable"),
        }
    }

    #[tokio::test]
    async fn local_abort_removes_staged_blob() {
        let temp = tempfile::tempdir().unwrap();
        let store = BlobStore::local(temp.path());
        let hash = "e".repeat(64);
        let staged = store.stage(&hash, b"payload".to_vec()).await.unwrap();
        store.abort(staged).await;
        assert!(!store.exists(&hash).await.unwrap());
        // The temp file is gone too — no orphan left behind.
        let leftovers: Vec<_> = std::fs::read_dir(temp.path())
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        assert!(leftovers.is_empty(), "staged temp file should be removed");
    }
}
