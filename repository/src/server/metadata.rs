//! plan-176-L: the registry server's metadata code. `AppState`, `serve`, the router and the
//! error responses live in `repository/src/server/mod.rs`.

use super::*;

/// A public key in project-metadata form, `ed25519:<base64url>`.
pub(super) fn metadata_key(public_key: &[u8]) -> String {
    format!("ed25519:{}", crypto::encode_bytes(public_key))
}

/// Snapshot metadata lifetime (plan-10-C2): the snapshot pins the index state
/// for a week; the timestamp refreshes daily and pins the current snapshot.
const SNAPSHOT_TTL_SECS: i64 = 7 * 24 * 3600;

const TIMESTAMP_TTL_SECS: i64 = 24 * 3600;

/// `GET /root.json` (plan-10-C2): the offline-root-signed metadata delegating
/// the online server/snapshot/timestamp keys. A client pins the root
/// fingerprint out of band and verifies every other key chains from it.
pub(super) async fn root_metadata(
    State(state): State<AppState>,
) -> Result<Json<RootResponse>, (StatusCode, Json<ErrorResponse>)> {
    let Some(config) = state.store.registry_config().map_err(internal)? else {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "registry root of trust is not initialized".to_string(),
            }),
        ));
    };
    Ok(Json(RootResponse {
        signed: config.root_json,
        signature: crypto::encode_bytes(&config.root_signature),
        root_key: crypto::encode_bytes(&config.root_public),
        root_fingerprint: crypto::fingerprint(&config.root_public),
    }))
}

/// `GET /snapshot.json` (plan-10-C2): the snapshot-key-signed statement of the
/// current index state (its canonical hash + version + the log checkpoint).
pub(super) async fn snapshot_metadata(
    State(state): State<AppState>,
) -> Result<Json<SignedMetadataResponse>, (StatusCode, Json<ErrorResponse>)> {
    let Some(config) = state.store.registry_config().map_err(internal)? else {
        return Err(metadata_uninitialized());
    };
    let version = state.store.log_size().map_err(internal)?;
    let index_hash = state.store.index_canonical_hash().map_err(internal)?;
    // bug-579: the same memoised head as /log/checkpoint.
    let (checkpoint_size, checkpoint_root_bytes, _signature) =
        state.store.signed_checkpoint().map_err(internal)?;
    let checkpoint_root = hex::encode(checkpoint_root_bytes);
    let signed = format!(
        "{{\"type\":\"snapshot\",\"registryId\":{},\"version\":{},\"expires\":{},\"indexHash\":{},\"checkpoint\":{{\"size\":{},\"rootHash\":{}}}}}",
        json_str(&config.registry_id),
        version,
        now_unix() + SNAPSHOT_TTL_SECS,
        json_str(&index_hash),
        checkpoint_size,
        json_str(&checkpoint_root),
    );
    let signature = crypto::sign(
        &config.snapshot_private,
        &crypto::snapshot_signing_input(signed.as_bytes()),
    )
    .map_err(internal)?;
    Ok(Json(SignedMetadataResponse {
        signed,
        signature: crypto::encode_bytes(&signature),
    }))
}

/// `GET /timestamp.json` (plan-10-C2): the timestamp-key-signed pointer to the
/// current snapshot version + index hash. Short-lived, refreshed on demand.
pub(super) async fn timestamp_metadata(
    State(state): State<AppState>,
) -> Result<Json<SignedMetadataResponse>, (StatusCode, Json<ErrorResponse>)> {
    let Some(config) = state.store.registry_config().map_err(internal)? else {
        return Err(metadata_uninitialized());
    };
    let version = state.store.log_size().map_err(internal)?;
    let index_hash = state.store.index_canonical_hash().map_err(internal)?;
    let signed = format!(
        "{{\"type\":\"timestamp\",\"registryId\":{},\"version\":{},\"expires\":{},\"snapshotVersion\":{},\"indexHash\":{}}}",
        json_str(&config.registry_id),
        version,
        now_unix() + TIMESTAMP_TTL_SECS,
        version,
        json_str(&index_hash),
    );
    let signature = crypto::sign(
        &config.timestamp_private,
        &crypto::timestamp_signing_input(signed.as_bytes()),
    )
    .map_err(internal)?;
    Ok(Json(SignedMetadataResponse {
        signed,
        signature: crypto::encode_bytes(&signature),
    }))
}

fn metadata_uninitialized() -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::NOT_FOUND,
        Json(ErrorResponse {
            error: "registry root of trust is not initialized".to_string(),
        }),
    )
}
