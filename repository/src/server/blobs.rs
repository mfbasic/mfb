//! plan-176-L: the registry server's blobs code. `AppState`, `serve`, the router and the
//! error responses live in `repository/src/server/mod.rs`.

use super::*;

/// `GET /blob/<hash>` (plan-10-A): serve the content-addressed `<hash>.mfp`
/// blob. The local backend streams the bytes inline, re-checking the recomputed
/// hash as a blob-store corruption defense. The S3 backend answers with a `302`
/// redirect to a short-lived presigned URL, so the bytes never transit the app
/// server; the client re-hashes what it downloads, so the integrity check moves
/// to the client on that path.
/// Validate a `/blob/<hash>` path component: 64 lowercase hex characters.
/// Shared by `GET`, `HEAD`, and `PUT` so all three reject malformed hashes
/// identically.
pub(super) fn validate_blob_hash(hash: &str) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    if hash.len() != 64
        || !hash
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(bad_request(
            "blob hash must be 64 lowercase hex characters".to_string(),
        ));
    }
    Ok(())
}

/// Resolve a blob hash to its stored [`BlobKind`], or `None` if the registry
/// has no such blob. A blob predating the `kind` column reads back as
/// `package` via the column default.
async fn resolve_blob_kind(
    state: &AppState,
    hash: &str,
) -> Result<Option<BlobKind>, (StatusCode, Json<ErrorResponse>)> {
    match state.store.blob_kind(hash).map_err(internal)? {
        Some(kind) => Ok(Some(BlobKind::from_db_str(&kind).map_err(internal)?)),
        None => Ok(None),
    }
}

pub(super) async fn package_blob(
    State(state): State<AppState>,
    axum::extract::Path(hash): axum::extract::Path<String>,
) -> Result<axum::response::Response, (StatusCode, Json<ErrorResponse>)> {
    validate_blob_hash(&hash)?;
    // Learn the blob's kind from the index first: an unknown hash 404s here
    // without touching the backend (no S3 round trip), and the kind selects the
    // right on-disk/S3 name suffix.
    let kind = match resolve_blob_kind(&state, &hash).await? {
        Some(kind) => kind,
        None => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "no blob with that hash".to_string(),
                }),
            ));
        }
    };
    let fetch = match state.blob_store.get(&hash, kind).await.map_err(internal)? {
        Some(fetch) => fetch,
        None => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "no blob with that hash".to_string(),
                }),
            ));
        }
    };
    match fetch {
        BlobFetch::Bytes(bytes) => {
            if hex::encode(crypto::sha256(&bytes)) != hash {
                return Err(internal(
                    "stored blob hash does not match its path (blob-store corruption)".to_string(),
                ));
            }
            axum::response::Response::builder()
                .status(StatusCode::OK)
                .header(axum::http::header::CONTENT_TYPE, "application/octet-stream")
                .header(
                    axum::http::header::CACHE_CONTROL,
                    "public, max-age=31536000, immutable",
                )
                .body(axum::body::Body::from(bytes))
                .map_err(|err| internal(format!("failed to build blob response: {err}")))
        }
        #[cfg(feature = "s3")]
        BlobFetch::Redirect(url) => axum::response::Response::builder()
            .status(StatusCode::FOUND)
            .header(axum::http::header::LOCATION, url)
            .header(axum::http::header::CACHE_CONTROL, "no-store")
            .body(axum::body::Body::empty())
            .map_err(|err| internal(format!("failed to build blob redirect: {err}"))),
    }
}

/// `HEAD /blob/<hash>` — the dedup probe (plan-48-A §4.2). `200` if a servable
/// blob exists, `404` otherwise; no body, no auth (it discloses only whether a
/// content hash the caller already knows is present, exactly what `GET` reveals).
pub(super) async fn head_blob(
    State(state): State<AppState>,
    axum::extract::Path(hash): axum::extract::Path<String>,
) -> StatusCode {
    if validate_blob_hash(&hash).is_err() {
        return StatusCode::BAD_REQUEST;
    }
    match state.store.blob_kind(&hash) {
        Ok(Some(kind)) => match BlobKind::from_db_str(&kind) {
            Ok(kind) => match state.blob_store.exists(&hash, kind).await {
                Ok(true) => StatusCode::OK,
                Ok(false) => StatusCode::NOT_FOUND,
                Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
            },
            Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
        },
        Ok(None) => StatusCode::NOT_FOUND,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// `PUT /blob/<hash>` — the write half (plan-48-A §4.3). Session-authenticated
/// via `Authorization: Bearer`, hash-verified before storage, idempotent, and
/// rate-limited per owner. Stores the raw bytes as a native-library `.bin` blob.
pub(super) async fn put_blob(
    State(state): State<AppState>,
    axum::extract::Path(hash): axum::extract::Path<String>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    validate_blob_hash(&hash)?;
    // Auth: the session JWT rides in an Authorization: Bearer header here — the
    // only header-borne credential on this server, because a raw-body PUT cannot
    // carry the body-field `sessionToken` every other authenticated route uses.
    let token = bearer_token(&headers)?;
    let claims = verify_session_token(&state.store, token).map_err(|err| unauthorized(&err))?;
    // bug-492: a publish token's expiry binds this route too. A blob has no
    // package, so scope does not apply, but an expired token must not keep
    // staging bytes on the strength of its still-live session JWT.
    let Some((_owner, key)) = state
        .store
        .owner_auth_key_by_fingerprint(&claims.sub, &claims.auth_fingerprint)
        .map_err(internal)?
    else {
        return Err(unauthorized("session key is not a current auth key"));
    };
    publish_token_scope(&state.store, key.id)
        .map_err(|(_status, Json(err))| unauthorized(&err.error))?;
    // Per-owner upload throttle (§4.3): non-optional given there is no GC, so an
    // authenticated publisher cannot fill the datapath with unreclaimable bytes.
    if !state.rate_limiter.allow(
        &format!("blob:{}", claims.sub),
        BLOB_UPLOAD_PER_OWNER_MAX,
        60,
    ) {
        return Err(too_many_requests());
    }
    // Content-address verification before storing anything: the store is keyed
    // by content hash, so this is the invariant that keeps it honest.
    let actual = hex::encode(crypto::sha256(&body));
    if actual != hash {
        return Err(bad_request(format!(
            "blob body hash {actual} does not match path hash {hash}"
        )));
    }
    // Idempotent: re-uploading an existing blob is a cheap success, not an error.
    if state
        .blob_store
        .exists(&hash, BlobKind::Native)
        .await
        .map_err(internal)?
    {
        return Ok(StatusCode::OK);
    }
    // stage → row → promote, aborting on failure — the exact order publish uses,
    // preserving the "no servable orphan" invariant.
    let staged = state
        .blob_store
        .stage(&hash, BlobKind::Native, body.to_vec())
        .await
        .map_err(internal)?;
    let blob_ref = state.blob_store.blob_ref(&hash, BlobKind::Native);
    let promote_bin = match state.store.record_native_blob(&hash, &blob_ref) {
        Ok(promote_bin) => promote_bin,
        Err(err) => {
            state.blob_store.abort(staged).await;
            return Err(internal(err));
        }
    };
    if !promote_bin {
        // These bytes are already stored under another kind, and `GET /blob` will
        // serve them from that row (bug-276 R5). Promoting would leave a second,
        // unreferenced copy, so drop the staging instead and report the upload as
        // the no-op it is.
        state.blob_store.abort(staged).await;
        return Ok(StatusCode::OK);
    }
    if let Err(err) = state.blob_store.promote(staged).await {
        return Err(internal(err));
    }
    Ok(StatusCode::CREATED)
}
