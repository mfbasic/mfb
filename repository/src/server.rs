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

#[derive(Debug, Serialize, Deserialize)]
pub struct SigningInfoRequest {
    pub owner: String,
    #[serde(rename = "sessionToken")]
    pub session_token: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SigningInfoResponse {
    pub owner: String,
    #[serde(rename = "identKey")]
    pub ident_key: String,
    #[serde(rename = "identFingerprint")]
    pub ident_fingerprint: String,
    #[serde(rename = "signingKey")]
    pub signing_key: String,
    #[serde(rename = "signingFingerprint")]
    pub signing_fingerprint: String,
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
        .route("/keys/signing", post(signing_info))
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

async fn signing_info(
    State(state): State<AppState>,
    Json(request): Json<SigningInfoRequest>,
) -> Result<Json<SigningInfoResponse>, (StatusCode, Json<ErrorResponse>)> {
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
            "session key does not match current signing key".to_string(),
        ));
    }
    let public_key = crypto::encode_bytes(&key.public_key);
    Ok(Json(SigningInfoResponse {
        owner: owner.owner_display,
        ident_key: public_key.clone(),
        ident_fingerprint: key.fingerprint.clone(),
        signing_key: public_key,
        signing_fingerprint: key.fingerprint,
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
    if request.ident_fingerprint != package.ident_fingerprint {
        diagnostics.push("request identFingerprint does not match package metadata".to_string());
    }
    if request.signing_fingerprint != package.signing_fingerprint {
        diagnostics.push("request signingFingerprint does not match package metadata".to_string());
    }

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
    let current_public_key = crypto::encode_bytes(&key.public_key);
    if package.ident_key != format!("ed25519:{current_public_key}") {
        diagnostics.push("package identKey does not match current owner ident key".to_string());
    }
    if package.ident_fingerprint != key.fingerprint {
        diagnostics.push("package identFingerprint does not match current owner ident key".to_string());
    }
    if package.signing_fingerprint != key.fingerprint {
        diagnostics.push("package signingFingerprint does not match current owner signing key".to_string());
    }
    if package.author != owner.owner_display {
        diagnostics.push("package author does not match owner name".to_string());
    }
    if let Err(err) = package::verify_package_signature(&package, &key.public_key) {
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
