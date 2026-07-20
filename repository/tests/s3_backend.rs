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

use mfb_repository::blobstore::{BlobBackend, BlobFetch, BlobKind, BlobStore};
use mfb_repository::gc::{self, GcOptions};
use mfb_repository::store::{now_unix, Store};

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

/// Build the live S3 store, or `None` when the environment is not configured.
async fn s3_store() -> Option<BlobStore> {
    let datapath = match std::env::var("MFB_TEST_S3_DATAPATH") {
        Ok(value) => value,
        Err(_) => {
            eprintln!("skipping: MFB_TEST_S3_DATAPATH not set");
            return None;
        }
    };
    let endpoint = std::env::var("MFB_TEST_S3_ENDPOINT").ok();
    Some(
        BlobBackend::parse(&datapath, endpoint)
            .expect("parse s3 datapath")
            .into_store()
            .await
            .expect("build s3 store"),
    )
}

#[tokio::test]
#[ignore = "needs a live S3-compatible endpoint (set MFB_TEST_S3_DATAPATH/ENDPOINT)"]
async fn s3_stage_promote_get_and_abort_roundtrip() {
    let Some(store) = s3_store().await else {
        return;
    };

    // Content-addressed payload keyed by its own hash.
    let payload = b"integration-test package blob payload".to_vec();
    let hash = sha256_hex(&payload);

    // Fresh hash: absent before we stage it.
    assert!(
        !store.exists(&hash, BlobKind::Package).await.unwrap(),
        "blob should not exist yet"
    );
    assert!(
        store.get(&hash, BlobKind::Package).await.unwrap().is_none(),
        "get should be 404-equivalent"
    );

    // stage -> promote makes it servable.
    let staged = store
        .stage(&hash, BlobKind::Package, payload.clone())
        .await
        .expect("stage");
    store.promote(staged).await.expect("promote");
    assert!(
        store.exists(&hash, BlobKind::Package).await.unwrap(),
        "blob should exist after promote"
    );
    assert_eq!(
        store.size(&hash, BlobKind::Package).await.unwrap(),
        Some(payload.len() as u64),
        "head_object reports the stored size (plan-49 §4.3)"
    );

    // get() yields a presigned redirect that actually serves the bytes.
    match store.get(&hash, BlobKind::Package).await.unwrap() {
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

    // S3 `abort` deliberately does NOT delete (bug-276 R4). It stages and
    // promotes to the *same* content-addressed key, so the staged object is not
    // this request's private copy — it is the object. Deleting it destroyed the
    // blob a concurrent publish of identical bytes had just committed. The cost
    // is an object nothing references, which is exactly what `gc` reclaims.
    let payload2 = b"second integration payload for abort".to_vec();
    let hash2 = sha256_hex(&payload2);
    let staged2 = store
        .stage(&hash2, BlobKind::Package, payload2)
        .await
        .expect("stage 2");
    store.abort(staged2).await;
    assert!(
        store.exists(&hash2, BlobKind::Package).await.unwrap(),
        "S3 abort must leave the content-addressed object alone (bug-276 R4)"
    );

    // Explicit deletion is the collector's job, and it is idempotent.
    store.delete(&hash2, BlobKind::Package).await.unwrap();
    assert!(!store.exists(&hash2, BlobKind::Package).await.unwrap());
    store
        .delete(&hash2, BlobKind::Package)
        .await
        .expect("deleting an absent object is success");
    assert_eq!(store.size(&hash2, BlobKind::Package).await.unwrap(), None);

    // Leave the bucket as we found it.
    store.delete(&hash, BlobKind::Package).await.unwrap();
}

/// plan-49 end to end against a real bucket: an orphaned upload is reclaimed
/// while a published package's blobs are untouched.
#[tokio::test]
#[ignore = "needs a live S3-compatible endpoint (set MFB_TEST_S3_DATAPATH/ENDPOINT)"]
async fn s3_gc_reclaims_only_the_orphan() {
    let Some(blobs) = s3_store().await else {
        return;
    };
    // Metadata is always local, even with an s3:// datapath (store.rs:65-121).
    let temp = tempfile::tempdir().unwrap();
    let opened = Store::open_repository(
        &temp.path().join("meta.db"),
        &std::path::PathBuf::from(std::env::var("MFB_TEST_S3_DATAPATH").unwrap()),
    )
    .expect("open store with an s3 datapath");
    let store = opened.store;

    let live = b"s3 gc live package payload".to_vec();
    let live_hash = sha256_hex(&live);
    let orphan = b"s3 gc orphaned upload payload".to_vec();
    let orphan_hash = sha256_hex(&orphan);

    for (hash, kind, bytes) in [
        (&live_hash, BlobKind::Package, live.clone()),
        (&orphan_hash, BlobKind::Native, orphan.clone()),
    ] {
        let staged = blobs.stage(hash, kind, bytes).await.expect("stage");
        blobs.promote(staged).await.expect("promote");
    }
    store
        .record_native_blob(
            &orphan_hash,
            &blobs.blob_ref(&orphan_hash, BlobKind::Native),
        )
        .unwrap();
    // A published version makes `live_hash` reachable; nothing ever names the
    // orphan.
    let owner = register_owner(&store, "alice");
    store
        .publish_package_version(
            owner,
            "alice#toolbox",
            "1.0.0",
            &live_hash,
            &blobs.blob_ref(&live_hash, BlobKind::Package),
            "{}",
            &[],
        )
        .unwrap();

    let options = GcOptions {
        grace_hours: 24,
        delete: true,
        json: true,
    };
    // Two days on, so both rows are outside the grace window.
    let report = gc::run(&store, &blobs, &options, now_unix() + 2 * 86_400)
        .await
        .expect("gc run");

    assert!(!report.failed(), "{:?}", report.errors);
    assert_eq!(report.unreachable.len(), 1, "{:?}", report.unreachable);
    assert_eq!(report.unreachable[0].hash, orphan_hash);
    assert_eq!(report.deleted_bytes, orphan.len() as u64);
    assert_eq!(report.reachable_bytes, Some(live.len() as u64));

    assert!(!blobs.exists(&orphan_hash, BlobKind::Native).await.unwrap());
    assert!(
        blobs.exists(&live_hash, BlobKind::Package).await.unwrap(),
        "a published package's blob must survive the sweep"
    );

    blobs.delete(&live_hash, BlobKind::Package).await.unwrap();
}

/// Register an owner with real proofs and return its row id.
fn register_owner(store: &Store, owner: &str) -> i64 {
    use mfb_repository::crypto;
    let (auth_public, auth_private) = crypto::generate_keypair();
    let (ident_public, ident_private) = crypto::generate_keypair();
    let auth_proof = crypto::sign(
        &auth_private,
        &crypto::registration_message(crypto::ROLE_AUTH, owner, &auth_public),
    )
    .unwrap();
    let ident_proof = crypto::sign(
        &ident_private,
        &crypto::registration_message(crypto::ROLE_IDENT, owner, &ident_public),
    )
    .unwrap();
    store
        .register_owner(
            owner,
            &auth_public,
            &auth_proof,
            &ident_public,
            &ident_proof,
        )
        .unwrap();
    store.owner_with_ident_key(owner).unwrap().unwrap().0.id
}
