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
    #[serde(rename = "logEntry")]
    pub log_entry: String,
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
    let Some((_owner, key)) = state
        .store
        .owner_with_auth_key(&request.owner)
        .map_err(internal)?
    else {
        return Err(bad_request("unknown owner".to_string()));
    };
    if key.fingerprint != request.auth_fingerprint {
        return Err(bad_request("mismatched local key fingerprint".to_string()));
    }
    let challenge = state
        .store
        .create_challenge(&request.owner)
        .map_err(conflict_or_bad_request)?;
    Ok(Json(ChallengeResponse {
        challenge_id: challenge.id,
        nonce: crypto::encode_bytes(&challenge.nonce),
        expires_at: challenge.expires_at,
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
    let Some((owner, key)) = state
        .store
        .owner_with_auth_key(&request.owner)
        .map_err(internal)?
    else {
        return Err(bad_request("unknown owner".to_string()));
    };
    if owner.id != claims.owner_id || key.fingerprint != claims.auth_fingerprint {
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
    let blob_stored = !path.exists();
    if blob_stored {
        std::fs::write(&path, &artifact)
            .map_err(|err| internal(format!("failed to write package blob: {err}")))?;
    }
    let owner_id = verify_session_token(&state.store, &request.session_token)
        .map_err(bad_request)?
        .owner_id;
    let published = state
        .store
        .publish_package_version(
            owner_id,
            &request.ident,
            &request.version,
            &hash,
            &path.to_string_lossy(),
        )
        .map_err(conflict_or_bad_request)?;
    Ok(Json(PublishPackageResponse {
        ident: published.ident,
        version: published.version,
        hash: published.hash,
        published_at: published.published_at,
        state: published.state,
        blob_stored,
        log_entry: format!("publish:{}", Uuid::new_v4()),
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
    let Some((owner, key)) = state
        .store
        .owner_with_auth_key(owner_part)
        .map_err(internal)?
    else {
        diagnostics.push("package ident owner is not registered".to_string());
        return Ok(invalid_report(hash, diagnostics));
    };
    if owner.id != claims.owner_id || key.fingerprint != claims.auth_fingerprint {
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

    Ok(ValidatePackageResponse {
        valid: diagnostics.is_empty(),
        content_hash: hash,
        abi_index: serde_json::json!({}),
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

    fn register_owner_with_keys(store: &Store, owner: &str) -> (Vec<u8>, Vec<u8>) {
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
        (auth_public, auth_private)
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
