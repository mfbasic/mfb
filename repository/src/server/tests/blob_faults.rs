//! The blob routes (`server/blobs.rs`) when the index and the blob store
//! disagree, the index cannot be read, or the upload's own index write fails:
//! a row with no bytes is a 404 (never an empty body), an unreadable or unknown
//! row is a 500, and a refused upload leaves no servable or staged bytes.

use super::*;
use crate::store::tests::{drop_tables, refuse_writes, tamper};

const BODY: &[u8] = b"\x7fELF vendored native library payload";

fn blob_hash() -> String {
    hex::encode(crypto::sha256(BODY))
}

async fn head(h: &Harness, hash: &str) -> StatusCode {
    head_blob(
        State(h.state.clone()),
        axum::extract::Path(hash.to_string()),
    )
    .await
}

async fn put(h: &Harness, token: &str) -> Result<StatusCode, (StatusCode, String)> {
    put_blob(
        State(h.state.clone()),
        axum::extract::Path(blob_hash()),
        bearer_headers(token),
        axum::body::Bytes::from(BODY.to_vec()),
    )
    .await
    .map_err(|(status, Json(body))| (status, body.error))
}

#[tokio::test]
async fn an_indexed_blob_missing_from_the_store_is_not_found() {
    let h = harness();
    let hash = blob_hash();
    h.store
        .record_native_blob(&hash, &h.state.blob_store.blob_ref(&hash, BlobKind::Native))
        .unwrap();
    assert_eq!(head(&h, &hash).await, StatusCode::NOT_FOUND);
    let (status, message) =
        err_of(package_blob(State(h.state.clone()), axum::extract::Path(hash.clone())).await);
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(message, "no blob with that hash");
}

#[tokio::test]
async fn the_dedup_probe_is_a_server_error_when_the_index_cannot_answer() {
    let h = harness();
    let hash = blob_hash();
    h.store.record_native_blob(&hash, "data/blob").unwrap();
    // A kind this server does not know how to serve.
    tamper(&h.store, "UPDATE package_blobs SET kind = 'sideloaded'");
    assert_eq!(head(&h, &hash).await, StatusCode::INTERNAL_SERVER_ERROR);
    // No index at all.
    drop_tables(&h.store, &["package_version_blobs", "package_blobs"]);
    assert_eq!(head(&h, &hash).await, StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn an_upload_on_a_session_whose_key_is_no_longer_current_is_refused() {
    let h = harness();
    let alice = register_owner_with_all_keys(&h.store, "alice");
    let token = open_session(&h.store, "alice", &alice.auth_private);
    // The session row is still live, but its key was retired underneath it.
    tamper(
        &h.store,
        "UPDATE keys SET status = 'past' WHERE role = 'auth'",
    );
    assert_eq!(
        put(&h, &token).await.unwrap_err(),
        (
            StatusCode::UNAUTHORIZED,
            "session key is not a current auth key".to_string()
        )
    );
}

#[tokio::test]
async fn an_upload_whose_index_row_cannot_be_written_leaves_no_bytes_behind() {
    let h = harness();
    let alice = register_owner_with_all_keys(&h.store, "alice");
    let token = open_session(&h.store, "alice", &alice.auth_private);
    refuse_writes(&h.store, "package_blobs", "INSERT");
    assert_eq!(
        put(&h, &token).await.unwrap_err(),
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to store native blob metadata: injected INSERT failure".to_string()
        )
    );
    // Neither the blob nor its staging file survives the refusal.
    let left: Vec<_> = std::fs::read_dir(&h.packages_dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert!(left.is_empty(), "{left:?}");
    assert_eq!(head(&h, &blob_hash()).await, StatusCode::NOT_FOUND);
}

/// A served blob is the exact bytes, as an opaque immutable download: it is
/// content-addressed, so any cache may keep it forever.
#[tokio::test]
async fn a_stored_blob_is_served_as_immutable_octets() {
    let h = harness();
    let alice = register_owner_with_all_keys(&h.store, "alice");
    let token = open_session(&h.store, "alice", &alice.auth_private);
    assert_eq!(put(&h, &token).await.unwrap(), StatusCode::CREATED);
    let response = package_blob(State(h.state.clone()), axum::extract::Path(blob_hash()))
        .await
        .expect("blob served");
    assert_eq!(response.status(), StatusCode::OK);
    let headers = response.headers().clone();
    assert_eq!(
        headers
            .get_all(axum::http::header::CONTENT_TYPE)
            .iter()
            .collect::<Vec<_>>(),
        vec!["application/octet-stream"]
    );
    assert_eq!(
        headers
            .get_all(axum::http::header::CACHE_CONTROL)
            .iter()
            .collect::<Vec<_>>(),
        vec!["public, max-age=31536000, immutable"]
    );
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(body.as_ref(), BODY);
}
