use crate::crypto;
use crate::store::{now_unix, NewSession, Store};
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use tokio::net::TcpListener;
use uuid::Uuid;

#[derive(Clone)]
pub struct AppState {
    store: Store,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RegisterRequest {
    pub owner: String,
    #[serde(rename = "authKey")]
    pub auth_key: String,
    pub proof: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RegisterResponse {
    pub owner: String,
    #[serde(rename = "authFingerprint")]
    pub auth_fingerprint: String,
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

pub async fn serve(store: Store, listen: SocketAddr) -> Result<SocketAddr, String> {
    let state = AppState { store };
    let app = Router::new()
        .route("/health", get(health))
        .route("/accounts/register", post(register))
        .route("/auth/challenge", post(challenge))
        .route("/auth/login", post(login))
        .route("/keys/signing", post(signing_info))
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

async fn register(
    State(state): State<AppState>,
    Json(request): Json<RegisterRequest>,
) -> Result<Json<RegisterResponse>, (StatusCode, Json<ErrorResponse>)> {
    let public = crypto::decode_bytes(&request.auth_key, "authKey").map_err(bad_request)?;
    let proof = crypto::decode_bytes(&request.proof, "proof").map_err(bad_request)?;
    let (owner, key) = state
        .store
        .register_owner(&request.owner, &public, &proof)
        .map_err(conflict_or_bad_request)?;
    Ok(Json(RegisterResponse {
        owner: owner.owner_display,
        auth_fingerprint: key.fingerprint,
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

    #[test]
    fn jwt_creation_sets_expected_claims_and_verifies() {
        let temp = tempfile::tempdir().unwrap();
        let opened = Store::open_repository(temp.path()).unwrap();
        let store = opened.store;
        let (public, private) = crypto::generate_keypair();
        let message = crypto::registration_message("alice", &public);
        let proof = crypto::sign(&private, &message).unwrap();
        store.register_owner("alice", &public, &proof).unwrap();

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
        let opened = Store::open_repository(temp.path()).unwrap();
        let store = opened.store;
        let (public, private) = crypto::generate_keypair();
        let message = crypto::registration_message("alice", &public);
        let proof = crypto::sign(&private, &message).unwrap();
        let (owner, key) = store.register_owner("alice", &public, &proof).unwrap();
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
