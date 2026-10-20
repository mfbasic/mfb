//! plan-176-L: the registry server's log_proofs code. `AppState`, `serve`, the router and the
//! error responses live in `repository/src/server/mod.rs`.

use super::*;

pub(super) async fn log_checkpoint(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
) -> Result<Json<CheckpointResponse>, (StatusCode, Json<ErrorResponse>)> {
    if !state
        .rate_limiter
        .allow(&format!("log:{}", peer.ip()), LOG_PER_IP_MAX, 60)
    {
        return Err(too_many_requests());
    }
    // bug-579: memoised by log size, so a repeated call is a COUNT(*) rather
    // than loading every leaf, rebuilding the Merkle root and signing it again.
    let (size, root, signature) = state.store.signed_checkpoint().map_err(internal)?;
    Ok(Json(CheckpointResponse {
        size,
        root_hash: hex::encode(root),
        signature: crypto::encode_bytes(&signature),
    }))
}

#[derive(Debug, Deserialize)]
pub(super) struct ProofQuery {
    pub(super) size: Option<i64>,
}

pub(super) async fn log_inclusion_proof(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    axum::extract::Path(index): axum::extract::Path<i64>,
    axum::extract::Query(query): axum::extract::Query<ProofQuery>,
) -> Result<Json<InclusionProofResponse>, (StatusCode, Json<ErrorResponse>)> {
    if !state
        .rate_limiter
        .allow(&format!("log:{}", peer.ip()), LOG_PER_IP_MAX, 60)
    {
        return Err(too_many_requests());
    }
    let leaves = state.store.log_leaf_hashes(query.size).map_err(internal)?;
    let size = leaves.len() as i64;
    if index < 0 || index >= size {
        return Err(bad_request(
            "log entry index is outside the tree".to_string(),
        ));
    }
    let path = crate::log::inclusion_path(index as usize, &leaves)
        .into_iter()
        .map(hex::encode)
        .collect();
    Ok(Json(InclusionProofResponse {
        index,
        size,
        leaf_hash: hex::encode(leaves[index as usize]),
        path,
    }))
}

#[derive(Debug, Deserialize)]
pub(super) struct ConsistencyQuery {
    pub(super) from: i64,
    pub(super) to: Option<i64>,
}

pub(super) async fn log_consistency_proof(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    axum::extract::Query(query): axum::extract::Query<ConsistencyQuery>,
) -> Result<Json<ConsistencyProofResponse>, (StatusCode, Json<ErrorResponse>)> {
    if !state
        .rate_limiter
        .allow(&format!("log:{}", peer.ip()), LOG_PER_IP_MAX, 60)
    {
        return Err(too_many_requests());
    }
    let leaves = state.store.log_leaf_hashes(query.to).map_err(internal)?;
    let to = leaves.len() as i64;
    if query.from < 0 || query.from > to {
        return Err(bad_request(
            "consistency proof sizes are invalid".to_string(),
        ));
    }
    let path = crate::log::consistency_path(query.from as usize, &leaves)
        .into_iter()
        .map(hex::encode)
        .collect();
    Ok(Json(ConsistencyProofResponse {
        from: query.from,
        to,
        path,
    }))
}

#[derive(Debug, Deserialize)]
pub(super) struct PublishEntryQuery {
    pub(super) ident: String,
    pub(super) version: String,
}

pub(super) async fn log_publish_entry(
    State(state): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<PublishEntryQuery>,
) -> Result<Json<LogEntry>, (StatusCode, Json<ErrorResponse>)> {
    let Some(entry) = state
        .store
        .publish_log_entry(&query.ident, &query.version)
        .map_err(internal)?
    else {
        return Err(bad_request(
            "no publish log entry for that package".to_string(),
        ));
    };
    Ok(Json(LogEntry {
        index: entry.index,
        leaf_hash: hex::encode(entry.leaf_hash),
    }))
}
