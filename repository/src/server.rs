use crate::{crypto, package};
use crate::store::{now_unix, NewSession, Store};
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::PathBuf;
use tokio::net::TcpListener;
use uuid::Uuid;

#[derive(Clone)]
pub struct AppState {
    store: Store,
    packages_dir: PathBuf,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RegisterRequest {
    pub owner: String,
    #[serde(rename = "authKey")]
    pub auth_key: String,
    #[serde(rename = "identKey")]
    pub ident_key: String,
    /// Role-separated proof-of-possession signatures: each private key signs
    /// the role-discriminated registration message for its own role, so one
    /// proof can never be replayed as the other (plan-23 Phase A1).
    pub proofs: RegisterProofs,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RegisterProofs {
    pub auth: String,
    pub ident: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RegisterResponse {
    pub owner: String,
    #[serde(rename = "authFingerprint")]
    pub auth_fingerprint: String,
    #[serde(rename = "identFingerprint")]
    pub ident_fingerprint: String,
}

/// `GET /ident` — the registry's own public key (plan-23 index §10.3). Clients
/// pin this as `server.pub` on first contact; its fingerprint is the
/// `repoFingerprint` named in every attestation.
#[derive(Debug, Serialize, Deserialize)]
pub struct ServerIdentResponse {
    #[serde(rename = "serverKey")]
    pub server_key: String,
    #[serde(rename = "serverFingerprint")]
    pub server_fingerprint: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChallengeRequest {
    pub owner: String,
    #[serde(rename = "authFingerprint")]
    pub auth_fingerprint: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChallengeResponse {
    #[serde(rename = "challengeId")]
    pub challenge_id: String,
    pub nonce: String,
    #[serde(rename = "expiresAt")]
    pub expires_at: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LoginRequest {
    #[serde(rename = "challengeId")]
    pub challenge_id: String,
    pub signature: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LoginResponse {
    #[serde(rename = "sessionToken")]
    pub session_token: String,
    pub owner: String,
    #[serde(rename = "expiresAt")]
    pub expires_at: i64,
}

/// `POST /signing` (plan-23 §3.3): an authenticated build pre-registers the
/// one-off signing key for one exact package+version and receives the
/// server-signed attestation naming it.
#[derive(Debug, Serialize, Deserialize)]
pub struct SigningRequest {
    pub owner: String,
    pub ident: String,
    pub version: String,
    #[serde(rename = "signingFingerprint")]
    pub signing_fingerprint: String,
    #[serde(rename = "sessionToken")]
    pub session_token: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SigningResponse {
    pub owner: String,
    /// The exact attestation JSON bytes the server signed (plan-23 §5).
    pub attestation: String,
    /// Base64url 64-byte Ed25519 server signature over
    /// `"MFP-ATTEST-v1\0" || attestation`.
    #[serde(rename = "attestationSignature")]
    pub attestation_signature: String,
}

/// Machine link (plan-23 §3.2): the old machine relays the argon2id-encrypted
/// ident keypair as a single-use, short-TTL blob the server cannot read.
#[derive(Debug, Serialize, Deserialize)]
pub struct LinkStartRequest {
    pub owner: String,
    /// Code-derived lookup key: `hex(SHA-256("mfb-pairing-lookup-v1\0" || code))`.
    pub lookup: String,
    /// Base64url ciphertext: 12-byte nonce || ChaCha20-Poly1305(ident prv || pub).
    pub blob: String,
    /// Base64url argon2id salt.
    pub salt: String,
    #[serde(rename = "sessionToken")]
    pub session_token: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LinkStartResponse {
    pub owner: String,
    #[serde(rename = "expiresAt")]
    pub expires_at: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LinkFetchRequest {
    pub owner: String,
    pub lookup: String,
    /// The new machine's own auth public key, registered in this exchange.
    #[serde(rename = "authKey")]
    pub auth_key: String,
    /// Role-separated proof-of-possession for the new auth key.
    pub proof: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LinkFetchResponse {
    pub owner: String,
    pub blob: String,
    pub salt: String,
    #[serde(rename = "authFingerprint")]
    pub auth_fingerprint: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RevokeChallengeRequest {
    pub owner: String,
}

/// Ident rotation (plan-23-B2): the old ident signs the chain link naming the
/// new key; the new key proves possession. Requires a live session too, so a
/// rotation needs both credentials.
#[derive(Debug, Serialize, Deserialize)]
pub struct RotateRequest {
    pub owner: String,
    #[serde(rename = "newIdentKey")]
    pub new_ident_key: String,
    /// Base64url old-ident signature over the ident rotation message.
    #[serde(rename = "chainSignature")]
    pub chain_signature: String,
    /// Base64url new-ident proof-of-possession (role-separated registration message).
    #[serde(rename = "possessionProof")]
    pub possession_proof: String,
    #[serde(rename = "sessionToken")]
    pub session_token: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RotateResponse {
    pub owner: String,
    #[serde(rename = "identFingerprint")]
    pub ident_fingerprint: String,
}

/// `GET /idents/<owner>` — the current name↔ident binding plus the signed
/// rotation chain, oldest link first. Consumers follow the chain to update
/// their pins; a current key not reachable from a consumer's pin through the
/// chain means the ident was re-anchored (hard error client-side).
#[derive(Debug, Serialize, Deserialize)]
pub struct IdentChainResponse {
    pub owner: String,
    #[serde(rename = "identKey")]
    pub ident_key: String,
    #[serde(rename = "identFingerprint")]
    pub ident_fingerprint: String,
    pub chain: Vec<IdentChainLink>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct IdentChainLink {
    #[serde(rename = "oldKey")]
    pub old_key: String,
    #[serde(rename = "newKey")]
    pub new_key: String,
    pub signature: String,
    pub issued: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RevokeRequest {
    #[serde(rename = "challengeId")]
    pub challenge_id: String,
    /// Fingerprint of the auth key being revoked.
    #[serde(rename = "authFingerprint")]
    pub auth_fingerprint: String,
    /// Base64url ident signature over the revocation message.
    #[serde(rename = "identSignature")]
    pub ident_signature: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RevokeResponse {
    pub owner: String,
    #[serde(rename = "authFingerprint")]
    pub auth_fingerprint: String,
    pub revoked: bool,
}

/// `POST /release-state` (plan-10-C1): a maintainer sets a published version's
/// release state. Requires both a live session (auth) and an ident signature
/// (authority) — an auth session alone can never change a release state.
#[derive(Debug, Serialize, Deserialize)]
pub struct ReleaseStateRequest {
    pub owner: String,
    pub ident: String,
    pub version: String,
    /// One of `available`, `deprecated`, `yanked`. `blocked` and
    /// `legal-tombstoned` are registry-operator states, refused here.
    pub state: String,
    #[serde(rename = "sessionToken")]
    pub session_token: String,
    /// Base64url ident signature over `release_state_message(ident, version, state)`.
    #[serde(rename = "identSignature")]
    pub ident_signature: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ReleaseStateResponse {
    pub ident: String,
    pub version: String,
    pub state: String,
    #[serde(rename = "logEntry")]
    pub log_entry: LogEntry,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PackageArtifactRequest {
    pub ident: String,
    pub version: String,
    pub artifact: String,
    #[serde(rename = "contentHash")]
    pub content_hash: String,
    #[serde(rename = "identFingerprint")]
    pub ident_fingerprint: String,
    #[serde(rename = "signingFingerprint")]
    pub signing_fingerprint: String,
    #[serde(rename = "sessionToken")]
    pub session_token: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ValidatePackageResponse {
    pub valid: bool,
    #[serde(rename = "contentHash")]
    pub content_hash: String,
    #[serde(rename = "abiIndex")]
    pub abi_index: serde_json::Value,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PublishPackageResponse {
    pub ident: String,
    pub version: String,
    pub hash: String,
    #[serde(rename = "publishedAt")]
    pub published_at: i64,
    pub state: String,
    #[serde(rename = "blobStored")]
    pub blob_stored: bool,
    /// The publish's transparency-log entry (plan-23-B3).
    #[serde(rename = "logEntry")]
    pub log_entry: LogEntry,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LogEntry {
    pub index: i64,
    #[serde(rename = "leafHash")]
    pub leaf_hash: String,
}

/// `GET /index/<owner>#<package>` (plan-10-A): the published version list plus
/// the owner's current ident key and a server-signed name binding, so a first
/// `mfb pkg add` can pin the ident against a registry-authenticated anchor.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct IndexResponse {
    pub ident: String,
    pub owner: String,
    /// The owner's current ident public key in metadata form
    /// (`ed25519:<base64url>`), ready to pin into `project.json`.
    #[serde(rename = "identKey")]
    pub ident_key: String,
    #[serde(rename = "identFingerprint")]
    pub ident_fingerprint: String,
    /// Server signature over `name_binding_message(owner, identFingerprint)`,
    /// verifiable under the pinned `server.pub`.
    #[serde(rename = "nameBindingSignature")]
    pub name_binding_signature: String,
    #[serde(rename = "serverFingerprint")]
    pub server_fingerprint: String,
    pub versions: Vec<IndexVersion>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct IndexVersion {
    pub version: String,
    pub hash: String,
    #[serde(rename = "publishedAt")]
    pub published_at: i64,
    pub state: String,
    /// Per-symbol ABI index (plan-10-B1): `{ "<symbol>": "<hex abiHash>" }`.
    #[serde(rename = "abiIndex")]
    pub abi_index: serde_json::Value,
    /// The version's publish transparency-log entry (plan-23-B3), if present.
    #[serde(rename = "logEntry")]
    pub log_entry: Option<LogEntry>,
}

impl IndexVersion {
    /// The exported-symbol ABI as an ordered `symbol -> hex hash` map (empty
    /// when the package carries no ABI index). Used by the client resolver
    /// (plan-10-B2) for the superset compatibility check.
    pub fn abi_map(&self) -> std::collections::BTreeMap<String, String> {
        self.abi_index
            .as_object()
            .map(|object| {
                object
                    .iter()
                    .filter_map(|(name, value)| {
                        value.as_str().map(|hash| (name.clone(), hash.to_string()))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// `GET /log/checkpoint` — the signed tree head (plan-23-B3): size + RFC 6962
/// root, signed by the server key so a checkpoint cannot be forged and two
/// consumers can compare views.
#[derive(Debug, Serialize, Deserialize)]
pub struct CheckpointResponse {
    pub size: i64,
    #[serde(rename = "rootHash")]
    pub root_hash: String,
    pub signature: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct InclusionProofResponse {
    pub index: i64,
    pub size: i64,
    #[serde(rename = "leafHash")]
    pub leaf_hash: String,
    pub path: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ConsistencyProofResponse {
    pub from: i64,
    pub to: i64,
    pub path: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HealthResponse {
    pub ok: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SessionClaims {
    pub sub: String,
    pub owner_id: i64,
    pub auth_fingerprint: String,
    pub iat: i64,
    pub exp: i64,
    pub jti: String,
}

pub async fn serve(store: Store, packages_dir: PathBuf, listen: SocketAddr) -> Result<SocketAddr, String> {
    let state = AppState {
        store,
        packages_dir,
    };
    let app = Router::new()
        .route("/health", get(health))
        .route("/ident", get(server_ident))
        .route("/accounts/register", post(register))
        .route("/auth/challenge", post(challenge))
        .route("/auth/login", post(login))
        .route("/signing", post(signing))
        .route("/log/checkpoint", get(log_checkpoint))
        .route("/log/proof/:index", get(log_inclusion_proof))
        .route("/log/consistency", get(log_consistency_proof))
        .route("/log/publish", get(log_publish_entry))
        .route("/keys/rotate", post(rotate_ident))
        .route("/idents/:owner", get(ident_chain))
        .route("/machines/link", post(link_start))
        .route("/machines/link/fetch", post(link_fetch))
        .route("/machines/revoke/challenge", post(revoke_challenge))
        .route("/machines/revoke", post(revoke_machine))
        .route("/index/:ident", get(package_index))
        .route("/blob/:hash", get(package_blob))
        .route("/release-state", post(release_state))
        .route("/validate", post(validate_package))
        .route("/publish", post(publish_package))
        .with_state(state);
    let listener = TcpListener::bind(listen)
        .await
        .map_err(|err| format!("failed to bind {listen}: {err}"))?;
    let actual = listener
        .local_addr()
        .map_err(|err| format!("failed to read listening address: {err}"))?;
    println!("MFB_REPO_LISTEN={actual}");
    axum::serve(listener, app)
        .await
        .map_err(|err| format!("repository server failed: {err}"))?;
    Ok(actual)
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { ok: true })
}

async fn server_ident(
    State(state): State<AppState>,
) -> Result<Json<ServerIdentResponse>, (StatusCode, Json<ErrorResponse>)> {
    let public = state.store.server_public_key().map_err(internal)?;
    Ok(Json(ServerIdentResponse {
        server_key: crypto::encode_bytes(&public),
        server_fingerprint: crypto::fingerprint(&public),
    }))
}

async fn register(
    State(state): State<AppState>,
    Json(request): Json<RegisterRequest>,
) -> Result<Json<RegisterResponse>, (StatusCode, Json<ErrorResponse>)> {
    let auth_key = crypto::decode_bytes(&request.auth_key, "authKey").map_err(bad_request)?;
    let ident_key = crypto::decode_bytes(&request.ident_key, "identKey").map_err(bad_request)?;
    let auth_proof = crypto::decode_bytes(&request.proofs.auth, "auth proof").map_err(bad_request)?;
    let ident_proof =
        crypto::decode_bytes(&request.proofs.ident, "ident proof").map_err(bad_request)?;
    let (owner, auth, ident) = state
        .store
        .register_owner(&request.owner, &auth_key, &auth_proof, &ident_key, &ident_proof)
        .map_err(conflict_or_bad_request)?;
    Ok(Json(RegisterResponse {
        owner: owner.owner_display,
        auth_fingerprint: auth.fingerprint,
        ident_fingerprint: ident.fingerprint,
    }))
}

async fn challenge(
    State(state): State<AppState>,
    Json(request): Json<ChallengeRequest>,
) -> Result<Json<ChallengeResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Owner existence first, so a missing-key client probe (empty
    // fingerprint) still learns "unknown owner" for an unregistered name.
    if state
        .store
        .owner_with_ident_key(&request.owner)
        .map_err(internal)?
        .is_none()
    {
        return Err(bad_request("unknown owner".to_string()));
    }
    // Machines are equals: challenge the specific machine's auth key.
    let challenge = state
        .store
        .create_auth_challenge(&request.owner, &request.auth_fingerprint)
        .map_err(conflict_or_bad_request)?;
    Ok(Json(ChallengeResponse {
        challenge_id: challenge.id,
        nonce: crypto::encode_bytes(&challenge.nonce),
        expires_at: challenge.expires_at,
    }))
}

async fn log_checkpoint(
    State(state): State<AppState>,
) -> Result<Json<CheckpointResponse>, (StatusCode, Json<ErrorResponse>)> {
    let leaves = state.store.log_leaf_hashes(None).map_err(internal)?;
    let root = crate::log::root(&leaves);
    let (_public, private) = state.store.server_keypair().map_err(internal)?;
    let signature = crypto::sign(
        &private,
        &crate::log::checkpoint_signing_input(leaves.len() as u64, &root),
    )
    .map_err(internal)?;
    Ok(Json(CheckpointResponse {
        size: leaves.len() as i64,
        root_hash: hex::encode(root),
        signature: crypto::encode_bytes(&signature),
    }))
}

#[derive(Debug, Deserialize)]
struct ProofQuery {
    size: Option<i64>,
}

async fn log_inclusion_proof(
    State(state): State<AppState>,
    axum::extract::Path(index): axum::extract::Path<i64>,
    axum::extract::Query(query): axum::extract::Query<ProofQuery>,
) -> Result<Json<InclusionProofResponse>, (StatusCode, Json<ErrorResponse>)> {
    let leaves = state.store.log_leaf_hashes(query.size).map_err(internal)?;
    let size = leaves.len() as i64;
    if index < 0 || index >= size {
        return Err(bad_request("log entry index is outside the tree".to_string()));
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
struct ConsistencyQuery {
    from: i64,
    to: Option<i64>,
}

async fn log_consistency_proof(
    State(state): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<ConsistencyQuery>,
) -> Result<Json<ConsistencyProofResponse>, (StatusCode, Json<ErrorResponse>)> {
    let leaves = state.store.log_leaf_hashes(query.to).map_err(internal)?;
    let to = leaves.len() as i64;
    if query.from < 0 || query.from > to {
        return Err(bad_request("consistency proof sizes are invalid".to_string()));
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
struct PublishEntryQuery {
    ident: String,
    version: String,
}

async fn log_publish_entry(
    State(state): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<PublishEntryQuery>,
) -> Result<Json<LogEntry>, (StatusCode, Json<ErrorResponse>)> {
    let Some(entry) = state
        .store
        .publish_log_entry(&query.ident, &query.version)
        .map_err(internal)?
    else {
        return Err(bad_request("no publish log entry for that package".to_string()));
    };
    Ok(Json(LogEntry {
        index: entry.index,
        leaf_hash: hex::encode(entry.leaf_hash),
    }))
}

/// `GET /index/<owner>#<package>` (plan-10-A): serve the published version
/// list, the owner's current ident key, and a server-signed name binding.
async fn package_index(
    State(state): State<AppState>,
    axum::extract::Path(ident): axum::extract::Path<String>,
) -> Result<Json<IndexResponse>, (StatusCode, Json<ErrorResponse>)> {
    let Some((owner_part, package_part)) = ident.split_once('#') else {
        return Err(bad_request("ident must use <owner>#<package>".to_string()));
    };
    if owner_part.is_empty() || package_part.is_empty() {
        return Err(bad_request("ident must use <owner>#<package>".to_string()));
    }
    let Some((owner_record, ident_key)) = state
        .store
        .owner_with_ident_key(owner_part)
        .map_err(internal)?
    else {
        return Err(bad_request("unknown owner".to_string()));
    };
    let mut versions = Vec::new();
    for row in state.store.list_package_versions(&ident).map_err(internal)? {
        let log_entry = state
            .store
            .publish_log_entry(&ident, &row.version)
            .map_err(internal)?
            .map(|entry| LogEntry {
                index: entry.index,
                leaf_hash: hex::encode(entry.leaf_hash),
            });
        let abi_index = serde_json::from_str(&row.abi_index).unwrap_or_else(|_| serde_json::json!({}));
        versions.push(IndexVersion {
            version: row.version,
            hash: row.hash,
            published_at: row.published_at,
            state: row.state,
            abi_index,
            log_entry,
        });
    }
    let (server_public, server_private) = state.store.server_keypair().map_err(internal)?;
    let name_binding = crypto::sign(
        &server_private,
        &crypto::name_binding_message(&owner_record.owner_display, &ident_key.fingerprint),
    )
    .map_err(internal)?;
    Ok(Json(IndexResponse {
        ident: ident.clone(),
        owner: owner_record.owner_display,
        ident_key: format!("ed25519:{}", crypto::encode_bytes(&ident_key.public_key)),
        ident_fingerprint: ident_key.fingerprint,
        name_binding_signature: crypto::encode_bytes(&name_binding),
        server_fingerprint: crypto::fingerprint(&server_public),
        versions,
    }))
}

/// `POST /release-state` (plan-10-C1): a maintainer moves a published version
/// between `available`/`deprecated`/`yanked`. Requires a live session AND an
/// ident signature — an auth session alone is refused.
async fn release_state(
    State(state): State<AppState>,
    Json(request): Json<ReleaseStateRequest>,
) -> Result<Json<ReleaseStateResponse>, (StatusCode, Json<ErrorResponse>)> {
    let claims = verify_session_token(&state.store, &request.session_token).map_err(bad_request)?;
    if claims.sub != request.owner {
        return Err(bad_request("session owner does not match requested owner".to_string()));
    }
    let Some((owner, _key)) = state
        .store
        .owner_auth_key_by_fingerprint(&request.owner, &claims.auth_fingerprint)
        .map_err(internal)?
    else {
        return Err(bad_request("session key is not a current auth key".to_string()));
    };
    if owner.id != claims.owner_id {
        return Err(bad_request("session owner does not match requested owner".to_string()));
    }
    // Maintainer states only — blocked/legal-tombstoned are operator states.
    if !matches!(request.state.as_str(), "available" | "deprecated" | "yanked") {
        return Err(bad_request(
            "state must be one of available, deprecated, or yanked".to_string(),
        ));
    }
    let Some((ident_owner, package_part)) = request.ident.split_once('#') else {
        return Err(bad_request("ident must use <owner>#<package>".to_string()));
    };
    if package_part.is_empty()
        || crate::validation::fold_owner(ident_owner)
            != crate::validation::fold_owner(&request.owner)
    {
        return Err(bad_request("ident owner does not match session owner".to_string()));
    }
    // Authority is the ident key: verify its signature over the exact change.
    let Some((_owner, ident_key)) = state
        .store
        .owner_with_ident_key(&request.owner)
        .map_err(internal)?
    else {
        return Err(bad_request("owner has no current ident key".to_string()));
    };
    let signature =
        crypto::decode_bytes(&request.ident_signature, "identSignature").map_err(bad_request)?;
    crypto::verify(
        &ident_key.public_key,
        &crypto::release_state_message(&request.ident, &request.version, &request.state),
        &signature,
    )
    .map_err(|_| bad_request("invalid release-state ident signature".to_string()))?;

    let log_entry = state
        .store
        .set_release_state(&request.ident, &request.version, &request.state)
        .map_err(bad_request)?;
    Ok(Json(ReleaseStateResponse {
        ident: request.ident,
        version: request.version,
        state: request.state,
        log_entry: LogEntry {
            index: log_entry.index,
            leaf_hash: hex::encode(log_entry.leaf_hash),
        },
    }))
}

/// `GET /blob/<hash>` (plan-10-A): stream `packages/<hash>.mfp`. The blob is
/// content-addressed and immutable; the recomputed hash is checked against the
/// path on read as a blob-store corruption defense.
async fn package_blob(
    State(state): State<AppState>,
    axum::extract::Path(hash): axum::extract::Path<String>,
) -> Result<axum::response::Response, (StatusCode, Json<ErrorResponse>)> {
    if hash.len() != 64
        || !hash
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(bad_request("blob hash must be 64 lowercase hex characters".to_string()));
    }
    let path = state.packages_dir.join(format!("{hash}.mfp"));
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(_) => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "no blob with that hash".to_string(),
                }),
            ));
        }
    };
    if hex::encode(crypto::sha256(&bytes)) != hash {
        return Err(internal(
            "stored blob hash does not match its path (blob-store corruption)".to_string(),
        ));
    }
    axum::response::Response::builder()
        .status(StatusCode::OK)
        .header(axum::http::header::CONTENT_TYPE, "application/octet-stream")
        .header(axum::http::header::CACHE_CONTROL, "public, max-age=31536000, immutable")
        .body(axum::body::Body::from(bytes))
        .map_err(|err| internal(format!("failed to build blob response: {err}")))
}

async fn rotate_ident(
    State(state): State<AppState>,
    Json(request): Json<RotateRequest>,
) -> Result<Json<RotateResponse>, (StatusCode, Json<ErrorResponse>)> {
    let claims = verify_session_token(&state.store, &request.session_token).map_err(bad_request)?;
    if claims.sub != request.owner {
        return Err(bad_request("session owner does not match requested owner".to_string()));
    }
    let Some((owner, _key)) = state
        .store
        .owner_auth_key_by_fingerprint(&request.owner, &claims.auth_fingerprint)
        .map_err(internal)?
    else {
        return Err(bad_request("session key is not a current auth key".to_string()));
    };
    if owner.id != claims.owner_id {
        return Err(bad_request("session owner does not match requested owner".to_string()));
    }
    let new_public =
        crypto::decode_bytes(&request.new_ident_key, "newIdentKey").map_err(bad_request)?;
    let chain_signature =
        crypto::decode_bytes(&request.chain_signature, "chainSignature").map_err(bad_request)?;
    let possession_proof =
        crypto::decode_bytes(&request.possession_proof, "possessionProof").map_err(bad_request)?;
    let (owner, new_key) = state
        .store
        .rotate_ident(&request.owner, &new_public, &chain_signature, &possession_proof)
        .map_err(conflict_or_bad_request)?;
    Ok(Json(RotateResponse {
        owner: owner.owner_display,
        ident_fingerprint: new_key.fingerprint,
    }))
}

async fn ident_chain(
    State(state): State<AppState>,
    axum::extract::Path(owner): axum::extract::Path<String>,
) -> Result<Json<IdentChainResponse>, (StatusCode, Json<ErrorResponse>)> {
    let Some((owner_record, ident_key)) = state
        .store
        .owner_with_ident_key(&owner)
        .map_err(internal)?
    else {
        return Err(bad_request("unknown owner".to_string()));
    };
    let chain = state
        .store
        .ident_chain(&owner)
        .map_err(internal)?
        .into_iter()
        .map(|(old_key, new_key, signature, issued)| IdentChainLink {
            old_key: crypto::encode_bytes(&old_key),
            new_key: crypto::encode_bytes(&new_key),
            signature: crypto::encode_bytes(&signature),
            issued,
        })
        .collect();
    Ok(Json(IdentChainResponse {
        owner: owner_record.owner_display,
        ident_key: crypto::encode_bytes(&ident_key.public_key),
        ident_fingerprint: ident_key.fingerprint,
        chain,
    }))
}

/// Old-machine side of a link (plan-23 §3.2): store the encrypted ident blob
/// under the code-derived lookup. Requires an authenticated session — an
/// anonymous caller cannot park blobs on an account.
async fn link_start(
    State(state): State<AppState>,
    Json(request): Json<LinkStartRequest>,
) -> Result<Json<LinkStartResponse>, (StatusCode, Json<ErrorResponse>)> {
    let claims = verify_session_token(&state.store, &request.session_token).map_err(bad_request)?;
    if claims.sub != request.owner {
        return Err(bad_request("session owner does not match requested owner".to_string()));
    }
    let Some((owner, _key)) = state
        .store
        .owner_auth_key_by_fingerprint(&request.owner, &claims.auth_fingerprint)
        .map_err(internal)?
    else {
        return Err(bad_request("session key is not a current auth key".to_string()));
    };
    if owner.id != claims.owner_id {
        return Err(bad_request("session owner does not match requested owner".to_string()));
    }
    if request.lookup.len() != 64
        || !request
            .lookup
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(bad_request("malformed pairing lookup".to_string()));
    }
    let blob = crypto::decode_bytes(&request.blob, "blob").map_err(bad_request)?;
    let salt = crypto::decode_bytes(&request.salt, "salt").map_err(bad_request)?;
    if blob.is_empty() || blob.len() > 4096 || salt.is_empty() || salt.len() > 64 {
        return Err(bad_request("malformed pairing blob".to_string()));
    }
    let expires_at = state
        .store
        .store_pairing_blob(owner.id, &request.lookup, &blob, &salt)
        .map_err(conflict_or_bad_request)?;
    Ok(Json(LinkStartResponse {
        owner: owner.owner_display,
        expires_at,
    }))
}

/// New-machine side of a link: presenting the correct code-derived lookup is
/// the pairing approval. The new machine's auth key is registered to the
/// account and the (single-use) blob handed over in the same exchange.
async fn link_fetch(
    State(state): State<AppState>,
    Json(request): Json<LinkFetchRequest>,
) -> Result<Json<LinkFetchResponse>, (StatusCode, Json<ErrorResponse>)> {
    let auth_key = crypto::decode_bytes(&request.auth_key, "authKey").map_err(bad_request)?;
    let proof = crypto::decode_bytes(&request.proof, "proof").map_err(bad_request)?;
    // Verify the proof BEFORE consuming the single-use blob, so a malformed
    // request cannot burn a pending pairing.
    let Some((owner_record, _ident)) = state
        .store
        .owner_with_ident_key(&request.owner)
        .map_err(internal)?
    else {
        return Err(bad_request("unknown owner".to_string()));
    };
    let message = crypto::registration_message(
        crypto::ROLE_AUTH,
        &owner_record.owner_display,
        &auth_key,
    );
    crypto::verify(&auth_key, &message, &proof)
        .map_err(|_| bad_request("invalid auth proof-of-possession signature".to_string()))?;
    let Some((blob, salt)) = state
        .store
        .take_pairing_blob(&request.owner, &request.lookup)
        .map_err(internal)?
    else {
        return Err(bad_request(
            "unknown, used, or expired pairing code".to_string(),
        ));
    };
    let (owner, key) = state
        .store
        .add_auth_key(&request.owner, &auth_key, &proof)
        .map_err(conflict_or_bad_request)?;
    Ok(Json(LinkFetchResponse {
        owner: owner.owner_display,
        blob: crypto::encode_bytes(&blob),
        salt: crypto::encode_bytes(&salt),
        auth_fingerprint: key.fingerprint,
    }))
}

/// Issue an ident challenge for an auth-key revocation. Revocation authority
/// is the ident key alone (plan-23 §3.6): an auth session must NOT suffice,
/// and the machine holding the ident key may not have a live session.
async fn revoke_challenge(
    State(state): State<AppState>,
    Json(request): Json<RevokeChallengeRequest>,
) -> Result<Json<ChallengeResponse>, (StatusCode, Json<ErrorResponse>)> {
    let challenge = state
        .store
        .create_ident_challenge(&request.owner)
        .map_err(conflict_or_bad_request)?;
    Ok(Json(ChallengeResponse {
        challenge_id: challenge.id,
        nonce: crypto::encode_bytes(&challenge.nonce),
        expires_at: challenge.expires_at,
    }))
}

async fn revoke_machine(
    State(state): State<AppState>,
    Json(request): Json<RevokeRequest>,
) -> Result<Json<RevokeResponse>, (StatusCode, Json<ErrorResponse>)> {
    let signature =
        crypto::decode_bytes(&request.ident_signature, "identSignature").map_err(bad_request)?;
    let (owner, _ident_key) = state
        .store
        .complete_revocation_challenge(
            &request.challenge_id,
            &signature,
            &request.auth_fingerprint,
        )
        .map_err(conflict_or_bad_request)?;
    let revoked = state
        .store
        .revoke_auth_key(owner.id, &request.auth_fingerprint)
        .map_err(internal)?;
    if !revoked {
        return Err(bad_request(
            "no current auth key with that fingerprint".to_string(),
        ));
    }
    Ok(Json(RevokeResponse {
        owner: owner.owner_display,
        auth_fingerprint: request.auth_fingerprint,
        revoked: true,
    }))
}

async fn login(
    State(state): State<AppState>,
    Json(request): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, (StatusCode, Json<ErrorResponse>)> {
    let signature = crypto::decode_bytes(&request.signature, "signature").map_err(bad_request)?;
    let (owner, key) = state
        .store
        .complete_challenge(&request.challenge_id, &signature)
        .map_err(conflict_or_bad_request)?;
    let issued_at = now_unix();
    let expires_at = issued_at + 3600;
    let jwt_id = Uuid::new_v4().to_string();
    let claims = SessionClaims {
        sub: owner.owner_display.clone(),
        owner_id: owner.id,
        auth_fingerprint: key.fingerprint.clone(),
        iat: issued_at,
        exp: expires_at,
        jti: jwt_id.clone(),
    };
    let secret = state.store.server_secret().map_err(internal)?;
    let token = encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(&secret),
    )
    .map_err(|err| internal(format!("failed to sign session token: {err}")))?;
    state
        .store
        .insert_session(&NewSession {
            owner_id: owner.id,
            key_id: key.id,
            jwt_id,
            issued_at,
            expires_at,
        })
        .map_err(internal)?;
    Ok(Json(LoginResponse {
        session_token: token,
        owner: owner.owner_display,
        expires_at,
    }))
}

/// Build the exact attestation JSON bytes (plan-23 §5). Field order is fixed;
/// values are JSON-escaped. No `expires` — an attestation is a statement of
/// fact at `issued`, true forever; freshness is enforced live at publish.
pub fn attestation_json(
    repo_fingerprint: &str,
    owner: &str,
    ident: &str,
    version: &str,
    ident_fingerprint: &str,
    signing_fingerprint: &str,
    issued: i64,
) -> String {
    format!(
        "{{\"repoFingerprint\":{},\"owner\":{},\"ident\":{},\"version\":{},\"identFingerprint\":{},\"signingFingerprint\":{},\"issued\":{}}}",
        json_str(repo_fingerprint),
        json_str(owner),
        json_str(ident),
        json_str(version),
        json_str(ident_fingerprint),
        json_str(signing_fingerprint),
        issued,
    )
}

fn json_str(value: &str) -> String {
    serde_json::to_string(value).expect("JSON string encoding cannot fail")
}

async fn signing(
    State(state): State<AppState>,
    Json(request): Json<SigningRequest>,
) -> Result<Json<SigningResponse>, (StatusCode, Json<ErrorResponse>)> {
    let claims = verify_session_token(&state.store, &request.session_token).map_err(bad_request)?;
    if claims.sub != request.owner {
        return Err(bad_request("session owner does not match requested owner".to_string()));
    }
    let Some((owner, _key)) = state
        .store
        .owner_auth_key_by_fingerprint(&request.owner, &claims.auth_fingerprint)
        .map_err(internal)?
    else {
        return Err(bad_request(
            "session key does not match current auth key".to_string(),
        ));
    };
    if owner.id != claims.owner_id {
        return Err(bad_request(
            "session key does not match current auth key".to_string(),
        ));
    }
    // The attestation pins one exact package+version: the ident must belong
    // to the session owner and the one-off key fingerprint must be
    // well-formed before the server puts its name on them.
    let Some((ident_owner, package_part)) = request.ident.split_once('#') else {
        return Err(bad_request("ident must use <owner>#<package>".to_string()));
    };
    if crate::validation::fold_owner(ident_owner) != crate::validation::fold_owner(&request.owner)
    {
        return Err(bad_request("ident owner does not match session owner".to_string()));
    }
    if package_part.is_empty() || request.ident.len() > 255 {
        return Err(bad_request("malformed ident".to_string()));
    }
    if request.version.is_empty() || request.version.len() > 64 {
        return Err(bad_request("malformed version".to_string()));
    }
    if request.signing_fingerprint.len() != 64
        || !request
            .signing_fingerprint
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(bad_request(
            "signingFingerprint must be 64 lowercase hex characters".to_string(),
        ));
    }
    let Some((_owner, ident_key)) = state
        .store
        .owner_with_ident_key(&request.owner)
        .map_err(internal)?
    else {
        return Err(bad_request("owner has no current ident key".to_string()));
    };

    // Record the request before signing: every attestation the server ever
    // issues is preceded by its log entry (plan-23 §7).
    state
        .store
        .record_signing_request(
            owner.id,
            &request.ident,
            &request.version,
            &request.signing_fingerprint,
        )
        .map_err(internal)?;

    let (server_public, server_private) = state.store.server_keypair().map_err(internal)?;
    let attestation = attestation_json(
        &crypto::fingerprint(&server_public),
        &owner.owner_display,
        &request.ident,
        &request.version,
        &ident_key.fingerprint,
        &request.signing_fingerprint,
        now_unix(),
    );
    let signature = crypto::sign(
        &server_private,
        &crypto::attestation_signing_input(attestation.as_bytes()),
    )
    .map_err(internal)?;
    Ok(Json(SigningResponse {
        owner: owner.owner_display,
        attestation,
        attestation_signature: crypto::encode_bytes(&signature),
    }))
}

async fn validate_package(
    State(state): State<AppState>,
    Json(request): Json<PackageArtifactRequest>,
) -> Result<Json<ValidatePackageResponse>, (StatusCode, Json<ErrorResponse>)> {
    let report = validate_package_request(&state, &request).await?;
    Ok(Json(report))
}

async fn publish_package(
    State(state): State<AppState>,
    Json(request): Json<PackageArtifactRequest>,
) -> Result<Json<PublishPackageResponse>, (StatusCode, Json<ErrorResponse>)> {
    let report = validate_package_request(&state, &request).await?;
    if !report.valid {
        return Err(bad_request(format!(
            "package validation failed: {}",
            report.diagnostics.join("; ")
        )));
    }
    let artifact = crypto::decode_bytes(&request.artifact, "artifact").map_err(bad_request)?;
    let hash = report.content_hash;
    let path = state.packages_dir.join(format!("{hash}.mfp"));
    let already_present = path.exists();
    // Blob-ordering fix (plan-10-A §2.6): stage the blob to a temp file, commit
    // the DB row, and only then rename into place — a failed transaction leaves
    // no orphan blob, and a served blob always has a committed version row.
    let temp_path = state
        .packages_dir
        .join(format!("{hash}.mfp.tmp-{}", Uuid::new_v4()));
    if !already_present {
        std::fs::write(&temp_path, &artifact)
            .map_err(|err| internal(format!("failed to stage package blob: {err}")))?;
    }
    let owner_id = verify_session_token(&state.store, &request.session_token)
        .map_err(bad_request)?
        .owner_id;
    // Persist the ABI index alongside the version row (plan-10-B1); it is
    // parsed from the same validated artifact `report` covered.
    let abi_index = serde_json::to_string(&report.abi_index).unwrap_or_else(|_| "{}".to_string());
    let published = match state.store.publish_package_version(
        owner_id,
        &request.ident,
        &request.version,
        &hash,
        &path.to_string_lossy(),
        &abi_index,
    ) {
        Ok(published) => published,
        Err(err) => {
            if !already_present {
                let _ = std::fs::remove_file(&temp_path);
            }
            return Err(conflict_or_bad_request(err));
        }
    };
    let blob_stored = if already_present {
        false
    } else {
        std::fs::rename(&temp_path, &path).map_err(|err| {
            let _ = std::fs::remove_file(&temp_path);
            internal(format!("failed to persist package blob: {err}"))
        })?;
        true
    };
    Ok(Json(PublishPackageResponse {
        ident: published.ident,
        version: published.version,
        hash: published.hash,
        published_at: published.published_at,
        state: published.state,
        blob_stored,
        log_entry: LogEntry {
            index: published.log_entry.index,
            leaf_hash: hex::encode(published.log_entry.leaf_hash),
        },
    }))
}

async fn validate_package_request(
    state: &AppState,
    request: &PackageArtifactRequest,
) -> Result<ValidatePackageResponse, (StatusCode, Json<ErrorResponse>)> {
    let claims = verify_session_token(&state.store, &request.session_token).map_err(bad_request)?;
    let artifact = crypto::decode_bytes(&request.artifact, "artifact").map_err(bad_request)?;
    let package = match package::parse_mfp_package(&artifact) {
        Ok(package) => package,
        Err(err) => {
            return Ok(invalid_report(String::new(), vec![err]));
        }
    };
    let hash = package.content_hash_hex();
    let mut diagnostics = Vec::new();
    if request.content_hash != hash {
        diagnostics.push("request contentHash does not match package content hash".to_string());
    }
    if request.ident != package.ident {
        diagnostics.push("request ident does not match package ident".to_string());
    }
    if request.version != package.version {
        diagnostics.push("request version does not match package version".to_string());
    }
    let header_ident_fingerprint = package.ident_fingerprint().unwrap_or_default();
    let header_signing_fingerprint = package.signing_fingerprint().unwrap_or_default();
    if request.ident_fingerprint != header_ident_fingerprint {
        diagnostics.push("request identFingerprint does not match package header".to_string());
    }
    if request.signing_fingerprint != header_signing_fingerprint {
        diagnostics.push("request signingFingerprint does not match package header".to_string());
    }

    // §3.4 step 1 — session owner == header owner; ident is <owner>#<package>.
    let Some((owner_part, _package_part)) = package.ident.split_once('#') else {
        diagnostics.push("package ident must use <owner>#<package>".to_string());
        return Ok(invalid_report(hash, diagnostics));
    };
    if crate::validation::fold_owner(owner_part) != crate::validation::fold_owner(&claims.sub) {
        diagnostics.push("session owner does not match package ident owner".to_string());
    }
    let Some((owner, _key)) = state
        .store
        .owner_auth_key_by_fingerprint(owner_part, &claims.auth_fingerprint)
        .map_err(internal)?
    else {
        // Distinguish "owner unknown" from "session key no longer current".
        if state
            .store
            .owner_with_ident_key(owner_part)
            .map_err(internal)?
            .is_none()
        {
            diagnostics.push("package ident owner is not registered".to_string());
            return Ok(invalid_report(hash, diagnostics));
        }
        diagnostics.push("session key does not match current owner key".to_string());
        return Ok(invalid_report(hash, diagnostics));
    };
    if owner.id != claims.owner_id {
        diagnostics.push("session key does not match current owner key".to_string());
    }
    if package.author != owner.owner_display {
        diagnostics.push("package author does not match owner name".to_string());
    }
    if package.signature_type != 1 {
        diagnostics.push("registry publishes require an Ed25519-signed package".to_string());
        return Ok(invalid_report(hash, diagnostics));
    }

    // §3.4 steps 2–4 — the attestation verifies under OUR key, names OUR
    // fingerprint, and pins this exact ident/version/identKey/signingKey.
    let (server_public, _server_private) = state.store.server_keypair().map_err(internal)?;
    if let Err(err) = package::verify_attestation(
        &package,
        &server_public,
        &crypto::fingerprint(&server_public),
    ) {
        diagnostics.push(format!("attestation verification failed: {err}"));
    }

    // §3.4 step 5 — the attestation must match the server's CURRENT
    // name↔ident binding; a stale attestation (pre-rotation) is refused.
    match state.store.owner_with_ident_key(owner_part).map_err(internal)? {
        Some((_ident_owner, ident_key)) => {
            if header_ident_fingerprint != ident_key.fingerprint {
                diagnostics.push(
                    "package identKey does not match the owner's current ident key".to_string(),
                );
            }
        }
        None => diagnostics.push("owner has no current ident key".to_string()),
    }

    // §3.4 step 6 — the proof verifies under the header identKey and pins
    // this exact package.
    match package::decode_metadata_key(&package.ident_key, "identKey") {
        Ok(ident_public) => {
            if let Err(err) = package::verify_proof(&package, &ident_public) {
                diagnostics.push(format!("proof verification failed: {err}"));
            }
        }
        Err(err) => diagnostics.push(err),
    }

    // §3.4 step 7 — the payload hash welds header to payload and the package
    // signature verifies under the one-off signing key.
    if let Err(err) = package::verify_payload_hash(&package) {
        diagnostics.push(err);
    }
    if let Err(err) = package::verify_package_signature(&package) {
        diagnostics.push(format!("package signature verification failed: {err}"));
    }

    if state
        .store
        .package_version_exists(&package.ident, &package.version)
        .map_err(internal)?
    {
        diagnostics.push(format!(
            "package version {}@{} is already published",
            package.ident, package.version
        ));
    }

    // The per-symbol ABI index (plan-10-B1) is parsed best-effort from the
    // payload; it is covered by packageBinaryHash + the signature, so the
    // registry serves it for resolution without having to trust it.
    let abi_index = crate::abi::abi_index_json(&package.payload);

    Ok(ValidatePackageResponse {
        valid: diagnostics.is_empty(),
        content_hash: hash,
        abi_index,
        diagnostics,
    })
}

fn invalid_report(content_hash: String, diagnostics: Vec<String>) -> ValidatePackageResponse {
    ValidatePackageResponse {
        valid: false,
        content_hash,
        abi_index: serde_json::json!({}),
        diagnostics,
    }
}

pub fn verify_session_token(store: &Store, token: &str) -> Result<SessionClaims, String> {
    let secret = store.server_secret()?;
    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_exp = true;
    let decoded = decode::<SessionClaims>(
        token,
        &DecodingKey::from_secret(&secret),
        &validation,
    )
    .map_err(|_| "expired or malformed session token".to_string())?;
    if !store.session_exists(&decoded.claims.jti)? {
        return Err("unknown session token".to_string());
    }
    Ok(decoded.claims)
}

fn bad_request(message: String) -> (StatusCode, Json<ErrorResponse>) {
    (StatusCode::BAD_REQUEST, Json(ErrorResponse { error: message }))
}

fn conflict_or_bad_request(message: String) -> (StatusCode, Json<ErrorResponse>) {
    if message.contains("already in use") || message.contains("reused challenge") {
        (StatusCode::CONFLICT, Json(ErrorResponse { error: message }))
    } else {
        bad_request(message)
    }
}

fn internal(message: String) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse { error: message }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;

    struct TestOwnerKeys {
        auth_private: Vec<u8>,
        ident_public: Vec<u8>,
        ident_private: Vec<u8>,
    }

    fn register_owner_with_all_keys(store: &Store, owner: &str) -> TestOwnerKeys {
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
            .register_owner(owner, &auth_public, &auth_proof, &ident_public, &ident_proof)
            .unwrap();
        TestOwnerKeys {
            auth_private,
            ident_public,
            ident_private,
        }
    }

    fn register_owner_with_keys(store: &Store, owner: &str) -> (Vec<u8>, Vec<u8>) {
        let keys = register_owner_with_all_keys(store, owner);
        (Vec::new(), keys.auth_private)
    }

    #[test]
    fn jwt_creation_sets_expected_claims_and_verifies() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("meta.db");
        let data_path = temp.path().join("data");
        let opened = Store::open_repository(&db_path, &data_path).unwrap();
        let store = opened.store;
        let (_public, private) = register_owner_with_keys(&store, "alice");

        let challenge = store.create_challenge("alice").unwrap();
        let message = crypto::challenge_message(&challenge.id, &challenge.nonce);
        let signature = crypto::sign(&private, &message).unwrap();
        let (owner, key) = store.complete_challenge(&challenge.id, &signature).unwrap();
        let issued_at = crate::store::now_unix();
        let expires_at = issued_at + 3600;
        let jwt_id = Uuid::new_v4().to_string();
        let claims = SessionClaims {
            sub: owner.owner_display,
            owner_id: owner.id,
            auth_fingerprint: key.fingerprint,
            iat: issued_at,
            exp: expires_at,
            jti: jwt_id.clone(),
        };
        let token = encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(&store.server_secret().unwrap()),
        )
        .unwrap();
        store
            .insert_session(&NewSession {
                owner_id: owner.id,
                key_id: key.id,
                jwt_id,
                issued_at,
                expires_at,
            })
            .unwrap();

        let payload = token.split('.').nth(1).unwrap();
        let decoded = URL_SAFE_NO_PAD.decode(payload).unwrap();
        let decoded: SessionClaims = serde_json::from_slice(&decoded).unwrap();
        assert_eq!(decoded.sub, "alice");
        assert!(decoded.exp - decoded.iat <= 3600);
        assert_eq!(verify_session_token(&store, &token).unwrap().sub, "alice");
        assert!(verify_session_token(&store, "bad.token").is_err());
    }

    fn open_session(store: &Store, owner_name: &str, private: &[u8]) -> String {
        let challenge = store.create_challenge(owner_name).unwrap();
        let message = crypto::challenge_message(&challenge.id, &challenge.nonce);
        let signature = crypto::sign(private, &message).unwrap();
        let (owner, key) = store.complete_challenge(&challenge.id, &signature).unwrap();
        let issued_at = crate::store::now_unix();
        let expires_at = issued_at + 3600;
        let jwt_id = Uuid::new_v4().to_string();
        let claims = SessionClaims {
            sub: owner.owner_display,
            owner_id: owner.id,
            auth_fingerprint: key.fingerprint,
            iat: issued_at,
            exp: expires_at,
            jti: jwt_id.clone(),
        };
        let token = encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(&store.server_secret().unwrap()),
        )
        .unwrap();
        store
            .insert_session(&NewSession {
                owner_id: owner.id,
                key_id: key.id,
                jwt_id,
                issued_at,
                expires_at,
            })
            .unwrap();
        token
    }

    #[tokio::test]
    async fn signing_requires_session_and_matching_owner_and_issues_attestation() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("meta.db");
        let data_path = temp.path().join("data");
        let opened = Store::open_repository(&db_path, &data_path).unwrap();
        let store = opened.store;
        let (_public, private) = register_owner_with_keys(&store, "alice");
        let token = open_session(&store, "alice", &private);
        let state = AppState {
            store: store.clone(),
            packages_dir: temp.path().join("data"),
        };
        let (signing_public, _signing_private) = crypto::generate_keypair();
        let signing_fingerprint = crypto::fingerprint(&signing_public);

        // No/garbage session is refused.
        let refused = signing(
            State(state.clone()),
            Json(SigningRequest {
                owner: "alice".to_string(),
                ident: "alice#toolbox".to_string(),
                version: "1.2.3".to_string(),
                signing_fingerprint: signing_fingerprint.clone(),
                session_token: "bad.token.here".to_string(),
            }),
        )
        .await;
        assert!(refused.is_err());

        // A session for alice cannot request an attestation naming bob.
        let refused = signing(
            State(state.clone()),
            Json(SigningRequest {
                owner: "alice".to_string(),
                ident: "bob#toolbox".to_string(),
                version: "1.2.3".to_string(),
                signing_fingerprint: signing_fingerprint.clone(),
                session_token: token.clone(),
            }),
        )
        .await;
        let (_status, body) = refused.err().expect("mismatched ident owner refused");
        assert!(body.error.contains("ident owner does not match"), "{}", body.error);

        // A session for alice cannot pose as bob either.
        let refused = signing(
            State(state.clone()),
            Json(SigningRequest {
                owner: "bob".to_string(),
                ident: "bob#toolbox".to_string(),
                version: "1.2.3".to_string(),
                signing_fingerprint: signing_fingerprint.clone(),
                session_token: token.clone(),
            }),
        )
        .await;
        let (_status, body) = refused.err().expect("mismatched session owner refused");
        assert!(body.error.contains("session owner does not match"), "{}", body.error);

        // The happy path issues an attestation that verifies under the
        // server key and pins the exact package, version, and keys.
        let response = signing(
            State(state),
            Json(SigningRequest {
                owner: "alice".to_string(),
                ident: "alice#toolbox".to_string(),
                version: "1.2.3".to_string(),
                signing_fingerprint: signing_fingerprint.clone(),
                session_token: token,
            }),
        )
        .await
        .expect("attestation issued");
        let (server_public, _server_private) = store.server_keypair().unwrap();
        let signature =
            crypto::decode_bytes(&response.0.attestation_signature, "signature").unwrap();
        crypto::verify(
            &server_public,
            &crypto::attestation_signing_input(response.0.attestation.as_bytes()),
            &signature,
        )
        .expect("attestation verifies under the server key");
        let attestation: serde_json::Value = serde_json::from_str(&response.0.attestation).unwrap();
        assert_eq!(attestation["owner"], "alice");
        assert_eq!(attestation["ident"], "alice#toolbox");
        assert_eq!(attestation["version"], "1.2.3");
        assert_eq!(attestation["signingFingerprint"], signing_fingerprint.as_str());
        assert_eq!(
            attestation["repoFingerprint"],
            crypto::fingerprint(&server_public).as_str()
        );
        let (_owner, ident_key) = store.owner_with_ident_key("alice").unwrap().unwrap();
        assert_eq!(attestation["identFingerprint"], ident_key.fingerprint.as_str());
        assert!(attestation["issued"].is_i64());
    }

    struct CraftArgs<'a> {
        ident_key_public: &'a [u8],
        proof_signer: &'a [u8],
        attestation: String,
        attestation_sig: Vec<u8>,
        version: &'a str,
    }

    /// Craft a container v1.0 package for `alice#toolbox` with a fresh one-off
    /// signing key, a proof signed by `proof_signer`, and the given
    /// attestation. Returns the artifact plus the wire request describing it.
    fn craft_package(args: CraftArgs<'_>, session_token: &str) -> PackageArtifactRequest {
        let (signing_public, signing_private) = crypto::generate_keypair();
        let ident_fingerprint = crypto::fingerprint(args.ident_key_public);
        let signing_fingerprint = crypto::fingerprint(&signing_public);
        let proof = format!(
            "{{\"owner\":\"alice\",\"ident\":\"alice#toolbox\",\"version\":\"{}\",\"identFingerprint\":\"{}\",\"signingFingerprint\":\"{}\",\"issued\":1}}",
            args.version, ident_fingerprint, signing_fingerprint,
        );
        let proof_sig = crypto::sign(
            args.proof_signer,
            &crypto::proof_signing_input(proof.as_bytes()),
        )
        .unwrap();
        let artifact = package::test_support::serialize(
            &package::test_support::TestPackage {
                name: "toolbox".to_string(),
                ident: "alice#toolbox".to_string(),
                version: args.version.to_string(),
                author: "alice".to_string(),
                payload: b"MFPCtestpayload".to_vec(),
                ident_key: format!("ed25519:{}", crypto::encode_bytes(args.ident_key_public)),
                signing_key: format!("ed25519:{}", crypto::encode_bytes(&signing_public)),
                proof,
                proof_sig,
                attestation: args.attestation,
                attestation_sig: args.attestation_sig,
            },
            &signing_private,
        );
        let parsed = package::parse_mfp_package(&artifact).expect("crafted package parses");
        PackageArtifactRequest {
            ident: parsed.ident.clone(),
            version: parsed.version.clone(),
            artifact: crypto::encode_bytes(&artifact),
            content_hash: parsed.content_hash_hex(),
            ident_fingerprint: parsed.ident_fingerprint().unwrap(),
            signing_fingerprint: parsed.signing_fingerprint().unwrap(),
            session_token: session_token.to_string(),
        }
    }

    /// Ask the real `/signing` handler for an attestation naming the given
    /// one-off fingerprint (extracted from the crafted request afterwards is
    /// impossible, so the caller passes the fingerprint a crafted package
    /// will use — here we instead attest whatever fingerprint is passed in).
    async fn real_attestation(
        state: &AppState,
        token: &str,
        version: &str,
        signing_fingerprint: &str,
    ) -> (String, Vec<u8>) {
        let response = signing(
            State(state.clone()),
            Json(SigningRequest {
                owner: "alice".to_string(),
                ident: "alice#toolbox".to_string(),
                version: version.to_string(),
                signing_fingerprint: signing_fingerprint.to_string(),
                session_token: token.to_string(),
            }),
        )
        .await
        .expect("attestation issued")
        .0;
        let signature =
            crypto::decode_bytes(&response.attestation_signature, "signature").unwrap();
        (response.attestation, signature)
    }

    #[tokio::test]
    async fn publish_chain_enforces_two_credentials_and_pinning() {
        let temp = tempfile::tempdir().unwrap();
        let opened =
            Store::open_repository(&temp.path().join("meta.db"), &temp.path().join("data"))
                .unwrap();
        let store = opened.store;
        let keys = register_owner_with_all_keys(&store, "alice");
        let token = open_session(&store, "alice", &keys.auth_private);
        let state = AppState {
            store: store.clone(),
            packages_dir: temp.path().join("data"),
        };

        // The crafted flow needs the attestation to pin the one-off key, and
        // the one-off key is generated inside craft_package. Pre-generate the
        // signing fingerprint by crafting once with a placeholder attestation
        // just to learn the fingerprint... instead, mirror the real client:
        // generate the one-off key first. craft_package generates its own, so
        // for the VALID case we call /signing with the fingerprint the craft
        // will use — achieved by crafting with a fixed rng is impossible, so
        // the valid-case craft is done inline here.
        let (signing_public, signing_private) = crypto::generate_keypair();
        let ident_fingerprint = crypto::fingerprint(&keys.ident_public);
        let signing_fingerprint = crypto::fingerprint(&signing_public);
        let (attestation, attestation_sig) =
            real_attestation(&state, &token, "1.0.0", &signing_fingerprint).await;
        let proof = format!(
            "{{\"owner\":\"alice\",\"ident\":\"alice#toolbox\",\"version\":\"1.0.0\",\"identFingerprint\":\"{}\",\"signingFingerprint\":\"{}\",\"issued\":1}}",
            ident_fingerprint, signing_fingerprint,
        );
        let proof_sig = crypto::sign(
            &keys.ident_private,
            &crypto::proof_signing_input(proof.as_bytes()),
        )
        .unwrap();
        let artifact = package::test_support::serialize(
            &package::test_support::TestPackage {
                name: "toolbox".to_string(),
                ident: "alice#toolbox".to_string(),
                version: "1.0.0".to_string(),
                author: "alice".to_string(),
                payload: b"MFPCtestpayload".to_vec(),
                ident_key: format!("ed25519:{}", crypto::encode_bytes(&keys.ident_public)),
                signing_key: format!("ed25519:{}", crypto::encode_bytes(&signing_public)),
                proof,
                proof_sig,
                attestation,
                attestation_sig,
            },
            &signing_private,
        );
        let parsed = package::parse_mfp_package(&artifact).unwrap();
        let valid_request = PackageArtifactRequest {
            ident: parsed.ident.clone(),
            version: parsed.version.clone(),
            artifact: crypto::encode_bytes(&artifact),
            content_hash: parsed.content_hash_hex(),
            ident_fingerprint: parsed.ident_fingerprint().unwrap(),
            signing_fingerprint: parsed.signing_fingerprint().unwrap(),
            session_token: token.clone(),
        };
        let report = validate_package_request(&state, &valid_request).await.unwrap();
        assert!(
            report.valid,
            "fully chained package must validate: {:?}",
            report.diagnostics
        );

        // Two-credential negative 1 — ident-only forgery: the attacker holds
        // the ident private key (valid proof) but no session, so no genuine
        // attestation exists; a self-minted one signed by the attacker's own
        // "server" key must be refused (§3.4 step 2).
        let (fake_server_public, fake_server_private) = crypto::generate_keypair();
        let fake_attestation = attestation_json(
            &crypto::fingerprint(&fake_server_public),
            "alice",
            "alice#toolbox",
            "1.0.0",
            &ident_fingerprint,
            "0000000000000000000000000000000000000000000000000000000000000000",
            1,
        );
        let fake_attestation_sig = crypto::sign(
            &fake_server_private,
            &crypto::attestation_signing_input(fake_attestation.as_bytes()),
        )
        .unwrap();
        let forged = craft_package(
            CraftArgs {
                ident_key_public: &keys.ident_public,
                proof_signer: &keys.ident_private,
                attestation: fake_attestation,
                attestation_sig: fake_attestation_sig,
                version: "1.0.0",
            },
            &token,
        );
        let report = validate_package_request(&state, &forged).await.unwrap();
        assert!(!report.valid);
        assert!(
            report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.contains("attestation verification failed")),
            "{:?}",
            report.diagnostics
        );

        // Two-credential negative 2 — auth-only forgery: the attacker has a
        // live session (real attestation) but not the ident private key, so
        // the proof cannot verify under the registered identKey (§3.4 step 6).
        let (_attacker_public, attacker_private) = crypto::generate_keypair();
        // The attestation must pin the forged package's one-off key, which
        // craft_package generates internally — so the attestation check will
        // also fail. To isolate the PROOF failure, attest the crafted
        // package's own fingerprint by crafting first with a throwaway
        // attestation, reading the fingerprint, then re-crafting is not
        // possible (fresh key each craft). Instead assert on the proof
        // diagnostic which fires regardless of the attestation result.
        let throwaway = real_attestation(&state, &token, "1.0.0", &signing_fingerprint).await;
        let forged = craft_package(
            CraftArgs {
                ident_key_public: &keys.ident_public,
                proof_signer: &attacker_private,
                attestation: throwaway.0,
                attestation_sig: throwaway.1,
                version: "1.0.0",
            },
            &token,
        );
        let report = validate_package_request(&state, &forged).await.unwrap();
        assert!(!report.valid);
        assert!(
            report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.contains("proof verification failed")),
            "{:?}",
            report.diagnostics
        );

        // Attestation reuse — a genuine attestation for 1.0.0 cannot publish
        // version 2.0.0 (§3.4 step 3 pinning).
        let (reuse_attestation, reuse_sig) =
            real_attestation(&state, &token, "1.0.0", &signing_fingerprint).await;
        let reused = craft_package(
            CraftArgs {
                ident_key_public: &keys.ident_public,
                proof_signer: &keys.ident_private,
                attestation: reuse_attestation,
                attestation_sig: reuse_sig,
                version: "2.0.0",
            },
            &token,
        );
        let report = validate_package_request(&state, &reused).await.unwrap();
        assert!(!report.valid);
        assert!(
            report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.contains("attestation")),
            "{:?}",
            report.diagnostics
        );

        // Wrong container version — hard v1.0 (index §10.2).
        let mut wrong_version = artifact.clone();
        wrong_version[10] = 9; // containerMinor = 9
        let request = PackageArtifactRequest {
            ident: valid_request.ident.clone(),
            version: valid_request.version.clone(),
            artifact: crypto::encode_bytes(&wrong_version),
            content_hash: valid_request.content_hash.clone(),
            ident_fingerprint: valid_request.ident_fingerprint.clone(),
            signing_fingerprint: valid_request.signing_fingerprint.clone(),
            session_token: token.clone(),
        };
        let report = validate_package_request(&state, &request).await.unwrap();
        assert!(!report.valid);
        assert!(
            report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.contains("unsupported MFP container version")),
            "{:?}",
            report.diagnostics
        );
    }

    /// Build a valid signed `alice#toolbox` package at `version`, publish it
    /// through the real handler, and return the artifact bytes and its hash.
    async fn publish_valid_package(
        state: &AppState,
        keys: &TestOwnerKeys,
        token: &str,
        version: &str,
    ) -> (Vec<u8>, String) {
        let (signing_public, signing_private) = crypto::generate_keypair();
        let ident_fingerprint = crypto::fingerprint(&keys.ident_public);
        let signing_fingerprint = crypto::fingerprint(&signing_public);
        let (attestation, attestation_sig) =
            real_attestation(state, token, version, &signing_fingerprint).await;
        let proof = format!(
            "{{\"owner\":\"alice\",\"ident\":\"alice#toolbox\",\"version\":\"{version}\",\"identFingerprint\":\"{ident_fingerprint}\",\"signingFingerprint\":\"{signing_fingerprint}\"}}",
        );
        let proof_sig =
            crypto::sign(&keys.ident_private, &crypto::proof_signing_input(proof.as_bytes()))
                .unwrap();
        let artifact = package::test_support::serialize(
            &package::test_support::TestPackage {
                name: "toolbox".to_string(),
                ident: "alice#toolbox".to_string(),
                version: version.to_string(),
                author: "alice".to_string(),
                payload: b"MFPCtestpayload".to_vec(),
                ident_key: format!("ed25519:{}", crypto::encode_bytes(&keys.ident_public)),
                signing_key: format!("ed25519:{}", crypto::encode_bytes(&signing_public)),
                proof,
                proof_sig,
                attestation,
                attestation_sig,
            },
            &signing_private,
        );
        let parsed = package::parse_mfp_package(&artifact).unwrap();
        let request = PackageArtifactRequest {
            ident: parsed.ident.clone(),
            version: parsed.version.clone(),
            artifact: crypto::encode_bytes(&artifact),
            content_hash: parsed.content_hash_hex(),
            ident_fingerprint: parsed.ident_fingerprint().unwrap(),
            signing_fingerprint: parsed.signing_fingerprint().unwrap(),
            session_token: token.to_string(),
        };
        let response = publish_package(State(state.clone()), Json(request))
            .await
            .expect("publish succeeds")
            .0;
        (artifact, response.hash)
    }

    #[tokio::test]
    async fn install_path_serves_blob_and_index() {
        let temp = tempfile::tempdir().unwrap();
        let opened =
            Store::open_repository(&temp.path().join("meta.db"), &temp.path().join("data"))
                .unwrap();
        let store = opened.store;
        let keys = register_owner_with_all_keys(&store, "alice");
        let token = open_session(&store, "alice", &keys.auth_private);
        let state = AppState {
            store: store.clone(),
            packages_dir: opened.packages_dir.clone(),
        };

        let (artifact, hash) = publish_valid_package(&state, &keys, &token, "1.0.0").await;

        // The blob round-trips byte-for-byte.
        let response = package_blob(State(state.clone()), axum::extract::Path(hash.clone()))
            .await
            .expect("blob served");
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(body.as_ref(), artifact.as_slice());

        // An unknown hash is a 404; a malformed hash is a 400.
        let missing = package_blob(
            State(state.clone()),
            axum::extract::Path("0".repeat(64)),
        )
        .await;
        assert_eq!(missing.err().unwrap().0, StatusCode::NOT_FOUND);
        let malformed =
            package_blob(State(state.clone()), axum::extract::Path("nothex".to_string())).await;
        assert_eq!(malformed.err().unwrap().0, StatusCode::BAD_REQUEST);

        // A corrupted stored blob is refused (blob-store corruption defense).
        std::fs::write(opened.packages_dir.join(format!("{hash}.mfp")), b"corrupt").unwrap();
        let corrupted =
            package_blob(State(state.clone()), axum::extract::Path(hash.clone())).await;
        assert_eq!(
            corrupted.err().unwrap().0,
            StatusCode::INTERNAL_SERVER_ERROR
        );

        // The index lists the version with its publish time, state, and a
        // name binding that verifies under the server key.
        let index = package_index(
            State(state.clone()),
            axum::extract::Path("alice#toolbox".to_string()),
        )
        .await
        .expect("index served")
        .0;
        assert_eq!(index.versions.len(), 1);
        assert_eq!(index.versions[0].version, "1.0.0");
        assert_eq!(index.versions[0].hash, hash);
        assert_eq!(index.versions[0].state, "available");
        assert!(index.versions[0].published_at > 0);
        assert!(index.versions[0].log_entry.is_some());
        let (server_public, _private) = store.server_keypair().unwrap();
        let ident_public = crypto::decode_bytes(
            index.ident_key.strip_prefix("ed25519:").unwrap(),
            "identKey",
        )
        .unwrap();
        assert_eq!(ident_public, keys.ident_public);
        crypto::verify(
            &server_public,
            &crypto::name_binding_message(&index.owner, &index.ident_fingerprint),
            &crypto::decode_bytes(&index.name_binding_signature, "sig").unwrap(),
        )
        .expect("name binding verifies");

        // An unknown owner/package is a 400.
        let unknown = package_index(
            State(state.clone()),
            axum::extract::Path("bob#toolbox".to_string()),
        )
        .await;
        assert_eq!(unknown.err().unwrap().0, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn release_state_requires_ident_signature_and_is_served_and_logged() {
        let temp = tempfile::tempdir().unwrap();
        let opened =
            Store::open_repository(&temp.path().join("meta.db"), &temp.path().join("data"))
                .unwrap();
        let store = opened.store;
        let keys = register_owner_with_all_keys(&store, "alice");
        let token = open_session(&store, "alice", &keys.auth_private);
        let state = AppState {
            store: store.clone(),
            packages_dir: opened.packages_dir.clone(),
        };
        publish_valid_package(&state, &keys, &token, "1.0.0").await;
        let log_before = store.log_size().unwrap();

        let sign_state = |new_state: &str| {
            crypto::encode_bytes(
                &crypto::sign(
                    &keys.ident_private,
                    &crypto::release_state_message("alice#toolbox", "1.0.0", new_state),
                )
                .unwrap(),
            )
        };

        // Auth session but no valid ident signature: refused (the ident key is
        // the authority, an auth session alone must not suffice).
        let refused = release_state(
            State(state.clone()),
            Json(ReleaseStateRequest {
                owner: "alice".to_string(),
                ident: "alice#toolbox".to_string(),
                version: "1.0.0".to_string(),
                state: "deprecated".to_string(),
                session_token: token.clone(),
                ident_signature: crypto::encode_bytes(&[0u8; 64]),
            }),
        )
        .await;
        assert!(refused.is_err());

        // An operator-only state is refused even with a valid ident signature.
        let refused = release_state(
            State(state.clone()),
            Json(ReleaseStateRequest {
                owner: "alice".to_string(),
                ident: "alice#toolbox".to_string(),
                version: "1.0.0".to_string(),
                state: "blocked".to_string(),
                session_token: token.clone(),
                ident_signature: sign_state("blocked"),
            }),
        )
        .await;
        assert!(refused.err().unwrap().0 == StatusCode::BAD_REQUEST);

        // The happy path moves the version to deprecated, appends one log
        // entry, and the index reflects it.
        let response = release_state(
            State(state.clone()),
            Json(ReleaseStateRequest {
                owner: "alice".to_string(),
                ident: "alice#toolbox".to_string(),
                version: "1.0.0".to_string(),
                state: "deprecated".to_string(),
                session_token: token.clone(),
                ident_signature: sign_state("deprecated"),
            }),
        )
        .await
        .expect("state change accepted")
        .0;
        assert_eq!(response.state, "deprecated");
        assert_eq!(store.log_size().unwrap(), log_before + 1);

        let index = package_index(
            State(state.clone()),
            axum::extract::Path("alice#toolbox".to_string()),
        )
        .await
        .expect("index served")
        .0;
        assert_eq!(index.versions[0].state, "deprecated");

        // The transition's log entry has a verifying inclusion proof.
        let leaves = store.log_leaf_hashes(None).unwrap();
        let root = crate::log::root(&leaves);
        let index_n = response.log_entry.index as usize;
        let path = crate::log::inclusion_path(index_n, &leaves);
        crate::log::verify_inclusion(index_n, leaves.len(), &leaves[index_n], &path, &root)
            .expect("release-state entry inclusion verifies");
    }

    #[tokio::test]
    async fn rotation_refuses_stale_attestations_and_serves_the_chain() {
        let temp = tempfile::tempdir().unwrap();
        let opened =
            Store::open_repository(&temp.path().join("meta.db"), &temp.path().join("data"))
                .unwrap();
        let store = opened.store;
        let keys = register_owner_with_all_keys(&store, "alice");
        let token = open_session(&store, "alice", &keys.auth_private);
        let state = AppState {
            store: store.clone(),
            packages_dir: temp.path().join("data"),
        };

        // Build a fully valid package (attestation minted pre-rotation).
        let (signing_public, signing_private) = crypto::generate_keypair();
        let ident_fingerprint = crypto::fingerprint(&keys.ident_public);
        let signing_fingerprint = crypto::fingerprint(&signing_public);
        let (attestation, attestation_sig) =
            real_attestation(&state, &token, "1.0.0", &signing_fingerprint).await;
        let proof = format!(
            "{{\"owner\":\"alice\",\"ident\":\"alice#toolbox\",\"version\":\"1.0.0\",\"identFingerprint\":\"{}\",\"signingFingerprint\":\"{}\",\"issued\":1}}",
            ident_fingerprint, signing_fingerprint,
        );
        let proof_sig = crypto::sign(
            &keys.ident_private,
            &crypto::proof_signing_input(proof.as_bytes()),
        )
        .unwrap();
        let artifact = package::test_support::serialize(
            &package::test_support::TestPackage {
                name: "toolbox".to_string(),
                ident: "alice#toolbox".to_string(),
                version: "1.0.0".to_string(),
                author: "alice".to_string(),
                payload: b"MFPCtestpayload".to_vec(),
                ident_key: format!("ed25519:{}", crypto::encode_bytes(&keys.ident_public)),
                signing_key: format!("ed25519:{}", crypto::encode_bytes(&signing_public)),
                proof: proof.clone(),
                proof_sig: proof_sig.clone(),
                attestation,
                attestation_sig,
            },
            &signing_private,
        );
        let parsed = package::parse_mfp_package(&artifact).unwrap();
        let request = PackageArtifactRequest {
            ident: parsed.ident.clone(),
            version: parsed.version.clone(),
            artifact: crypto::encode_bytes(&artifact),
            content_hash: parsed.content_hash_hex(),
            ident_fingerprint: parsed.ident_fingerprint().unwrap(),
            signing_fingerprint: parsed.signing_fingerprint().unwrap(),
            session_token: token.clone(),
        };
        let report = validate_package_request(&state, &request).await.unwrap();
        assert!(report.valid, "{:?}", report.diagnostics);

        // Rotate the ident through the handler.
        let (new_public, new_private) = crypto::generate_keypair();
        let chain_signature = crypto::sign(
            &keys.ident_private,
            &crypto::ident_rotation_message("alice", &ident_fingerprint, &new_public),
        )
        .unwrap();
        let possession_proof = crypto::sign(
            &new_private,
            &crypto::registration_message(crypto::ROLE_IDENT, "alice", &new_public),
        )
        .unwrap();
        let rotated = rotate_ident(
            State(state.clone()),
            Json(RotateRequest {
                owner: "alice".to_string(),
                new_ident_key: crypto::encode_bytes(&new_public),
                chain_signature: crypto::encode_bytes(&chain_signature),
                possession_proof: crypto::encode_bytes(&possession_proof),
                session_token: token.clone(),
            }),
        )
        .await
        .expect("rotation accepted")
        .0;
        assert_eq!(rotated.ident_fingerprint, crypto::fingerprint(&new_public));

        // §3.4 step 5 is now reachable: the same pre-rotation package (its
        // attestation names the PAST ident) is refused as stale.
        let report = validate_package_request(&state, &request).await.unwrap();
        assert!(!report.valid);
        assert!(
            report.diagnostics.iter().any(|diagnostic| diagnostic
                .contains("does not match the owner's current ident key")),
            "{:?}",
            report.diagnostics
        );

        // Refetch + rebuild under the new ident succeeds.
        let (signing_public2, signing_private2) = crypto::generate_keypair();
        let signing_fingerprint2 = crypto::fingerprint(&signing_public2);
        let new_fingerprint = crypto::fingerprint(&new_public);
        let (attestation2, attestation_sig2) =
            real_attestation(&state, &token, "1.0.0", &signing_fingerprint2).await;
        let proof2 = format!(
            "{{\"owner\":\"alice\",\"ident\":\"alice#toolbox\",\"version\":\"1.0.0\",\"identFingerprint\":\"{}\",\"signingFingerprint\":\"{}\",\"issued\":2}}",
            new_fingerprint, signing_fingerprint2,
        );
        let proof_sig2 = crypto::sign(
            &new_private,
            &crypto::proof_signing_input(proof2.as_bytes()),
        )
        .unwrap();
        let rebuilt = package::test_support::serialize(
            &package::test_support::TestPackage {
                name: "toolbox".to_string(),
                ident: "alice#toolbox".to_string(),
                version: "1.0.0".to_string(),
                author: "alice".to_string(),
                payload: b"MFPCtestpayload".to_vec(),
                ident_key: format!("ed25519:{}", crypto::encode_bytes(&new_public)),
                signing_key: format!("ed25519:{}", crypto::encode_bytes(&signing_public2)),
                proof: proof2,
                proof_sig: proof_sig2,
                attestation: attestation2,
                attestation_sig: attestation_sig2,
            },
            &signing_private2,
        );
        let reparsed = package::parse_mfp_package(&rebuilt).unwrap();
        let request = PackageArtifactRequest {
            ident: reparsed.ident.clone(),
            version: reparsed.version.clone(),
            artifact: crypto::encode_bytes(&rebuilt),
            content_hash: reparsed.content_hash_hex(),
            ident_fingerprint: reparsed.ident_fingerprint().unwrap(),
            signing_fingerprint: reparsed.signing_fingerprint().unwrap(),
            session_token: token.clone(),
        };
        let report = validate_package_request(&state, &request).await.unwrap();
        assert!(report.valid, "{:?}", report.diagnostics);

        // The chain endpoint serves the verifiable link, and a client can
        // follow it from the old pin to the new key.
        let chain = ident_chain(State(state.clone()), axum::extract::Path("alice".to_string()))
            .await
            .expect("chain served")
            .0;
        assert_eq!(chain.ident_fingerprint, crypto::fingerprint(&new_public));
        assert_eq!(chain.chain.len(), 1);
        let followed =
            crate::client::follow_ident_chain("alice", &keys.ident_public, &chain.chain)
                .unwrap()
                .expect("chain reaches a successor");
        assert_eq!(followed, new_public);

        // A re-anchor records NO chain link: following from the (rotated)
        // current key yields None — the hard-error case for consumers.
        let (anchor_public, _anchor_private) = crypto::generate_keypair();
        store.reanchor_ident("alice", &anchor_public).unwrap();
        let chain = ident_chain(State(state), axum::extract::Path("alice".to_string()))
            .await
            .expect("chain served")
            .0;
        assert_eq!(chain.ident_fingerprint, crypto::fingerprint(&anchor_public));
        assert!(
            crate::client::follow_ident_chain("alice", &new_public, &chain.chain)
                .unwrap()
                .is_none(),
            "a re-anchored ident must not be reachable through the chain"
        );
    }

    #[tokio::test]
    async fn log_endpoints_serve_verifiable_checkpoints_and_proofs() {
        let temp = tempfile::tempdir().unwrap();
        let opened =
            Store::open_repository(&temp.path().join("meta.db"), &temp.path().join("data"))
                .unwrap();
        let store = opened.store;
        let keys = register_owner_with_all_keys(&store, "alice");
        let token = open_session(&store, "alice", &keys.auth_private);
        let state = AppState {
            store: store.clone(),
            packages_dir: temp.path().join("data"),
        };
        // Grow the log: register (1) + three attestations (4 total).
        for version in ["1.0.0", "1.1.0", "1.2.0"] {
            let (signing_public, _signing_private) = crypto::generate_keypair();
            real_attestation(&state, &token, version, &crypto::fingerprint(&signing_public))
                .await;
        }
        let checkpoint_small = log_checkpoint(State(state.clone())).await.unwrap().0;
        assert_eq!(checkpoint_small.size, 4);

        // The checkpoint signature verifies under the server key.
        let (server_public, _private) = store.server_keypair().unwrap();
        let root = hex::decode(&checkpoint_small.root_hash).unwrap();
        let mut root32 = [0u8; 32];
        root32.copy_from_slice(&root);
        crypto::verify(
            &server_public,
            &crate::log::checkpoint_signing_input(checkpoint_small.size as u64, &root32),
            &crypto::decode_bytes(&checkpoint_small.signature, "signature").unwrap(),
        )
        .expect("checkpoint signature verifies");

        // Every entry has a verifying inclusion proof against the head.
        for index in 0..checkpoint_small.size {
            let proof = log_inclusion_proof(
                State(state.clone()),
                axum::extract::Path(index),
                axum::extract::Query(ProofQuery { size: None }),
            )
            .await
            .unwrap()
            .0;
            let leaf = {
                let raw = hex::decode(&proof.leaf_hash).unwrap();
                let mut leaf = [0u8; 32];
                leaf.copy_from_slice(&raw);
                leaf
            };
            let path: Vec<[u8; 32]> = proof
                .path
                .iter()
                .map(|node| {
                    let raw = hex::decode(node).unwrap();
                    let mut out = [0u8; 32];
                    out.copy_from_slice(&raw);
                    out
                })
                .collect();
            crate::log::verify_inclusion(
                index as usize,
                checkpoint_small.size as usize,
                &leaf,
                &path,
                &root32,
            )
            .unwrap_or_else(|err| panic!("index {index}: {err}"));
        }

        // Append more entries; the consistency proof ties old head to new.
        let (signing_public, _signing_private) = crypto::generate_keypair();
        real_attestation(&state, &token, "2.0.0", &crypto::fingerprint(&signing_public)).await;
        let checkpoint_big = log_checkpoint(State(state.clone())).await.unwrap().0;
        assert_eq!(checkpoint_big.size, 5);
        let proof = log_consistency_proof(
            State(state.clone()),
            axum::extract::Query(ConsistencyQuery {
                from: checkpoint_small.size,
                to: None,
            }),
        )
        .await
        .unwrap()
        .0;
        let new_root = {
            let raw = hex::decode(&checkpoint_big.root_hash).unwrap();
            let mut out = [0u8; 32];
            out.copy_from_slice(&raw);
            out
        };
        let path: Vec<[u8; 32]> = proof
            .path
            .iter()
            .map(|node| {
                let raw = hex::decode(node).unwrap();
                let mut out = [0u8; 32];
                out.copy_from_slice(&raw);
                out
            })
            .collect();
        crate::log::verify_consistency(
            checkpoint_small.size as usize,
            checkpoint_big.size as usize,
            &root32,
            &new_root,
            &path,
        )
        .expect("consistency proof verifies");
    }

    #[test]
    fn jwt_verification_rejects_expired_wrong_signature_and_unknown_session() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("meta.db");
        let data_path = temp.path().join("data");
        let opened = Store::open_repository(&db_path, &data_path).unwrap();
        let store = opened.store;
        register_owner_with_keys(&store, "alice");
        let (owner, key) = store.owner_with_auth_key("alice").unwrap().unwrap();
        let now = crate::store::now_unix();

        let expired_jti = Uuid::new_v4().to_string();
        let expired_claims = SessionClaims {
            sub: "alice".to_string(),
            owner_id: owner.id,
            auth_fingerprint: key.fingerprint.clone(),
            iat: now - 7200,
            exp: now - 3600,
            jti: expired_jti.clone(),
        };
        let expired_token = encode(
            &Header::new(Algorithm::HS256),
            &expired_claims,
            &EncodingKey::from_secret(&store.server_secret().unwrap()),
        )
        .unwrap();
        store
            .insert_session(&NewSession {
                owner_id: owner.id,
                key_id: key.id,
                jwt_id: expired_jti,
                issued_at: now - 7200,
                expires_at: now - 3600,
            })
            .unwrap();
        assert!(verify_session_token(&store, &expired_token)
            .unwrap_err()
            .contains("expired or malformed"));

        let unknown_claims = SessionClaims {
            sub: "alice".to_string(),
            owner_id: owner.id,
            auth_fingerprint: key.fingerprint.clone(),
            iat: now,
            exp: now + 3600,
            jti: Uuid::new_v4().to_string(),
        };
        let unknown_token = encode(
            &Header::new(Algorithm::HS256),
            &unknown_claims,
            &EncodingKey::from_secret(&store.server_secret().unwrap()),
        )
        .unwrap();
        assert!(verify_session_token(&store, &unknown_token)
            .unwrap_err()
            .contains("unknown session"));

        let wrong_signature_token = encode(
            &Header::new(Algorithm::HS256),
            &unknown_claims,
            &EncodingKey::from_secret(b"wrong-secret"),
        )
        .unwrap();
        assert!(verify_session_token(&store, &wrong_signature_token)
            .unwrap_err()
            .contains("expired or malformed"));
    }
}
