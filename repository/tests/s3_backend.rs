//! Live S3-backend integration test. Ignored by default and gated behind the
//! `s3` feature: it needs a reachable S3-compatible endpoint (e.g. MinIO). Run
//! it against a local MinIO with, for example:
//!
//! ```sh
//! docker run -d --name mfb-minio -p 9100:9000 \
//!   -e MINIO_ROOT_USER=testkey -e MINIO_ROOT_PASSWORD=testsecret123 \
//!   minio/minio server /data
//! aws --endpoint-url http://127.0.0.1:9100 s3api create-bucket --bucket mfb-pkgs
//!
//! AWS_ACCESS_KEY_ID=testkey AWS_SECRET_ACCESS_KEY=testsecret123 AWS_REGION=us-east-1 \
//!   MFB_TEST_S3_DATAPATH=s3://mfb-pkgs/it \
//!   MFB_TEST_S3_ENDPOINT=http://127.0.0.1:9100 \
//!   cargo test -p mfb_repository --features s3 --test s3_backend -- --ignored --nocapture
//! ```
#![cfg(feature = "s3")]

use mfb_repository::blobstore::{BlobBackend, BlobFetch};

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

#[tokio::test]
#[ignore = "needs a live S3-compatible endpoint (set MFB_TEST_S3_DATAPATH/ENDPOINT)"]
async fn s3_stage_promote_get_and_abort_roundtrip() {
    let datapath = match std::env::var("MFB_TEST_S3_DATAPATH") {
        Ok(value) => value,
        Err(_) => {
            eprintln!("skipping: MFB_TEST_S3_DATAPATH not set");
            return;
        }
    };
    let endpoint = std::env::var("MFB_TEST_S3_ENDPOINT").ok();

    let store = BlobBackend::parse(&datapath, endpoint)
        .expect("parse s3 datapath")
        .into_store()
        .await
        .expect("build s3 store");

    // Content-addressed payload keyed by its own hash.
    let payload = b"integration-test package blob payload".to_vec();
    let hash = sha256_hex(&payload);

    // Fresh hash: absent before we stage it.
    assert!(!store.exists(&hash).await.unwrap(), "blob should not exist yet");
    assert!(store.get(&hash).await.unwrap().is_none(), "get should be 404-equivalent");

    // stage -> promote makes it servable.
    let staged = store.stage(&hash, payload.clone()).await.expect("stage");
    store.promote(staged).await.expect("promote");
    assert!(store.exists(&hash).await.unwrap(), "blob should exist after promote");

    // get() yields a presigned redirect that actually serves the bytes.
    match store.get(&hash).await.unwrap() {
        Some(BlobFetch::Redirect(url)) => {
            let body = reqwest::get(&url)
                .await
                .expect("follow presigned url")
                .bytes()
                .await
                .expect("read presigned body");
            assert_eq!(body.as_ref(), payload.as_slice(), "presigned bytes match");
            assert_eq!(sha256_hex(&body), hash, "downloaded hash matches");
        }
        other => panic!("expected an S3 redirect, got {:?}", other.is_some()),
    }

    // abort removes a freshly staged (never-promoted-over) blob. Use a second
    // distinct payload so we do not disturb the promoted one above.
    let payload2 = b"second integration payload for abort".to_vec();
    let hash2 = sha256_hex(&payload2);
    let staged2 = store.stage(&hash2, payload2).await.expect("stage 2");
    store.abort(staged2).await;
    assert!(!store.exists(&hash2).await.unwrap(), "aborted blob should be gone");
}
