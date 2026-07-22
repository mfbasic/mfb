use crate::blobstore::{BlobFetch, BlobKind, BlobStore};
use crate::store::{now_unix, NewSession, Store};
use crate::{crypto, package};
use axum::extract::{ConnectInfo, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
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
    blob_store: BlobStore,
    rate_limiter: RateLimiter,
}

/// A minimal in-memory sliding-window rate limiter (plan-10-D2). Keyed per
/// endpoint (and, where meaningful, per owner), it caps abusive bursts on the
/// cheap, loggable operations (register/challenge/login/signing) without a
/// dependency. Approximate and process-local — a real deployment fronts this
/// with a proxy — but enough to keep the transparency log spam-free.
#[derive(Clone)]
struct RateLimiter {
    hits: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, Vec<i64>>>>,
}

impl RateLimiter {
    fn new() -> Self {
        Self {
            hits: std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
        }
    }

    /// Record a hit for `key`; return false if it exceeded `max` hits within
    /// the last `window_secs`.
    fn allow(&self, key: &str, max: usize, window_secs: i64) -> bool {
        let now = now_unix();
        // Recover a poisoned lock rather than panicking every subsequent call: a
        // panic while this lock was held must not wedge rate limiting for the
        // whole process (bug-264 / REPO-09). The hit map is plain data with no
        // cross-field invariant a mid-panic leaves broken.
        let mut hits = self
            .hits
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let entry = hits.entry(key.to_string()).or_default();
        entry.retain(|timestamp| now - *timestamp < window_secs);
        if entry.len() >= max {
            return false;
        }
        entry.push(now);
        true
    }

    /// Drop keys whose windows have fully elapsed, so the map stays bounded.
    fn prune(&self, window_secs: i64) {
        let now = now_unix();
        // Recover a poisoned lock rather than panicking every subsequent call: a
        // panic while this lock was held must not wedge rate limiting for the
        // whole process (bug-264 / REPO-09). The hit map is plain data with no
        // cross-field invariant a mid-panic leaves broken.
        let mut hits = self
            .hits
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        hits.retain(|_key, times| {
            times.retain(|timestamp| now - *timestamp < window_secs);
            !times.is_empty()
        });
    }
}

fn too_many_requests() -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::TOO_MANY_REQUESTS,
        Json(ErrorResponse {
            error: "rate limit exceeded; slow down".to_string(),
        }),
    )
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

/// Signed-metadata responses (plan-10-C2). Each carries the exact signed JSON
/// string (`signed`) plus a signature the client verifies over those bytes.
#[derive(Debug, Serialize, Deserialize)]
pub struct RootResponse {
    /// The root-signed `root.json` bytes (verified under `rootKey`).
    pub signed: String,
    pub signature: String,
    #[serde(rename = "rootKey")]
    pub root_key: String,
    #[serde(rename = "rootFingerprint")]
    pub root_fingerprint: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SignedMetadataResponse {
    pub signed: String,
    pub signature: String,
}

/// Org membership change (plan-10-D1): an owner/admin member — or the org
/// itself for the first grant — sets a member's role. Ident-authorized + a
/// live session, like every account mutation.
#[derive(Debug, Serialize, Deserialize)]
pub struct OrgMemberRequest {
    pub org: String,
    /// The member account whose role is being set/removed by the grantor's ident.
    pub grantor: String,
    pub member: String,
    /// `owner`, `admin`, or `publisher` (ignored when `action` is `remove`).
    pub role: String,
    /// `grant` or `remove`.
    pub action: String,
    #[serde(rename = "sessionToken")]
    pub session_token: String,
    #[serde(rename = "identSignature")]
    pub ident_signature: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OrgMemberResponse {
    pub org: String,
    pub member: String,
    pub role: String,
}

/// Publish-token issuance (plan-10-D1): a scoped, TTL-bounded auth key for CI.
#[derive(Debug, Serialize, Deserialize)]
pub struct TokenIssueRequest {
    pub owner: String,
    #[serde(rename = "tokenKey")]
    pub token_key: String,
    pub proof: String,
    /// `<owner>#<package>` or `<owner>#*` — the packages this token may attest.
    pub scope: String,
    #[serde(rename = "ttlSeconds")]
    pub ttl_seconds: i64,
    #[serde(rename = "sessionToken")]
    pub session_token: String,
    #[serde(rename = "identSignature")]
    pub ident_signature: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TokenIssueResponse {
    pub owner: String,
    #[serde(rename = "tokenFingerprint")]
    pub token_fingerprint: String,
    pub scope: String,
    #[serde(rename = "expiresAt")]
    pub expires_at: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TokenRevokeRequest {
    pub owner: String,
    #[serde(rename = "tokenFingerprint")]
    pub token_fingerprint: String,
    #[serde(rename = "sessionToken")]
    pub session_token: String,
    #[serde(rename = "identSignature")]
    pub ident_signature: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TokenRevokeResponse {
    pub owner: String,
    #[serde(rename = "tokenFingerprint")]
    pub token_fingerprint: String,
    pub revoked: bool,
}

/// Two-sided ownership transfer (plan-10-D1). Both halves are ident-signed and
/// logged; already-published versions keep verifying against the old ident.
#[derive(Debug, Serialize, Deserialize)]
pub struct TransferOfferRequest {
    pub ident: String,
    #[serde(rename = "fromOwner")]
    pub from_owner: String,
    #[serde(rename = "toOwner")]
    pub to_owner: String,
    #[serde(rename = "sessionToken")]
    pub session_token: String,
    #[serde(rename = "identSignature")]
    pub ident_signature: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TransferAcceptRequest {
    pub ident: String,
    #[serde(rename = "toOwner")]
    pub to_owner: String,
    #[serde(rename = "sessionToken")]
    pub session_token: String,
    #[serde(rename = "identSignature")]
    pub ident_signature: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TransferResponse {
    pub ident: String,
    #[serde(rename = "toOwner")]
    pub to_owner: String,
    pub accepted: bool,
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
    /// Warn-only typosquat notices (plan-10-D2): existing idents within edit
    /// distance 1 of the published one. Never blocks the publish.
    #[serde(default)]
    pub warnings: Vec<String>,
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

/// `GET /packages/<owner>#<package>` (plan-61-B): the public, anonymous package
/// view. Distinct from `IndexResponse`, which is the install path's contract and
/// is deliberately left untouched.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PackageDetailResponse {
    pub ident: String,
    pub owner: String,
    #[serde(rename = "identKey")]
    pub ident_key: String,
    #[serde(rename = "identFingerprint")]
    pub ident_fingerprint: String,
    #[serde(rename = "serverFingerprint")]
    pub server_fingerprint: String,
    pub author: Option<String>,
    pub url: Option<String>,
    /// `null` until plan-61-E populates it. Present in the shape from day one
    /// so E adds no field and breaks no consumer.
    pub description: Option<String>,
    #[serde(rename = "latestVersion")]
    pub latest_version: Option<String>,
    /// **Every** version, newest first, including yanked and superseded — see
    /// `PackageDetailVersionResponse::state`.
    pub versions: Vec<PackageDetailVersionResponse>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PackageDetailVersionResponse {
    pub version: String,
    pub hash: String,
    #[serde(rename = "publishedAt")]
    pub published_at: i64,
    /// The release state as a value to render, never as a filter the server
    /// applied: omitting non-current versions would reproduce the SUP-03
    /// truncation this surface exists to make observable.
    pub state: String,
    #[serde(rename = "abiIndex")]
    pub abi_index: serde_json::Value,
    #[serde(rename = "logEntry")]
    pub log_entry: Option<LogEntry>,
    pub targets: Vec<PackageTargetResponse>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PackageTargetResponse {
    pub os: String,
    /// `null` is the any-arch wildcard — the locator matches every
    /// architecture on its OS — not missing data.
    pub arch: Option<String>,
    pub libc: Option<String>,
    #[serde(rename = "libType")]
    pub lib_type: String,
    pub logical: String,
    pub source: String,
    #[serde(rename = "blobHash")]
    pub blob_hash: Option<String>,
}

/// `GET /packages/<ident>/audit` (plan-61-B §4): the transparency record.
///
/// The publish entries carry an **inclusion proof**, not just an index. A
/// rendered `logEntry` number that cannot be independently verified proves
/// nothing; the proof is what lets a third-party monitor catch a registry
/// showing different histories to different clients.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PackageAuditResponse {
    pub ident: String,
    pub owner: String,
    #[serde(rename = "logCheckpoint")]
    pub log_checkpoint: CheckpointResponse,
    pub publishes: Vec<AuditPublishEntry>,
    #[serde(rename = "stateChanges")]
    pub state_changes: Vec<AuditStateChange>,
    #[serde(rename = "identChain")]
    pub ident_chain: Vec<AuditIdentRotation>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AuditPublishEntry {
    pub version: String,
    pub index: i64,
    #[serde(rename = "leafHash")]
    pub leaf_hash: String,
    /// Sibling hashes proving this leaf is in the checkpoint above.
    pub proof: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AuditStateChange {
    pub version: String,
    pub state: String,
    pub at: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AuditIdentRotation {
    #[serde(rename = "oldKey")]
    pub old_key: String,
    #[serde(rename = "newKey")]
    pub new_key: String,
    pub signature: String,
    pub issued: i64,
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
#[derive(Debug, Serialize, Deserialize, Clone)]
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

/// JWT issuer/audience binding (REPO-04). A session token is minted by this
/// issuer for this registry's session audience, and verification requires the
/// exact pair — so a token minted for a different service or audience (even one
/// signed with the same HS256 secret) cannot be replayed against the session
/// endpoints here.
const SESSION_TOKEN_ISSUER: &str = "mfb-repo";
const SESSION_TOKEN_AUDIENCE: &str = "mfb-repo/session";

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SessionClaims {
    pub sub: String,
    pub owner_id: i64,
    pub auth_fingerprint: String,
    pub iat: i64,
    pub exp: i64,
    pub jti: String,
    /// Issuer (REPO-04) — always `SESSION_TOKEN_ISSUER`; validated on decode.
    pub iss: String,
    /// Audience (REPO-04) — always `SESSION_TOKEN_AUDIENCE`; validated on decode.
    pub aud: String,
}

/// Maximum inline request body (plan-10-D2): caps the base64 artifact carried
/// by `/validate` and `/publish` so a single upload cannot exhaust memory.
const MAX_BODY_BYTES: usize = 64 * 1024 * 1024;

/// Per-client (peer-IP) rate caps on the anonymous auth endpoints, replacing the
/// old global-string buckets that let one client lock the whole user base out of
/// registration/login (audit-2 REPO-12 / bug-188). A generous global ceiling is
/// kept as a secondary backstop so extreme aggregate abuse is still bounded, but
/// it is high enough never to trip on one abuser's traffic.
const REGISTER_PER_IP_MAX: usize = 20;
const LOGIN_PER_IP_MAX: usize = 30;
const AUTH_GLOBAL_CEILING: usize = 2000;
/// Per-IP sliding-window cap on `GET /search` (plan-61-B Phase 2).
///
/// Keyed on the **peer IP**, following `REGISTER_PER_IP_MAX` /
/// `LOGIN_PER_IP_MAX`, because `/search` is anonymous and has no `claims.sub`
/// to key on the way `BLOB_UPLOAD_PER_OWNER_MAX` does. Higher than the auth
/// caps because a search box legitimately issues many requests, and search is
/// the only route in this sub-plan that does real query work per call.
const SEARCH_PER_IP_MAX: usize = 120;
/// Server-side ceiling on `?limit`. An uncapped limit on an anonymous
/// enumerate route is a trivial resource-exhaustion lever, so an over-cap
/// request is **clamped**, never honoured and never rejected.
const SEARCH_LIMIT_MAX: i64 = 50;
const SEARCH_LIMIT_DEFAULT: i64 = 20;
/// How much of a description a search result carries (plan-61-E §Open
/// Decisions).
///
/// Clamped on a **character** boundary, not a byte one: descriptions are UTF-8
/// and a byte clamp can split a multi-byte character. Without a clamp, a page
/// of 50 results could carry 50 × 4096 bytes of description on an anonymous
/// route — the same resource lever the `limit` cap exists to close.
const SEARCH_DESCRIPTION_PREVIEW_CHARS: usize = 200;

/// Clamp a description to the search-result preview length, appending an
/// ellipsis when it was actually shortened.
fn description_preview(description: Option<String>) -> Option<String> {
    description.map(|text| {
        if text.chars().count() <= SEARCH_DESCRIPTION_PREVIEW_CHARS {
            return text;
        }
        let mut preview: String = text
            .chars()
            .take(SEARCH_DESCRIPTION_PREVIEW_CHARS)
            .collect();
        preview.push('…');
        preview
    })
}
/// Per-owner sliding-window caps on the authenticated package endpoints, whose
/// only prior protection was the shared 64 MiB body cap — a registered (near
/// anonymous) client could hammer `/validate` (5 Ed25519 verifies/call) for CPU
/// or `/publish` for permanent disk (audit-2 REPO-13 / bug-188).
const VALIDATE_PER_OWNER_MAX: usize = 60;
const PUBLISH_PER_OWNER_MAX: usize = 30;
/// Per-owner sliding-window cap on native-blob uploads (plan-48-A §4.3).
/// Non-optional: without it an authenticated publisher can fill the datapath
/// with 64 MiB objects that nothing can ever reclaim (the registry has no GC).
/// A 7-slot binding is 7 PUTs, so this must comfortably clear one binding's
/// full target set while still bounding a flood.
const BLOB_UPLOAD_PER_OWNER_MAX: usize = 120;
/// Per-owner published-version quota: the total number of `package_versions`
/// rows an owner may accumulate. Bounds permanent blob/DB growth from an
/// authenticated flood without constraining any realistic publisher.
const MAX_VERSIONS_PER_OWNER: i64 = 10_000;

// coverage:off — serve() binds a real TCP listener, spawns the background
// reaper task, and runs the axum accept loop; none of that is reachable under a
// unit test. The individual route handlers it wires up are tested directly by
// constructing AppState and calling them, which is where the request/response
// logic lives.
pub async fn serve(
    store: Store,
    blob_store: BlobStore,
    listen: SocketAddr,
) -> Result<SocketAddr, String> {
    let state = AppState {
        store,
        blob_store,
        rate_limiter: RateLimiter::new(),
    };
    // Background reaper (plan-10-D2): sweep expired challenges/sessions/pairing
    // blobs and prune the rate-limiter map so nothing accumulates unbounded.
    {
        let reaper_state = state.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                let _ = reaper_state.store.reap_expired();
                reaper_state.rate_limiter.prune(3600);
            }
        });
    }
    let app = build_router(state);
    let listener = TcpListener::bind(listen)
        .await
        .map_err(|err| format!("failed to bind {listen}: {err}"))?;
    let actual = listener
        .local_addr()
        .map_err(|err| format!("failed to read listening address: {err}"))?;
    println!("MFB_REPO_LISTEN={actual}");
    // `into_make_service_with_connect_info` exposes each connection's peer
    // `SocketAddr` to handlers via `ConnectInfo`, so register/login can throttle
    // per client IP instead of one shared global bucket (bug-188 / REPO-12).
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .map_err(|err| format!("repository server failed: {err}"))?;
    Ok(actual)
}
// coverage:on

/// Assemble the route table.
///
/// Split out of `serve` so it can be constructed in a test without binding a
/// listener. That matters for more than tidiness: `Router::route` **panics on a
/// route conflict at construction time**, so a bad table is a dead server at
/// startup rather than a failing handler test. `GET /packages/:ident` puts a
/// parameter segment beside the static `/packages/transfer/*` routes, and
/// `router_has_no_route_conflicts` is what proves matchit resolves that the way
/// this table assumes (static wins over param).
pub fn build_router(state: AppState) -> Router {
    Router::new()
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
        // plan-61-C: the HTML surface. All GET, all anonymous, all carrying
        // the CSP from `web::html_response`.
        .route("/", get(landing_page))
        .route("/style.css", get(stylesheet))
        .route("/search.html", get(search_html))
        .route("/p/:ident", get(package_page_html))
        .route("/p/:ident/audit", get(package_audit_html))
        .route("/search", get(search))
        .route("/index/:ident", get(package_index))
        // plan-61-B: anonymous read surface. These read no credential of any
        // kind — no session token, no bearer header, no cookie — and are `GET`
        // only. `:ident` sits beside the static `/packages/transfer/*` routes
        // below; matchit resolves static before param.
        .route("/packages/:ident", get(package_detail))
        .route("/packages/:ident/audit", get(package_audit))
        .route(
            "/blob/:hash",
            get(package_blob).head(head_blob).put(put_blob),
        )
        .route("/release-state", post(release_state))
        .route("/orgs/members", post(org_members))
        .route("/tokens", post(issue_token))
        .route("/tokens/revoke", post(revoke_token))
        .route("/packages/transfer/offer", post(transfer_offer))
        .route("/packages/transfer/accept", post(transfer_accept))
        .route("/root.json", get(root_metadata))
        .route("/snapshot.json", get(snapshot_metadata))
        .route("/timestamp.json", get(timestamp_metadata))
        .route("/validate", post(validate_package))
        .route("/publish", post(publish_package))
        .layer(axum::extract::DefaultBodyLimit::max(MAX_BODY_BYTES))
        .with_state(state)
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
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Json(request): Json<RegisterRequest>,
) -> Result<Json<RegisterResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Per-client bucket keyed by peer IP (bug-188 / REPO-12): a single abuser can
    // no longer lock every user out of registration by spending one shared global
    // bucket. The global ceiling below is a much higher secondary backstop.
    if !state
        .rate_limiter
        .allow(&format!("register:{}", peer.ip()), REGISTER_PER_IP_MAX, 60)
        || !state
            .rate_limiter
            .allow("register", AUTH_GLOBAL_CEILING, 60)
    {
        return Err(too_many_requests());
    }
    let auth_key = crypto::decode_bytes(&request.auth_key, "authKey").map_err(bad_request)?;
    let ident_key = crypto::decode_bytes(&request.ident_key, "identKey").map_err(bad_request)?;
    let auth_proof =
        crypto::decode_bytes(&request.proofs.auth, "auth proof").map_err(bad_request)?;
    let ident_proof =
        crypto::decode_bytes(&request.proofs.ident, "ident proof").map_err(bad_request)?;
    let (owner, auth, ident) = state
        .store
        .register_owner(
            &request.owner,
            &auth_key,
            &auth_proof,
            &ident_key,
            &ident_proof,
        )
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
    if !state
        .rate_limiter
        .allow(&format!("challenge:{}", request.owner), 20, 60)
    {
        return Err(too_many_requests());
    }
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
struct SearchQuery {
    q: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SearchResponse {
    pub query: String,
    /// The number of results **on this page**, not a whole-corpus count. Named
    /// `total` to match the shape plan-61-B §3 published; a true corpus count
    /// would need a second unbounded query on an anonymous route.
    pub total: usize,
    pub results: Vec<SearchResult>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SearchResult {
    pub ident: String,
    pub owner: String,
    #[serde(rename = "latestVersion")]
    pub latest_version: Option<String>,
    /// `null` until plan-61-E.
    pub description: Option<String>,
    #[serde(rename = "publishedAt")]
    pub published_at: Option<i64>,
}

/// `GET /search?q=&limit=&offset=` — anonymous, read-only (plan-61-B Phase 2).
///
/// Reads no credential. `limit` is clamped to `SEARCH_LIMIT_MAX` server-side,
/// and an empty or whitespace-only `q` returns an empty result set rather than
/// the whole table.
async fn search(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    axum::extract::Query(query): axum::extract::Query<SearchQuery>,
) -> Result<Json<SearchResponse>, (StatusCode, Json<ErrorResponse>)> {
    if !state
        .rate_limiter
        .allow(&format!("search:{}", peer.ip()), SEARCH_PER_IP_MAX, 60)
    {
        return Err(too_many_requests());
    }
    let text = query.q.unwrap_or_default();
    // Clamp rather than reject: an over-cap `limit` is a client that wants more
    // than it may have, not an error, and rejecting it would make paging
    // brittle for no security gain.
    let limit = query
        .limit
        .unwrap_or(SEARCH_LIMIT_DEFAULT)
        .clamp(0, SEARCH_LIMIT_MAX);
    let offset = query.offset.unwrap_or(0).max(0);

    let results: Vec<SearchResult> = state
        .store
        .search_packages(&text, limit, offset)
        .map_err(internal)?
        .into_iter()
        .map(|row| SearchResult {
            ident: row.ident,
            owner: row.owner,
            latest_version: row.latest_version,
            description: description_preview(row.description),
            published_at: row.published_at,
        })
        .collect();

    Ok(Json(SearchResponse {
        query: text,
        total: results.len(),
        results,
    }))
}

/// The registry id and root fingerprint the HTML pages display.
///
/// Both are `None` before `mfb-repo init-root` runs, and the landing page
/// simply omits the fingerprint block rather than showing a placeholder — a
/// blank or fabricated fingerprint in the one section whose entire purpose is
/// "compare this exactly" is worse than no section at all.
fn registry_identity(state: &AppState) -> (String, Option<String>) {
    match state.store.registry_config() {
        Ok(Some(config)) => (
            config.registry_id,
            Some(crypto::fingerprint(&config.root_public)),
        ),
        _ => ("(uninitialized)".to_string(), None),
    }
}

/// `GET /style.css` — the stylesheet, compiled into the binary.
///
/// A real route rather than an inline `<style>` block because the CSP is
/// `style-src 'self'` with no `'unsafe-inline'`; and `include_str!` rather than
/// a `ServeDir` so the server stays a single self-contained binary.
async fn stylesheet() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=3600"),
        ],
        crate::web::STYLESHEET,
    )
}

/// `GET /` — the landing page (plan-61-C Phase 2).
async fn landing_page(State(state): State<AppState>) -> Response {
    let (registry_id, root_fingerprint) = registry_identity(&state);
    crate::web::html_response(
        StatusCode::OK,
        crate::web::landing(&registry_id, root_fingerprint.as_deref()),
    )
}

/// `GET /search.html?q=` — the rendered search page (plan-61-C Phase 2).
///
/// Shares `search_packages` with the JSON route, including its server-side
/// `limit` cap, so the HTML surface cannot be used to enumerate more than the
/// API allows.
async fn search_html(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    axum::extract::Query(query): axum::extract::Query<SearchQuery>,
) -> Response {
    let (registry_id, _fingerprint) = registry_identity(&state);
    if !state
        .rate_limiter
        .allow(&format!("search:{}", peer.ip()), SEARCH_PER_IP_MAX, 60)
    {
        return crate::web::html_response(
            StatusCode::TOO_MANY_REQUESTS,
            crate::web::message_page(
                &registry_id,
                "Too many searches",
                "This client has made too many searches in a short window. Wait a \
                 minute and try again.",
            ),
        );
    }
    let text = query.q.unwrap_or_default();
    let limit = query
        .limit
        .unwrap_or(SEARCH_LIMIT_DEFAULT)
        .clamp(0, SEARCH_LIMIT_MAX);
    let offset = query.offset.unwrap_or(0).max(0);

    let rows = match state.store.search_packages(&text, limit, offset) {
        Ok(rows) => rows,
        Err(_err) => {
            return crate::web::html_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                crate::web::message_page(
                    &registry_id,
                    "Search failed",
                    "The registry could not complete this search.",
                ),
            )
        }
    };
    let rows: Vec<crate::web::SearchRow> = rows
        .into_iter()
        .map(|row| crate::web::SearchRow {
            ident: row.ident,
            owner: row.owner,
            latest_version: row.latest_version,
            description: description_preview(row.description),
            published_at: row.published_at,
        })
        .collect();

    // A query that matches nothing is 200 with a "no results" page: the request
    // succeeded and the answer is "none", which is not the same as "that page
    // does not exist".
    crate::web::html_response(
        StatusCode::OK,
        crate::web::search_page(&registry_id, &text, &rows),
    )
}

/// `GET /p/:ident` — the rendered package page (plan-61-C Phase 3).
///
/// Renders the *same* `package_detail` handler output the JSON route serves, so
/// the two surfaces cannot disagree about what a package's history is — which
/// is the whole point of a transparency view.
async fn package_page_html(
    State(state): State<AppState>,
    axum::extract::Path(ident): axum::extract::Path<String>,
) -> Response {
    let (registry_id, _fingerprint) = registry_identity(&state);
    let detail =
        match package_detail(State(state.clone()), axum::extract::Path(ident.clone())).await {
            Ok(Json(detail)) => detail,
            Err((status, Json(error))) => {
                return crate::web::html_response(
                    status,
                    crate::web::message_page(&registry_id, "Package not found", &error.error),
                )
            }
        };

    let view = crate::web::PackageView {
        ident: detail.ident,
        owner: detail.owner,
        ident_key: detail.ident_key,
        ident_fingerprint: detail.ident_fingerprint,
        server_fingerprint: detail.server_fingerprint,
        author: detail.author,
        url: detail.url,
        description: detail.description,
        latest_version: detail.latest_version,
        versions: detail
            .versions
            .into_iter()
            .map(|version| crate::web::VersionRow {
                version: version.version,
                hash: version.hash,
                published_at: version.published_at,
                state: version.state,
                abi_symbols: version
                    .abi_index
                    .as_object()
                    .map(|map| map.len())
                    .unwrap_or(0),
                log_index: version.log_entry.map(|entry| entry.index),
                targets: version
                    .targets
                    .into_iter()
                    .map(|target| crate::web::TargetRow {
                        os: target.os,
                        arch: target.arch,
                        libc: target.libc,
                        lib_type: target.lib_type,
                        logical: target.logical,
                        source: target.source,
                        blob_hash: target.blob_hash,
                    })
                    .collect(),
            })
            .collect(),
    };
    crate::web::html_response(
        StatusCode::OK,
        crate::web::package_page(&registry_id, &view),
    )
}

/// `GET /p/:ident/audit` — the rendered transparency tab (plan-61-C Phase 3).
async fn package_audit_html(
    State(state): State<AppState>,
    axum::extract::Path(ident): axum::extract::Path<String>,
) -> Response {
    let (registry_id, _fingerprint) = registry_identity(&state);
    let audit = match package_audit(State(state.clone()), axum::extract::Path(ident.clone())).await
    {
        Ok(Json(audit)) => audit,
        Err((status, Json(error))) => {
            return crate::web::html_response(
                status,
                crate::web::message_page(&registry_id, "Package not found", &error.error),
            )
        }
    };

    let view = crate::web::AuditView {
        ident: audit.ident,
        checkpoint_size: audit.log_checkpoint.size,
        checkpoint_root: audit.log_checkpoint.root_hash,
        checkpoint_signature: audit.log_checkpoint.signature,
        publishes: audit
            .publishes
            .into_iter()
            .map(|entry| crate::web::AuditPublish {
                version: entry.version,
                index: entry.index,
                leaf_hash: entry.leaf_hash,
                proof: entry.proof,
            })
            .collect(),
        state_changes: audit
            .state_changes
            .into_iter()
            .map(|change| (change.version, change.state, change.at))
            .collect(),
        ident_chain: audit
            .ident_chain
            .into_iter()
            .map(|rotation| {
                (
                    rotation.old_key,
                    rotation.new_key,
                    rotation.signature,
                    rotation.issued,
                )
            })
            .collect(),
    };
    crate::web::html_response(StatusCode::OK, crate::web::audit_page(&registry_id, &view))
}

/// Split `<owner>#<package>`, or a 400 naming the expected shape.
///
/// axum percent-decodes a `Path` segment before the handler sees it, so a
/// browser-safe `%23` arrives here as a literal `#`. `package_index` has always
/// relied on that; these routes mirror it rather than re-deriving it.
fn split_ident(ident: &str) -> Result<(&str, &str), (StatusCode, Json<ErrorResponse>)> {
    match ident.split_once('#') {
        Some((owner, package)) if !owner.is_empty() && !package.is_empty() => Ok((owner, package)),
        _ => Err(bad_request("ident must use <owner>#<package>".to_string())),
    }
}

/// `GET /packages/:ident` — anonymous, read-only (plan-61-B Phase 1).
///
/// Reads **no** credential: no `sessionToken`, no `Authorization` header, no
/// cookie. A request carrying one behaves identically to one that does not,
/// which is asserted by test rather than left as a property of the signature.
async fn package_detail(
    State(state): State<AppState>,
    axum::extract::Path(ident): axum::extract::Path<String>,
) -> Result<Json<PackageDetailResponse>, (StatusCode, Json<ErrorResponse>)> {
    let (owner_part, _package_part) = split_ident(&ident)?;
    let Some(detail) = state.store.package_detail(&ident).map_err(internal)? else {
        // One 404 for "no such package", whatever the reason. Distinguishing
        // "unknown owner" from "unknown package under a known owner" would
        // turn this route into an owner-enumeration oracle.
        return Err(not_found("unknown package".to_string()));
    };
    let Some((owner_record, ident_key)) = state
        .store
        .owner_with_ident_key(owner_part)
        .map_err(internal)?
    else {
        return Err(not_found("unknown package".to_string()));
    };

    // The package-level metadata shown is the **newest** version's. Versions
    // are ordered newest-first, and an older version's author/url describes
    // that older artifact, not the package as it stands today. Read before the
    // loop below consumes the rows.
    let (author, url, description) = match detail.versions.first() {
        Some(newest) => (
            newest.author.clone(),
            newest.url.clone(),
            newest.description.clone(),
        ),
        None => (None, None, None),
    };

    let mut versions = Vec::new();
    for row in detail.versions {
        let log_entry = state
            .store
            .publish_log_entry(&ident, &row.version)
            .map_err(internal)?
            .map(|entry| LogEntry {
                index: entry.index,
                leaf_hash: hex::encode(entry.leaf_hash),
            });
        versions.push(PackageDetailVersionResponse {
            version: row.version,
            hash: row.hash,
            published_at: row.published_at,
            state: row.state,
            abi_index: serde_json::from_str(&row.abi_index)
                .unwrap_or_else(|_| serde_json::json!({})),
            log_entry,
            targets: row
                .targets
                .into_iter()
                .map(|target| PackageTargetResponse {
                    os: target.os,
                    arch: target.arch,
                    libc: target.libc,
                    lib_type: target.lib_type,
                    logical: target.logical,
                    source: target.source,
                    blob_hash: target.blob_hash,
                })
                .collect(),
        });
    }

    let (server_public, _server_private) = state.store.server_keypair().map_err(internal)?;
    Ok(Json(PackageDetailResponse {
        latest_version: versions.first().map(|version| version.version.clone()),
        ident,
        owner: owner_record.owner_display,
        ident_key: format!("ed25519:{}", crypto::encode_bytes(&ident_key.public_key)),
        ident_fingerprint: ident_key.fingerprint,
        server_fingerprint: crypto::fingerprint(&server_public),
        author,
        url,
        description,
        versions,
    }))
}

/// `GET /packages/:ident/audit` — anonymous, read-only (plan-61-B §4).
///
/// Every publish entry carries its inclusion proof against the checkpoint
/// returned in the same response, so a third-party monitor can verify the
/// history rather than take the registry's word for it.
async fn package_audit(
    State(state): State<AppState>,
    axum::extract::Path(ident): axum::extract::Path<String>,
) -> Result<Json<PackageAuditResponse>, (StatusCode, Json<ErrorResponse>)> {
    let (owner_part, _package_part) = split_ident(&ident)?;
    let Some(audit) = state.store.package_audit(&ident).map_err(internal)? else {
        return Err(not_found("unknown package".to_string()));
    };

    let leaves = state.store.log_leaf_hashes(None).map_err(internal)?;
    let root = crate::log::root(&leaves);
    let (_public, private) = state.store.server_keypair().map_err(internal)?;
    let signature = crypto::sign(
        &private,
        &crate::log::checkpoint_signing_input(leaves.len() as u64, &root),
    )
    .map_err(internal)?;

    let mut publishes = Vec::new();
    for version in audit.versions {
        let Some(entry) = state
            .store
            .publish_log_entry(&ident, &version)
            .map_err(internal)?
        else {
            continue;
        };
        // A proof is only meaningful against the checkpoint in this same
        // response, so both are computed from the one `leaves` snapshot.
        let proof = if entry.index >= 0 && (entry.index as usize) < leaves.len() {
            crate::log::inclusion_path(entry.index as usize, &leaves)
                .into_iter()
                .map(hex::encode)
                .collect()
        } else {
            Vec::new()
        };
        publishes.push(AuditPublishEntry {
            version,
            index: entry.index,
            leaf_hash: hex::encode(entry.leaf_hash),
            proof,
        });
    }

    let ident_chain = state
        .store
        .ident_chain(owner_part)
        .map_err(internal)?
        .into_iter()
        .map(|(old_key, new_key, signature, issued)| AuditIdentRotation {
            old_key: format!("ed25519:{}", crypto::encode_bytes(&old_key)),
            new_key: format!("ed25519:{}", crypto::encode_bytes(&new_key)),
            signature: crypto::encode_bytes(&signature),
            issued,
        })
        .collect();

    Ok(Json(PackageAuditResponse {
        ident,
        owner: audit.owner,
        log_checkpoint: CheckpointResponse {
            size: leaves.len() as i64,
            root_hash: hex::encode(root),
            signature: crypto::encode_bytes(&signature),
        },
        publishes,
        state_changes: audit
            .state_changes
            .into_iter()
            .map(|change| AuditStateChange {
                version: change.version,
                state: change.state,
                at: change.at,
            })
            .collect(),
        ident_chain,
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
        return Err(bad_request(
            "no publish log entry for that package".to_string(),
        ));
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
    for row in state
        .store
        .list_package_versions(&ident)
        .map_err(internal)?
    {
        let log_entry = state
            .store
            .publish_log_entry(&ident, &row.version)
            .map_err(internal)?
            .map(|entry| LogEntry {
                index: entry.index,
                leaf_hash: hex::encode(entry.leaf_hash),
            });
        let abi_index =
            serde_json::from_str(&row.abi_index).unwrap_or_else(|_| serde_json::json!({}));
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

/// Snapshot metadata lifetime (plan-10-C2): the snapshot pins the index state
/// for a week; the timestamp refreshes daily and pins the current snapshot.
const SNAPSHOT_TTL_SECS: i64 = 7 * 24 * 3600;
const TIMESTAMP_TTL_SECS: i64 = 24 * 3600;

/// Shared preamble for ident-authorized account mutations (plan-10-D1): verify
/// the session names `owner` and matches a current auth key, and return the
/// owner record plus their current ident public key (for ident-signature
/// verification). An auth session alone can never mutate account state.
fn session_and_ident(
    state: &AppState,
    owner: &str,
    session_token: &str,
) -> Result<(crate::store::OwnerRecord, Vec<u8>), (StatusCode, Json<ErrorResponse>)> {
    let claims = verify_session_token(&state.store, session_token).map_err(bad_request)?;
    if claims.sub != owner {
        return Err(bad_request(
            "session owner does not match requested owner".to_string(),
        ));
    }
    let Some((owner_record, _key)) = state
        .store
        .owner_auth_key_by_fingerprint(owner, &claims.auth_fingerprint)
        .map_err(internal)?
    else {
        return Err(bad_request(
            "session key is not a current auth key".to_string(),
        ));
    };
    if owner_record.id != claims.owner_id {
        return Err(bad_request(
            "session owner does not match requested owner".to_string(),
        ));
    }
    let Some((_owner, ident_key)) = state.store.owner_with_ident_key(owner).map_err(internal)?
    else {
        return Err(bad_request("owner has no current ident key".to_string()));
    };
    Ok((owner_record, ident_key.public_key))
}

/// `POST /orgs/members` (plan-10-D1): grant or remove a member's org role. The
/// grantor must be the org itself (bootstrap) or an owner/admin member, and the
/// change is authorized by the grantor's ident signature and logged.
async fn org_members(
    State(state): State<AppState>,
    Json(request): Json<OrgMemberRequest>,
) -> Result<Json<OrgMemberResponse>, (StatusCode, Json<ErrorResponse>)> {
    let (_grantor, grantor_ident) =
        session_and_ident(&state, &request.grantor, &request.session_token)?;
    let role_in_message = if request.action == "remove" {
        "removed"
    } else {
        &request.role
    };
    let signature =
        crypto::decode_bytes(&request.ident_signature, "identSignature").map_err(bad_request)?;
    crypto::verify(
        &grantor_ident,
        &crypto::org_role_message(&request.org, &request.member, role_in_message),
        &signature,
    )
    .map_err(|_| bad_request("invalid org role ident signature".to_string()))?;

    // Authority: the org itself (first grant) or an owner/admin member.
    let is_org = crate::validation::fold_owner(&request.grantor)
        == crate::validation::fold_owner(&request.org);
    let grantor_role = state
        .store
        .org_member_role(&request.org, &request.grantor)
        .map_err(internal)?;
    if !is_org && !matches!(grantor_role.as_deref(), Some("owner") | Some("admin")) {
        return Err(bad_request(
            "grantor must be the org or an owner/admin member".to_string(),
        ));
    }

    if request.action == "remove" {
        state
            .store
            .remove_org_member(&request.org, &request.member)
            .map_err(bad_request)?;
        return Ok(Json(OrgMemberResponse {
            org: request.org,
            member: request.member,
            role: "removed".to_string(),
        }));
    }
    state
        .store
        .grant_org_member(&request.org, &request.member, &request.role)
        .map_err(bad_request)?;
    Ok(Json(OrgMemberResponse {
        org: request.org,
        member: request.member,
        role: request.role,
    }))
}

/// `POST /tokens` (plan-10-D1): issue a scoped, TTL-bounded publish token —
/// ident-authorized and logged. The token can request attestations only within
/// its scope and never bypasses the ident-proof requirement.
async fn issue_token(
    State(state): State<AppState>,
    Json(request): Json<TokenIssueRequest>,
) -> Result<Json<TokenIssueResponse>, (StatusCode, Json<ErrorResponse>)> {
    let (_owner, ident_public) = session_and_ident(&state, &request.owner, &request.session_token)?;
    let token_public = crypto::decode_bytes(&request.token_key, "tokenKey").map_err(bad_request)?;
    let proof = crypto::decode_bytes(&request.proof, "proof").map_err(bad_request)?;
    let token_fingerprint = crypto::fingerprint(&token_public);
    let signature =
        crypto::decode_bytes(&request.ident_signature, "identSignature").map_err(bad_request)?;
    crypto::verify(
        &ident_public,
        &crypto::token_issue_message(&request.owner, &token_fingerprint, &request.scope),
        &signature,
    )
    .map_err(|_| bad_request("invalid token issuance ident signature".to_string()))?;
    // Scope must belong to the issuing owner.
    if !scope_owner_matches(&request.scope, &request.owner) {
        return Err(bad_request(
            "token scope must be within the issuing owner".to_string(),
        ));
    }
    let (owner, key, expires_at) = state
        .store
        .issue_publish_token(
            &request.owner,
            &token_public,
            &proof,
            &request.scope,
            request.ttl_seconds,
        )
        .map_err(bad_request)?;
    Ok(Json(TokenIssueResponse {
        owner: owner.owner_display,
        token_fingerprint: key.fingerprint,
        scope: request.scope,
        expires_at,
    }))
}

/// `POST /tokens/revoke` (plan-10-D1): revoke a publish token — ident-authorized
/// and logged; its sessions are closed.
async fn revoke_token(
    State(state): State<AppState>,
    Json(request): Json<TokenRevokeRequest>,
) -> Result<Json<TokenRevokeResponse>, (StatusCode, Json<ErrorResponse>)> {
    let (_owner, ident_public) = session_and_ident(&state, &request.owner, &request.session_token)?;
    let signature =
        crypto::decode_bytes(&request.ident_signature, "identSignature").map_err(bad_request)?;
    crypto::verify(
        &ident_public,
        &crypto::token_revoke_message(&request.owner, &request.token_fingerprint),
        &signature,
    )
    .map_err(|_| bad_request("invalid token revocation ident signature".to_string()))?;
    let revoked = state
        .store
        .revoke_publish_token(&request.owner, &request.token_fingerprint)
        .map_err(bad_request)?;
    if !revoked {
        return Err(bad_request(
            "no active token with that fingerprint".to_string(),
        ));
    }
    Ok(Json(TokenRevokeResponse {
        owner: request.owner,
        token_fingerprint: request.token_fingerprint,
        revoked: true,
    }))
}

/// `POST /packages/transfer/offer` (plan-10-D1): the current owner offers a
/// package to a recipient, authorized by the current owner's ident.
async fn transfer_offer(
    State(state): State<AppState>,
    Json(request): Json<TransferOfferRequest>,
) -> Result<Json<TransferResponse>, (StatusCode, Json<ErrorResponse>)> {
    let (_owner, ident_public) =
        session_and_ident(&state, &request.from_owner, &request.session_token)?;
    let signature =
        crypto::decode_bytes(&request.ident_signature, "identSignature").map_err(bad_request)?;
    crypto::verify(
        &ident_public,
        &crypto::transfer_offer_message(&request.ident, &request.from_owner, &request.to_owner),
        &signature,
    )
    .map_err(|_| bad_request("invalid transfer offer ident signature".to_string()))?;
    state
        .store
        .create_transfer_offer(&request.ident, &request.from_owner, &request.to_owner)
        .map_err(bad_request)?;
    Ok(Json(TransferResponse {
        ident: request.ident,
        to_owner: request.to_owner,
        accepted: false,
    }))
}

/// `POST /packages/transfer/accept` (plan-10-D1): the recipient accepts a
/// pending offer; the package is re-bound to them and both halves are logged.
async fn transfer_accept(
    State(state): State<AppState>,
    Json(request): Json<TransferAcceptRequest>,
) -> Result<Json<TransferResponse>, (StatusCode, Json<ErrorResponse>)> {
    let (_owner, ident_public) =
        session_and_ident(&state, &request.to_owner, &request.session_token)?;
    let signature =
        crypto::decode_bytes(&request.ident_signature, "identSignature").map_err(bad_request)?;
    crypto::verify(
        &ident_public,
        &crypto::transfer_accept_message(&request.ident, &request.to_owner),
        &signature,
    )
    .map_err(|_| bad_request("invalid transfer accept ident signature".to_string()))?;
    state
        .store
        .accept_transfer(&request.ident, &request.to_owner)
        .map_err(bad_request)?;
    Ok(Json(TransferResponse {
        ident: request.ident,
        to_owner: request.to_owner,
        accepted: true,
    }))
}

/// Whether a token `scope` (`<owner>#<package>` or `<owner>#*`) belongs to
/// `owner`.
fn scope_owner_matches(scope: &str, owner: &str) -> bool {
    scope
        .split_once('#')
        .map(|(scope_owner, _)| {
            crate::validation::fold_owner(scope_owner) == crate::validation::fold_owner(owner)
        })
        .unwrap_or(false)
}

/// Whether a token `scope` permits attesting `ident` (`<owner>#<package>`).
/// `<owner>#*` matches any package of that owner; otherwise an exact match.
fn scope_permits(scope: &str, ident: &str) -> bool {
    if let Some((scope_owner, scope_pkg)) = scope.split_once('#') {
        if let Some((ident_owner, _)) = ident.split_once('#') {
            let owner_ok = crate::validation::fold_owner(scope_owner)
                == crate::validation::fold_owner(ident_owner);
            return owner_ok && (scope_pkg == "*" || scope == ident);
        }
    }
    false
}

/// `GET /root.json` (plan-10-C2): the offline-root-signed metadata delegating
/// the online server/snapshot/timestamp keys. A client pins the root
/// fingerprint out of band and verifies every other key chains from it.
async fn root_metadata(
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
async fn snapshot_metadata(
    State(state): State<AppState>,
) -> Result<Json<SignedMetadataResponse>, (StatusCode, Json<ErrorResponse>)> {
    let Some(config) = state.store.registry_config().map_err(internal)? else {
        return Err(metadata_uninitialized());
    };
    let version = state.store.log_size().map_err(internal)?;
    let index_hash = state.store.index_canonical_hash().map_err(internal)?;
    let leaves = state.store.log_leaf_hashes(None).map_err(internal)?;
    let checkpoint_root = hex::encode(crate::log::root(&leaves));
    let signed = format!(
        "{{\"type\":\"snapshot\",\"registryId\":{},\"version\":{},\"expires\":{},\"indexHash\":{},\"checkpoint\":{{\"size\":{},\"rootHash\":{}}}}}",
        json_str(&config.registry_id),
        version,
        now_unix() + SNAPSHOT_TTL_SECS,
        json_str(&index_hash),
        leaves.len(),
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
async fn timestamp_metadata(
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

/// `POST /release-state` (plan-10-C1): a maintainer moves a published version
/// between `available`/`deprecated`/`yanked`. Requires a live session AND an
/// ident signature — an auth session alone is refused.
async fn release_state(
    State(state): State<AppState>,
    Json(request): Json<ReleaseStateRequest>,
) -> Result<Json<ReleaseStateResponse>, (StatusCode, Json<ErrorResponse>)> {
    let claims = verify_session_token(&state.store, &request.session_token).map_err(bad_request)?;
    if claims.sub != request.owner {
        return Err(bad_request(
            "session owner does not match requested owner".to_string(),
        ));
    }
    let Some((owner, _key)) = state
        .store
        .owner_auth_key_by_fingerprint(&request.owner, &claims.auth_fingerprint)
        .map_err(internal)?
    else {
        return Err(bad_request(
            "session key is not a current auth key".to_string(),
        ));
    };
    if owner.id != claims.owner_id {
        return Err(bad_request(
            "session owner does not match requested owner".to_string(),
        ));
    }
    // Maintainer states only — blocked/legal-tombstoned are operator states.
    if !matches!(
        request.state.as_str(),
        "available" | "deprecated" | "yanked"
    ) {
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
        return Err(bad_request(
            "ident owner does not match session owner".to_string(),
        ));
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

/// `GET /blob/<hash>` (plan-10-A): serve the content-addressed `<hash>.mfp`
/// blob. The local backend streams the bytes inline, re-checking the recomputed
/// hash as a blob-store corruption defense. The S3 backend answers with a `302`
/// redirect to a short-lived presigned URL, so the bytes never transit the app
/// server; the client re-hashes what it downloads, so the integrity check moves
/// to the client on that path.
/// Validate a `/blob/<hash>` path component: 64 lowercase hex characters.
/// Shared by `GET`, `HEAD`, and `PUT` so all three reject malformed hashes
/// identically.
fn validate_blob_hash(hash: &str) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
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

async fn package_blob(
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
async fn head_blob(
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
async fn put_blob(
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

async fn rotate_ident(
    State(state): State<AppState>,
    Json(request): Json<RotateRequest>,
) -> Result<Json<RotateResponse>, (StatusCode, Json<ErrorResponse>)> {
    let claims = verify_session_token(&state.store, &request.session_token).map_err(bad_request)?;
    if claims.sub != request.owner {
        return Err(bad_request(
            "session owner does not match requested owner".to_string(),
        ));
    }
    let Some((owner, _key)) = state
        .store
        .owner_auth_key_by_fingerprint(&request.owner, &claims.auth_fingerprint)
        .map_err(internal)?
    else {
        return Err(bad_request(
            "session key is not a current auth key".to_string(),
        ));
    };
    if owner.id != claims.owner_id {
        return Err(bad_request(
            "session owner does not match requested owner".to_string(),
        ));
    }
    let new_public =
        crypto::decode_bytes(&request.new_ident_key, "newIdentKey").map_err(bad_request)?;
    let chain_signature =
        crypto::decode_bytes(&request.chain_signature, "chainSignature").map_err(bad_request)?;
    let possession_proof =
        crypto::decode_bytes(&request.possession_proof, "possessionProof").map_err(bad_request)?;
    let (owner, new_key) = state
        .store
        .rotate_ident(
            &request.owner,
            &new_public,
            &chain_signature,
            &possession_proof,
        )
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
    let Some((owner_record, ident_key)) =
        state.store.owner_with_ident_key(&owner).map_err(internal)?
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
        return Err(bad_request(
            "session owner does not match requested owner".to_string(),
        ));
    }
    let Some((owner, _key)) = state
        .store
        .owner_auth_key_by_fingerprint(&request.owner, &claims.auth_fingerprint)
        .map_err(internal)?
    else {
        return Err(bad_request(
            "session key is not a current auth key".to_string(),
        ));
    };
    if owner.id != claims.owner_id {
        return Err(bad_request(
            "session owner does not match requested owner".to_string(),
        ));
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
    let message =
        crypto::registration_message(crypto::ROLE_AUTH, &owner_record.owner_display, &auth_key);
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
        .complete_revocation_challenge(&request.challenge_id, &signature, &request.auth_fingerprint)
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
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Json(request): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Per-client bucket keyed by peer IP (bug-188 / REPO-12): invalid attempts
    // count (the check precedes signature decode), but they now only exhaust the
    // attacker's own bucket, not a global one shared with every legitimate user.
    if !state
        .rate_limiter
        .allow(&format!("login:{}", peer.ip()), LOGIN_PER_IP_MAX, 60)
        || !state.rate_limiter.allow("login", AUTH_GLOBAL_CEILING, 60)
    {
        return Err(too_many_requests());
    }
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
        iss: SESSION_TOKEN_ISSUER.to_string(),
        aud: SESSION_TOKEN_AUDIENCE.to_string(),
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
    if !state
        .rate_limiter
        .allow(&format!("signing:{}", request.owner), 60, 60)
    {
        return Err(too_many_requests());
    }
    let claims = verify_session_token(&state.store, &request.session_token).map_err(bad_request)?;
    if claims.sub != request.owner {
        return Err(bad_request(
            "session owner does not match requested owner".to_string(),
        ));
    }
    let Some((owner, key)) = state
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
    // If the session's auth key is a scoped publish token (plan-10-D1), it may
    // only attest packages within its scope, and only until it expires.
    if let Some((scope, expires_at, revoked_at)) = state
        .store
        .publish_token_for_key(key.id)
        .map_err(internal)?
    {
        if revoked_at.is_some() {
            return Err(bad_request("publish token is revoked".to_string()));
        }
        if expires_at <= now_unix() {
            return Err(bad_request("publish token has expired".to_string()));
        }
        if !scope_permits(&scope, &request.ident) {
            return Err(bad_request(
                "publish token scope does not permit this package".to_string(),
            ));
        }
    }
    // The attestation pins one exact package+version: the ident must belong
    // to the session owner and the one-off key fingerprint must be
    // well-formed before the server puts its name on them.
    let Some((ident_owner, package_part)) = request.ident.split_once('#') else {
        return Err(bad_request("ident must use <owner>#<package>".to_string()));
    };
    if crate::validation::fold_owner(ident_owner) != crate::validation::fold_owner(&request.owner) {
        return Err(bad_request(
            "ident owner does not match session owner".to_string(),
        ));
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
    let report =
        validate_package_request(&state, &request, "validate", VALIDATE_PER_OWNER_MAX).await?;
    Ok(Json(report))
}

async fn publish_package(
    State(state): State<AppState>,
    Json(request): Json<PackageArtifactRequest>,
) -> Result<Json<PublishPackageResponse>, (StatusCode, Json<ErrorResponse>)> {
    let report =
        validate_package_request(&state, &request, "publish", PUBLISH_PER_OWNER_MAX).await?;
    if !report.valid {
        return Err(bad_request(format!(
            "package validation failed: {}",
            report.diagnostics.join("; ")
        )));
    }
    // Per-owner version quota (bug-188 / REPO-13): bound the permanent blob/DB
    // growth an authenticated flood can inflict. Checked before staging the blob
    // so a rejected publish leaves nothing behind. A re-publish of an existing
    // (ident, version) row updates in place and is not newly counted below.
    let quota_owner_id = verify_session_token(&state.store, &request.session_token)
        .map_err(bad_request)?
        .owner_id;
    let version_count = state
        .store
        .owner_version_count(quota_owner_id)
        .map_err(internal)?;
    if version_count >= MAX_VERSIONS_PER_OWNER
        && !state
            .store
            .package_version_exists(&request.ident, &request.version)
            .map_err(internal)?
    {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            Json(ErrorResponse {
                error: format!("per-owner version quota of {MAX_VERSIONS_PER_OWNER} reached"),
            }),
        ));
    }
    let artifact = crypto::decode_bytes(&request.artifact, "artifact").map_err(bad_request)?;
    let hash = report.content_hash;
    // The vendor locators section 10 names — recorded as version→blob edges so
    // a future GC (plan-49) can compute reachability (plan-48-A §4.5), and as
    // the native target matrix (plan-61-A §3). Their existence was already
    // enforced by the validation above; re-parse the already-verified payload
    // rather than threading the list through `report`.
    //
    // Keep the whole `VendorBlobRef`, not just `.hash`: the platform axis is
    // the metadata the target matrix is made of, and collapsing to hashes here
    // is what used to throw it away.
    let parsed = package::parse_mfp_package(&artifact).ok();
    let vendor_blobs: Vec<crate::abi::VendorBlobRef> = parsed
        .as_ref()
        .and_then(|package| crate::abi::parse_vendor_blobs(&package.payload).ok())
        .unwrap_or_default();
    // plan-61-A §4: render what the publisher *signed*. `author`/`url` exist
    // twice — in the plaintext header, and interned in MANIFEST section 1 inside
    // the signed payload — and only the second is covered by the signature. Take
    // the signed copy, and refuse a package whose two copies disagree rather
    // than silently preferring either: a mismatch is a malformed or tampered
    // artifact, and quietly picking one is how a registry ends up displaying
    // something nobody signed.
    let manifest_metadata = match parsed.as_ref() {
        Some(package) => {
            let signed = crate::abi::parse_manifest_metadata(&package.payload)
                .map_err(|err| bad_request(format!("failed to read package manifest: {err}")))?;
            if let Some(signed) = signed.as_ref() {
                if signed.author != package.author || signed.url != package.url {
                    return Err(bad_request(format!(
                        "package header and signed manifest disagree: header author {:?} url {:?}, \
                         manifest author {:?} url {:?}",
                        package.author, package.url, signed.author, signed.url,
                    )));
                }
            }
            signed
        }
        None => None,
    };
    // An empty string means the publisher set nothing; store NULL rather than
    // '' so "not provided" and "provided as empty" stay one fact, not two.
    // plan-61-E: the description rides in MFPC section 18, added by plan-61-D.
    // A package built before that carries no section 18 at all, which is a
    // normal outcome and stays NULL — not an error and not a warning.
    let description = parsed
        .as_ref()
        .and_then(|package| crate::abi::parse_package_description(&package.payload).ok())
        .flatten()
        .filter(|value| !value.is_empty());
    let publish_metadata = match manifest_metadata {
        Some(meta) => crate::store::PublishMetadata {
            author: Some(meta.author).filter(|value| !value.is_empty()),
            url: Some(meta.url).filter(|value| !value.is_empty()),
            description,
        },
        None => crate::store::PublishMetadata {
            description,
            ..Default::default()
        },
    };
    let already_present = state
        .blob_store
        .exists(&hash, BlobKind::Package)
        .await
        .map_err(internal)?;
    // Blob-ordering fix (plan-10-A §2.6): stage the blob, commit the DB row, and
    // only then promote it to servable — a failed transaction leaves no orphan
    // blob, and a served blob always has a committed version row. The blob
    // backend (local file or S3 object) implements the stage/promote/abort
    // protocol.
    let staged = if already_present {
        None
    } else {
        Some(
            state
                .blob_store
                .stage(&hash, BlobKind::Package, artifact)
                .await
                .map_err(internal)?,
        )
    };
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
        &state.blob_store.blob_ref(&hash, BlobKind::Package),
        &abi_index,
        &vendor_blobs,
        &publish_metadata,
    ) {
        Ok(published) => published,
        Err(err) => {
            if let Some(staged) = staged {
                state.blob_store.abort(staged).await;
            }
            return Err(conflict_or_bad_request(err));
        }
    };
    let blob_stored = if let Some(staged) = staged {
        state.blob_store.promote(staged).await.map_err(internal)?;
        true
    } else {
        false
    };
    // Warn-only typosquat check (plan-10-D2): surface near-duplicate idents so
    // the publisher can notice an impersonation attempt; never blocks.
    let warnings = state
        .store
        .typosquat_candidates(&published.ident)
        .unwrap_or_default()
        .into_iter()
        .map(|existing| {
            format!(
                "published `{}` is one edit away from existing `{existing}`",
                published.ident
            )
        })
        .collect();
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
        warnings,
    }))
}

async fn validate_package_request(
    state: &AppState,
    request: &PackageArtifactRequest,
    route: &str,
    per_owner_max: usize,
) -> Result<ValidatePackageResponse, (StatusCode, Json<ErrorResponse>)> {
    let claims = verify_session_token(&state.store, &request.session_token).map_err(bad_request)?;
    // Per-owner sliding-window throttle on the expensive authenticated routes
    // (bug-188 / REPO-13). Registration is open, so "authenticated" is near
    // anonymous; without this a single owner could hammer /validate's Ed25519
    // verifies or /publish's permanent blob writes. Keyed per route so a publish
    // flood does not exhaust the validate budget and vice-versa.
    if !state
        .rate_limiter
        .allow(&format!("{route}:{}", claims.sub), per_owner_max, 60)
    {
        return Err(too_many_requests());
    }
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
    match state
        .store
        .owner_with_ident_key(owner_part)
        .map_err(internal)?
    {
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

    // plan-48-A §4.4: every `vendor` locator named by section 10 must already
    // have its blob uploaded, so a successful publish can never leave a
    // section-10 hash dangling. Applied on both /validate (a dry run reports
    // missing blobs before the publisher uploads anything) and /publish (which
    // funnels through this function). The section rides inside the signed+welded
    // payload, so these hashes are authenticated by re-hash alone (§3.2).
    match crate::abi::parse_vendor_blobs(&package.payload) {
        Ok(vendor_blobs) => {
            // Probe each distinct hash once (bug-275). A library legitimately
            // lists the same vendored file for several platforms, so without
            // dedup every repeat costs another backend round trip; the parser's
            // locator cap bounds the count, and this bounds the duplicates.
            let mut probed: std::collections::HashMap<&str, bool> =
                std::collections::HashMap::new();
            for vref in &vendor_blobs {
                if !probed.contains_key(vref.hash.as_str()) {
                    let exists = state
                        .blob_store
                        .exists(&vref.hash, BlobKind::Native)
                        .await
                        .map_err(internal)?;
                    probed.insert(vref.hash.as_str(), exists);
                }
            }
            // Diagnostics stay per-locator so a missing blob still names every
            // logical library and source filename that referenced it.
            for vref in &vendor_blobs {
                if !probed.get(vref.hash.as_str()).copied().unwrap_or(false) {
                    diagnostics.push(format!(
                        "native library '{}' references vendor blob {} ({}) that is not uploaded",
                        vref.logical, vref.hash, vref.source
                    ));
                }
            }
        }
        Err(err) => diagnostics.push(format!("native library table is malformed: {err}")),
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
    // REPO-04: bind the token to this issuer/audience. `set_audience` also adds
    // `aud` to the required claims, so a token missing either binding is rejected.
    validation.set_issuer(&[SESSION_TOKEN_ISSUER]);
    validation.set_audience(&[SESSION_TOKEN_AUDIENCE]);
    let decoded = decode::<SessionClaims>(token, &DecodingKey::from_secret(&secret), &validation)
        .map_err(|_| "expired or malformed session token".to_string())?;
    if !store.session_exists(&decoded.claims.jti)? {
        return Err("unknown session token".to_string());
    }
    Ok(decoded.claims)
}

fn bad_request(message: String) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::BAD_REQUEST,
        Json(ErrorResponse { error: message }),
    )
}

fn conflict_or_bad_request(message: String) -> (StatusCode, Json<ErrorResponse>) {
    if message.contains("already in use") || message.contains("reused challenge") {
        (StatusCode::CONFLICT, Json(ErrorResponse { error: message }))
    } else {
        bad_request(message)
    }
}

/// 404 with the standard error shape. Used by the plan-61-B read routes, which
/// answer "no such package" for an unknown owner and an unknown package alike
/// rather than letting a caller tell the two apart.
fn not_found(message: String) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::NOT_FOUND,
        Json(ErrorResponse { error: message }),
    )
}

fn internal(message: String) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse { error: message }),
    )
}

fn unauthorized(message: &str) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::UNAUTHORIZED,
        Json(ErrorResponse {
            error: message.to_string(),
        }),
    )
}

/// Extract the bearer token from an `Authorization: Bearer <token>` header —
/// the one header-borne credential on this server (plan-48-A §4.3), used by
/// `PUT /blob/<hash>` because a raw-body request cannot carry the body-field
/// `sessionToken` every other authenticated route uses.
fn bearer_token(
    headers: &axum::http::HeaderMap,
) -> Result<&str, (StatusCode, Json<ErrorResponse>)> {
    let value = headers
        .get(axum::http::header::AUTHORIZATION)
        .ok_or_else(|| unauthorized("missing Authorization header"))?;
    let value = value
        .to_str()
        .map_err(|_| unauthorized("malformed Authorization header"))?;
    value
        .strip_prefix("Bearer ")
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .ok_or_else(|| unauthorized("Authorization header must be `Bearer <token>`"))
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
            .register_owner(
                owner,
                &auth_public,
                &auth_proof,
                &ident_public,
                &ident_proof,
            )
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
            iss: SESSION_TOKEN_ISSUER.to_string(),
            aud: SESSION_TOKEN_AUDIENCE.to_string(),
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
            iss: SESSION_TOKEN_ISSUER.to_string(),
            aud: SESSION_TOKEN_AUDIENCE.to_string(),
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
            blob_store: BlobStore::local(temp.path().join("data")),
            rate_limiter: RateLimiter::new(),
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
        assert!(
            body.error.contains("ident owner does not match"),
            "{}",
            body.error
        );

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
        assert!(
            body.error.contains("session owner does not match"),
            "{}",
            body.error
        );

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
        assert_eq!(
            attestation["signingFingerprint"],
            signing_fingerprint.as_str()
        );
        assert_eq!(
            attestation["repoFingerprint"],
            crypto::fingerprint(&server_public).as_str()
        );
        let (_owner, ident_key) = store.owner_with_ident_key("alice").unwrap().unwrap();
        assert_eq!(
            attestation["identFingerprint"],
            ident_key.fingerprint.as_str()
        );
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
                url: String::new(),
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
        let signature = crypto::decode_bytes(&response.attestation_signature, "signature").unwrap();
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
            blob_store: BlobStore::local(temp.path().join("data")),
            rate_limiter: RateLimiter::new(),
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
                url: String::new(),
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
        let report =
            validate_package_request(&state, &valid_request, "validate", VALIDATE_PER_OWNER_MAX)
                .await
                .unwrap();
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
        let report = validate_package_request(&state, &forged, "validate", VALIDATE_PER_OWNER_MAX)
            .await
            .unwrap();
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
        let report = validate_package_request(&state, &forged, "validate", VALIDATE_PER_OWNER_MAX)
            .await
            .unwrap();
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
        let report = validate_package_request(&state, &reused, "validate", VALIDATE_PER_OWNER_MAX)
            .await
            .unwrap();
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
        let report = validate_package_request(&state, &request, "validate", VALIDATE_PER_OWNER_MAX)
            .await
            .unwrap();
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
        let proof_sig = crypto::sign(
            &keys.ident_private,
            &crypto::proof_signing_input(proof.as_bytes()),
        )
        .unwrap();
        let artifact = package::test_support::serialize(
            &package::test_support::TestPackage {
                name: "toolbox".to_string(),
                ident: "alice#toolbox".to_string(),
                version: version.to_string(),
                author: "alice".to_string(),
                url: String::new(),
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

    /// plan-48-A §4.2/§4.3: the native-blob write half. A blob PUTs, HEADs, and
    /// GETs back byte-identically under the `.bin` namespace; a hash-mismatched
    /// body stores nothing; a re-PUT is a cheap success; an unauthenticated or
    /// bogus-token PUT is refused.
    #[tokio::test]
    async fn native_blob_put_head_get_roundtrip_and_auth() {
        let temp = tempfile::tempdir().unwrap();
        let opened =
            Store::open_repository(&temp.path().join("meta.db"), &temp.path().join("data"))
                .unwrap();
        let store = opened.store;
        let keys = register_owner_with_all_keys(&store, "alice");
        let token = open_session(&store, "alice", &keys.auth_private);
        let state = AppState {
            store: store.clone(),
            blob_store: BlobStore::local(opened.packages_dir.clone()),
            rate_limiter: RateLimiter::new(),
        };

        let body = b"\x7fELF vendored native library payload".to_vec();
        let hash = hex::encode(crypto::sha256(&body));
        let bearer = |token: &str| {
            let mut headers = axum::http::HeaderMap::new();
            headers.insert(
                axum::http::header::AUTHORIZATION,
                format!("Bearer {token}").parse().unwrap(),
            );
            headers
        };

        // Nothing stored yet.
        assert_eq!(
            head_blob(State(state.clone()), axum::extract::Path(hash.clone())).await,
            StatusCode::NOT_FOUND
        );

        // No Authorization header at all → 401, and nothing is stored.
        let no_auth = put_blob(
            State(state.clone()),
            axum::extract::Path(hash.clone()),
            axum::http::HeaderMap::new(),
            axum::body::Bytes::from(body.clone()),
        )
        .await;
        assert_eq!(no_auth.err().unwrap().0, StatusCode::UNAUTHORIZED);

        // A syntactically valid but unknown token → 401.
        let bad_token = put_blob(
            State(state.clone()),
            axum::extract::Path(hash.clone()),
            bearer("not.a.session"),
            axum::body::Bytes::from(body.clone()),
        )
        .await;
        assert_eq!(bad_token.err().unwrap().0, StatusCode::UNAUTHORIZED);
        assert!(!state
            .blob_store
            .exists(&hash, BlobKind::Native)
            .await
            .unwrap());

        // A real session stores the blob.
        let created = put_blob(
            State(state.clone()),
            axum::extract::Path(hash.clone()),
            bearer(&token),
            axum::body::Bytes::from(body.clone()),
        )
        .await
        .expect("authenticated put stores the blob");
        assert_eq!(created, StatusCode::CREATED);

        // HEAD now reports it, and it lives under the `.bin` namespace — never
        // `<hash>.mfp`, which would lie about what the file is.
        assert_eq!(
            head_blob(State(state.clone()), axum::extract::Path(hash.clone())).await,
            StatusCode::OK
        );
        assert!(opened.packages_dir.join(format!("{hash}.bin")).exists());
        assert!(!opened.packages_dir.join(format!("{hash}.mfp")).exists());

        // GET serves the exact bytes back.
        let response = package_blob(State(state.clone()), axum::extract::Path(hash.clone()))
            .await
            .expect("native blob served");
        let served = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(served.as_ref(), body.as_slice());

        // Re-uploading the same blob is a cheap success, not an error, and does
        // not duplicate anything.
        let again = put_blob(
            State(state.clone()),
            axum::extract::Path(hash.clone()),
            bearer(&token),
            axum::body::Bytes::from(body.clone()),
        )
        .await
        .expect("re-put is idempotent");
        assert_eq!(again, StatusCode::OK);

        // A body that does not hash to the path is refused and stores NOTHING.
        let other_hash = hex::encode(crypto::sha256(b"entirely different bytes"));
        let mismatch = put_blob(
            State(state.clone()),
            axum::extract::Path(other_hash.clone()),
            bearer(&token),
            axum::body::Bytes::from(body.clone()),
        )
        .await;
        assert_eq!(mismatch.err().unwrap().0, StatusCode::BAD_REQUEST);
        assert!(!state
            .blob_store
            .exists(&other_hash, BlobKind::Native)
            .await
            .unwrap());
        assert_eq!(
            head_blob(State(state.clone()), axum::extract::Path(other_hash)).await,
            StatusCode::NOT_FOUND
        );

        // A non-hex path is a 400 on both verbs.
        assert_eq!(
            head_blob(
                State(state.clone()),
                axum::extract::Path("nothex".to_string())
            )
            .await,
            StatusCode::BAD_REQUEST
        );
        let malformed = put_blob(
            State(state.clone()),
            axum::extract::Path("nothex".to_string()),
            bearer(&token),
            axum::body::Bytes::from(body.clone()),
        )
        .await;
        assert_eq!(malformed.err().unwrap().0, StatusCode::BAD_REQUEST);
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
            blob_store: BlobStore::local(opened.packages_dir.clone()),
            rate_limiter: RateLimiter::new(),
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
        let missing = package_blob(State(state.clone()), axum::extract::Path("0".repeat(64))).await;
        assert_eq!(missing.err().unwrap().0, StatusCode::NOT_FOUND);
        let malformed = package_blob(
            State(state.clone()),
            axum::extract::Path("nothex".to_string()),
        )
        .await;
        assert_eq!(malformed.err().unwrap().0, StatusCode::BAD_REQUEST);

        // A corrupted stored blob is refused (blob-store corruption defense).
        std::fs::write(opened.packages_dir.join(format!("{hash}.mfp")), b"corrupt").unwrap();
        let corrupted = package_blob(State(state.clone()), axum::extract::Path(hash.clone())).await;
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

    /// Open a session bound to a specific auth key (by fingerprint), so a
    /// publish-token session can be exercised.
    fn open_session_for_key(
        store: &Store,
        owner: &str,
        private: &[u8],
        fingerprint: &str,
    ) -> String {
        let challenge = store.create_auth_challenge(owner, fingerprint).unwrap();
        let message = crypto::challenge_message(&challenge.id, &challenge.nonce);
        let signature = crypto::sign(private, &message).unwrap();
        let (owner_rec, key) = store.complete_challenge(&challenge.id, &signature).unwrap();
        let issued_at = crate::store::now_unix();
        let jwt_id = Uuid::new_v4().to_string();
        let claims = SessionClaims {
            sub: owner_rec.owner_display,
            owner_id: owner_rec.id,
            auth_fingerprint: key.fingerprint,
            iat: issued_at,
            exp: issued_at + 3600,
            jti: jwt_id.clone(),
            iss: SESSION_TOKEN_ISSUER.to_string(),
            aud: SESSION_TOKEN_AUDIENCE.to_string(),
        };
        let token = encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(&store.server_secret().unwrap()),
        )
        .unwrap();
        store
            .insert_session(&NewSession {
                owner_id: owner_rec.id,
                key_id: key.id,
                jwt_id,
                issued_at,
                expires_at: issued_at + 3600,
            })
            .unwrap();
        token
    }

    #[tokio::test]
    async fn challenge_rate_limit_trips_after_the_window_cap() {
        let temp = tempfile::tempdir().unwrap();
        let opened =
            Store::open_repository(&temp.path().join("meta.db"), &temp.path().join("data"))
                .unwrap();
        let store = opened.store;
        register_owner_with_all_keys(&store, "alice");
        let state = AppState {
            store: store.clone(),
            blob_store: BlobStore::local(opened.packages_dir.clone()),
            rate_limiter: RateLimiter::new(),
        };
        // The challenge cap is 20 per window; the 21st is refused with 429.
        let mut last = Ok(());
        for _ in 0..21 {
            last = challenge(
                State(state.clone()),
                Json(ChallengeRequest {
                    owner: "alice".to_string(),
                    auth_fingerprint: crypto::fingerprint(&register_dummy()),
                }),
            )
            .await
            .map(|_| ())
            .map_err(|(status, _)| status);
        }
        assert_eq!(last.unwrap_err(), StatusCode::TOO_MANY_REQUESTS);
    }

    fn register_dummy() -> Vec<u8> {
        crypto::generate_keypair().0
    }

    #[tokio::test]
    async fn register_rate_limit_is_per_client_ip() {
        // REPO-12 / bug-188: register/login buckets are keyed by peer IP, so one
        // abusive client can no longer lock every user out of registration.
        let temp = tempfile::tempdir().unwrap();
        let opened =
            Store::open_repository(&temp.path().join("meta.db"), &temp.path().join("data"))
                .unwrap();
        let state = AppState {
            store: opened.store,
            blob_store: BlobStore::local(opened.packages_dir.clone()),
            rate_limiter: RateLimiter::new(),
        };
        // Malformed body: the rate gate runs *before* validation, so each attempt
        // still counts, then fails with 400 until the bucket is exhausted.
        let dummy = || RegisterRequest {
            owner: "alice".to_string(),
            auth_key: String::new(),
            ident_key: String::new(),
            proofs: RegisterProofs {
                auth: String::new(),
                ident: String::new(),
            },
        };
        let ip_a: SocketAddr = "127.0.0.1:5000".parse().unwrap();
        let ip_b: SocketAddr = "127.0.0.2:6000".parse().unwrap();
        let mut last = StatusCode::OK;
        for _ in 0..(REGISTER_PER_IP_MAX + 1) {
            last = register(State(state.clone()), ConnectInfo(ip_a), Json(dummy()))
                .await
                .map(|_| StatusCode::OK)
                .unwrap_or_else(|(status, _)| status);
        }
        assert_eq!(
            last,
            StatusCode::TOO_MANY_REQUESTS,
            "IP A must be throttled once past its per-client cap",
        );
        // A different client IP still gets in (its own empty bucket): no global
        // lockout. It fails with 400 on the malformed body, never 429.
        let other = register(State(state.clone()), ConnectInfo(ip_b), Json(dummy()))
            .await
            .map(|_| StatusCode::OK)
            .unwrap_or_else(|(status, _)| status);
        assert_ne!(
            other,
            StatusCode::TOO_MANY_REQUESTS,
            "a different client IP must not be locked out by IP A's abuse",
        );
    }

    #[tokio::test]
    async fn org_roles_are_ident_authorized_and_role_gated() {
        let temp = tempfile::tempdir().unwrap();
        let opened =
            Store::open_repository(&temp.path().join("meta.db"), &temp.path().join("data"))
                .unwrap();
        let store = opened.store;
        let org = register_owner_with_all_keys(&store, "acme");
        let _member = register_owner_with_all_keys(&store, "alice");
        let mallory = register_owner_with_all_keys(&store, "mallory");
        let org_token = open_session(&store, "acme", &org.auth_private);
        let mallory_token = open_session(&store, "mallory", &mallory.auth_private);
        let state = AppState {
            store: store.clone(),
            blob_store: BlobStore::local(opened.packages_dir.clone()),
            rate_limiter: RateLimiter::new(),
        };

        let grant = |grantor: &str, ident_private: &[u8], token: &str, member: &str, role: &str| {
            let sig = crypto::encode_bytes(
                &crypto::sign(
                    ident_private,
                    &crypto::org_role_message("acme", member, role),
                )
                .unwrap(),
            );
            OrgMemberRequest {
                org: "acme".to_string(),
                grantor: grantor.to_string(),
                member: member.to_string(),
                role: role.to_string(),
                action: "grant".to_string(),
                session_token: token.to_string(),
                ident_signature: sig,
            }
        };

        // The org bootstraps its first member.
        let _ = org_members(
            State(state.clone()),
            Json(grant(
                "acme",
                &org.ident_private,
                &org_token,
                "alice",
                "admin",
            )),
        )
        .await
        .expect("org grants first member");
        assert_eq!(
            store.org_member_role("acme", "alice").unwrap().as_deref(),
            Some("admin")
        );

        // A non-member cannot grant roles even with a valid ident signature.
        let refused = org_members(
            State(state.clone()),
            Json(grant(
                "mallory",
                &mallory.ident_private,
                &mallory_token,
                "mallory",
                "owner",
            )),
        )
        .await;
        assert!(refused.is_err());

        // Removal is logged and takes effect.
        let remove = OrgMemberRequest {
            org: "acme".to_string(),
            grantor: "acme".to_string(),
            member: "alice".to_string(),
            role: String::new(),
            action: "remove".to_string(),
            session_token: org_token.clone(),
            ident_signature: crypto::encode_bytes(
                &crypto::sign(
                    &org.ident_private,
                    &crypto::org_role_message("acme", "alice", "removed"),
                )
                .unwrap(),
            ),
        };
        let _ = org_members(State(state.clone()), Json(remove))
            .await
            .expect("removal");
        assert!(store.org_member_role("acme", "alice").unwrap().is_none());
    }

    #[tokio::test]
    async fn publish_tokens_are_scoped_and_revocable() {
        let temp = tempfile::tempdir().unwrap();
        let opened =
            Store::open_repository(&temp.path().join("meta.db"), &temp.path().join("data"))
                .unwrap();
        let store = opened.store;
        let alice = register_owner_with_all_keys(&store, "alice");
        let owner_token = open_session(&store, "alice", &alice.auth_private);
        let state = AppState {
            store: store.clone(),
            blob_store: BlobStore::local(opened.packages_dir.clone()),
            rate_limiter: RateLimiter::new(),
        };

        // Issue a token scoped to exactly alice#toolbox.
        let (token_public, token_private) = crypto::generate_keypair();
        let token_fingerprint = crypto::fingerprint(&token_public);
        let proof = crypto::sign(
            &token_private,
            &crypto::registration_message(crypto::ROLE_AUTH, "alice", &token_public),
        )
        .unwrap();
        let issue = TokenIssueRequest {
            owner: "alice".to_string(),
            token_key: crypto::encode_bytes(&token_public),
            proof: crypto::encode_bytes(&proof),
            scope: "alice#toolbox".to_string(),
            ttl_seconds: 3600,
            session_token: owner_token.clone(),
            ident_signature: crypto::encode_bytes(
                &crypto::sign(
                    &alice.ident_private,
                    &crypto::token_issue_message("alice", &token_fingerprint, "alice#toolbox"),
                )
                .unwrap(),
            ),
        };
        let _ = issue_token(State(state.clone()), Json(issue))
            .await
            .expect("token issued");

        // The token opens its own session and can attest within scope...
        let token_session =
            open_session_for_key(&store, "alice", &token_private, &token_fingerprint);
        let (in_scope_public, _) = crypto::generate_keypair();
        let _ = signing(
            State(state.clone()),
            Json(SigningRequest {
                owner: "alice".to_string(),
                ident: "alice#toolbox".to_string(),
                version: "1.0.0".to_string(),
                signing_fingerprint: crypto::fingerprint(&in_scope_public),
                session_token: token_session.clone(),
            }),
        )
        .await
        .expect("in-scope attestation");

        // ...but not outside its scope.
        let (out_public, _) = crypto::generate_keypair();
        let refused = signing(
            State(state.clone()),
            Json(SigningRequest {
                owner: "alice".to_string(),
                ident: "alice#other".to_string(),
                version: "1.0.0".to_string(),
                signing_fingerprint: crypto::fingerprint(&out_public),
                session_token: token_session.clone(),
            }),
        )
        .await;
        assert!(refused.err().unwrap().1.error.contains("scope"));

        // Revocation (ident-authorized) kills the token session.
        let _ = revoke_token(
            State(state.clone()),
            Json(TokenRevokeRequest {
                owner: "alice".to_string(),
                token_fingerprint: token_fingerprint.clone(),
                session_token: owner_token.clone(),
                ident_signature: crypto::encode_bytes(
                    &crypto::sign(
                        &alice.ident_private,
                        &crypto::token_revoke_message("alice", &token_fingerprint),
                    )
                    .unwrap(),
                ),
            }),
        )
        .await
        .expect("token revoked");
        let after = signing(
            State(state.clone()),
            Json(SigningRequest {
                owner: "alice".to_string(),
                ident: "alice#toolbox".to_string(),
                version: "2.0.0".to_string(),
                signing_fingerprint: crypto::fingerprint(&in_scope_public),
                session_token: token_session,
            }),
        )
        .await;
        assert!(after.is_err());
    }

    #[tokio::test]
    async fn ownership_transfer_is_two_sided_and_rebinds_the_package() {
        let temp = tempfile::tempdir().unwrap();
        let opened =
            Store::open_repository(&temp.path().join("meta.db"), &temp.path().join("data"))
                .unwrap();
        let store = opened.store;
        let alice = register_owner_with_all_keys(&store, "alice");
        let bob = register_owner_with_all_keys(&store, "bob");
        let alice_token = open_session(&store, "alice", &alice.auth_private);
        let bob_token = open_session(&store, "bob", &bob.auth_private);
        let state = AppState {
            store: store.clone(),
            blob_store: BlobStore::local(opened.packages_dir.clone()),
            rate_limiter: RateLimiter::new(),
        };
        publish_valid_package(&state, &alice, &alice_token, "1.0.0").await;

        // An offer signed by the wrong ident is refused.
        let bad_offer = TransferOfferRequest {
            ident: "alice#toolbox".to_string(),
            from_owner: "alice".to_string(),
            to_owner: "bob".to_string(),
            session_token: alice_token.clone(),
            ident_signature: crypto::encode_bytes(
                &crypto::sign(
                    &bob.ident_private,
                    &crypto::transfer_offer_message("alice#toolbox", "alice", "bob"),
                )
                .unwrap(),
            ),
        };
        assert!(transfer_offer(State(state.clone()), Json(bad_offer))
            .await
            .is_err());

        // A correctly signed offer, then a bob-signed acceptance, re-binds it.
        let offer = TransferOfferRequest {
            ident: "alice#toolbox".to_string(),
            from_owner: "alice".to_string(),
            to_owner: "bob".to_string(),
            session_token: alice_token.clone(),
            ident_signature: crypto::encode_bytes(
                &crypto::sign(
                    &alice.ident_private,
                    &crypto::transfer_offer_message("alice#toolbox", "alice", "bob"),
                )
                .unwrap(),
            ),
        };
        let _ = transfer_offer(State(state.clone()), Json(offer))
            .await
            .expect("offer");
        let accept = TransferAcceptRequest {
            ident: "alice#toolbox".to_string(),
            to_owner: "bob".to_string(),
            session_token: bob_token.clone(),
            ident_signature: crypto::encode_bytes(
                &crypto::sign(
                    &bob.ident_private,
                    &crypto::transfer_accept_message("alice#toolbox", "bob"),
                )
                .unwrap(),
            ),
        };
        let _ = transfer_accept(State(state.clone()), Json(accept))
            .await
            .expect("accept");

        // The package is re-bound to bob; the already-published version persists.
        assert_eq!(
            store
                .package_owner("alice#toolbox")
                .unwrap()
                .unwrap()
                .owner_display,
            "bob"
        );
        assert_eq!(
            store.list_package_versions("alice#toolbox").unwrap().len(),
            1
        );
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
            blob_store: BlobStore::local(opened.packages_dir.clone()),
            rate_limiter: RateLimiter::new(),
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
            blob_store: BlobStore::local(temp.path().join("data")),
            rate_limiter: RateLimiter::new(),
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
                url: String::new(),
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
        let report = validate_package_request(&state, &request, "validate", VALIDATE_PER_OWNER_MAX)
            .await
            .unwrap();
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
        let report = validate_package_request(&state, &request, "validate", VALIDATE_PER_OWNER_MAX)
            .await
            .unwrap();
        assert!(!report.valid);
        assert!(
            report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic
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
                url: String::new(),
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
        let report = validate_package_request(&state, &request, "validate", VALIDATE_PER_OWNER_MAX)
            .await
            .unwrap();
        assert!(report.valid, "{:?}", report.diagnostics);

        // The chain endpoint serves the verifiable link, and a client can
        // follow it from the old pin to the new key.
        let chain = ident_chain(
            State(state.clone()),
            axum::extract::Path("alice".to_string()),
        )
        .await
        .expect("chain served")
        .0;
        assert_eq!(chain.ident_fingerprint, crypto::fingerprint(&new_public));
        assert_eq!(chain.chain.len(), 1);
        let followed = crate::client::follow_ident_chain("alice", &keys.ident_public, &chain.chain)
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
            blob_store: BlobStore::local(temp.path().join("data")),
            rate_limiter: RateLimiter::new(),
        };
        // Grow the log: register (1) + three attestations (4 total).
        for version in ["1.0.0", "1.1.0", "1.2.0"] {
            let (signing_public, _signing_private) = crypto::generate_keypair();
            real_attestation(
                &state,
                &token,
                version,
                &crypto::fingerprint(&signing_public),
            )
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
        real_attestation(
            &state,
            &token,
            "2.0.0",
            &crypto::fingerprint(&signing_public),
        )
        .await;
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
            iss: SESSION_TOKEN_ISSUER.to_string(),
            aud: SESSION_TOKEN_AUDIENCE.to_string(),
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
            iss: SESSION_TOKEN_ISSUER.to_string(),
            aud: SESSION_TOKEN_AUDIENCE.to_string(),
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

    // -----------------------------------------------------------------------
    // bug-347: coverage for the handler surface the tests above leave untouched
    // — the anonymous endpoints (health/ident/register/challenge/login), the
    // machine-link and revocation flows, the signed-metadata chain, and every
    // rejection branch of the authenticated routes.
    // -----------------------------------------------------------------------

    /// One temp-dir-backed registry: the store, a local blob backend, and the
    /// `AppState` the handlers take.
    struct Harness {
        _temp: tempfile::TempDir,
        store: Store,
        state: AppState,
        packages_dir: std::path::PathBuf,
    }

    fn harness() -> Harness {
        let temp = tempfile::tempdir().unwrap();
        let opened =
            Store::open_repository(&temp.path().join("meta.db"), &temp.path().join("data"))
                .unwrap();
        let store = opened.store.clone();
        let state = AppState {
            store: opened.store,
            blob_store: BlobStore::local(opened.packages_dir.clone()),
            rate_limiter: RateLimiter::new(),
        };
        Harness {
            _temp: temp,
            store,
            state,
            packages_dir: opened.packages_dir,
        }
    }

    /// plan-61-B Phase 1's first task, and the reason `build_router` exists.
    ///
    /// `GET /packages/:ident` makes a parameter segment a sibling of the static
    /// `/packages/transfer/offer` and `/packages/transfer/accept` routes.
    /// matchit is expected to prefer the static segment, but a conflict
    /// **panics inside `Router::route` at construction**, which without this
    /// test would surface as a server that dies at startup rather than as a red
    /// test. Constructing the router at all is the assertion.
    #[tokio::test]
    async fn router_has_no_route_conflicts_and_static_beats_param() {
        use tower::ServiceExt;
        let h = harness();
        let router = build_router(h.state.clone());

        // Both transfer routes still resolve to their POST handlers, and are
        // not swallowed by `/packages/:ident`. A 405 (not 404) proves the path
        // matched the static route, which only accepts POST.
        for path in ["/packages/transfer/offer", "/packages/transfer/accept"] {
            let response = router
                .clone()
                .oneshot(
                    axum::http::Request::builder()
                        .method("GET")
                        .uri(path)
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                StatusCode::METHOD_NOT_ALLOWED,
                "{path} must still resolve to the static POST route, not to /packages/:ident",
            );
        }

        // And the param route is reachable for a real ident.
        let response = router
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("GET")
                    .uri("/packages/alice%23toolbox")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::NOT_FOUND,
            "an unpublished ident reaches the handler and 404s",
        );
    }

    /// plan-61-B Phase 1 / §4 — the sub-plan's central behavioural claim.
    ///
    /// A **yanked** version must still be listed. A view that silently omits
    /// non-current versions reproduces exactly the truncation the open SUP-03
    /// downgrade attack performs (`plan-61-repo-web.md` §4), so `state` is a
    /// field to render and never a filter the server applies. This is the test
    /// that would fail if someone later "tidied" the query with a WHERE clause.
    #[tokio::test]
    async fn package_detail_lists_every_version_including_yanked_ones() {
        let h = harness();
        register_owner_with_all_keys(&h.store, "alice");
        let alice_id = h.store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
        h.store
            .record_native_blob("vendorhash", "data/vendorhash.bin")
            .unwrap();

        let targets = [
            crate::abi::VendorBlobRef {
                logical: "snd".to_string(),
                source: "libsnd.dylib".to_string(),
                hash: "vendorhash".to_string(),
                os: "macos".to_string(),
                arch: None,
                libc: None,
                lib_type: "vendor".to_string(),
            },
            crate::abi::VendorBlobRef {
                logical: "snd".to_string(),
                source: "libsnd.a".to_string(),
                hash: "vendorhash".to_string(),
                os: "linux".to_string(),
                arch: Some("x86_64".to_string()),
                libc: Some("musl".to_string()),
                lib_type: "vendor".to_string(),
            },
        ];
        for (version, blobs) in [("1.0.0", &targets[..]), ("2.0.0", &[][..])] {
            h.store
                .publish_package_version(
                    alice_id,
                    "alice#toolbox",
                    version,
                    &format!("hash-{version}"),
                    &format!("data/{version}.mfp"),
                    "{}",
                    blobs,
                    &crate::store::PublishMetadata {
                        author: Some("alice".to_string()),
                        url: Some("https://example.invalid".to_string()),
                        description: None,
                    },
                )
                .unwrap();
        }
        h.store
            .set_release_state("alice#toolbox", "1.0.0", "yanked")
            .unwrap();

        // `%23` is percent-decoded by axum before the handler sees it, so the
        // handler receives a literal `#`.
        let detail = package_detail(
            State(h.state.clone()),
            axum::extract::Path("alice#toolbox".to_string()),
        )
        .await
        .expect("detail served")
        .0;

        assert_eq!(detail.ident, "alice#toolbox");
        assert_eq!(detail.owner, "alice");
        assert_eq!(detail.versions.len(), 2, "the yanked version is still here");
        // Newest first, so `latestVersion` is the un-yanked 2.0.0.
        assert_eq!(detail.latest_version.as_deref(), Some("2.0.0"));
        let yanked = detail
            .versions
            .iter()
            .find(|version| version.version == "1.0.0")
            .expect("the yanked version must be listed, not filtered out");
        assert_eq!(yanked.state, "yanked");

        // Both platform targets render, and the any-arch wildcard stays null
        // rather than collapsing into the concrete arch.
        assert_eq!(yanked.targets.len(), 2);
        let macos = &yanked.targets[0];
        assert_eq!(macos.os, "macos");
        assert_eq!(macos.arch, None);
        assert_eq!(macos.lib_type, "vendor");
        let linux = &yanked.targets[1];
        assert_eq!(linux.arch.as_deref(), Some("x86_64"));
        assert_eq!(linux.libc.as_deref(), Some("musl"));

        // Package-level metadata comes from the newest version.
        assert_eq!(detail.author.as_deref(), Some("alice"));
        assert_eq!(detail.url.as_deref(), Some("https://example.invalid"));
        // `description` is in the shape from day one, null until plan-61-E, so
        // that sub-plan adds no field and breaks no consumer.
        assert_eq!(detail.description, None);
    }

    /// An unknown package 404s with the standard error shape, and an unknown
    /// *owner* is indistinguishable from an unknown *package* — otherwise the
    /// route is an owner-enumeration oracle.
    #[tokio::test]
    async fn the_read_routes_404_without_leaking_whether_an_owner_exists() {
        let h = harness();
        register_owner_with_all_keys(&h.store, "alice");

        let known_owner = err_of(
            package_detail(
                State(h.state.clone()),
                axum::extract::Path("alice#nosuchpackage".to_string()),
            )
            .await,
        );
        let unknown_owner = err_of(
            package_detail(
                State(h.state.clone()),
                axum::extract::Path("mallory#nosuchpackage".to_string()),
            )
            .await,
        );
        assert_eq!(known_owner.0, StatusCode::NOT_FOUND);
        assert_eq!(
            known_owner, unknown_owner,
            "a registered owner and an unregistered one must be indistinguishable",
        );

        // The audit route answers identically.
        let audit_known = err_of(
            package_audit(
                State(h.state.clone()),
                axum::extract::Path("alice#nosuchpackage".to_string()),
            )
            .await,
        );
        let audit_unknown = err_of(
            package_audit(
                State(h.state.clone()),
                axum::extract::Path("mallory#nosuchpackage".to_string()),
            )
            .await,
        );
        assert_eq!(audit_known.0, StatusCode::NOT_FOUND);
        assert_eq!(audit_known, audit_unknown);

        // A malformed ident is a 400, not a 404.
        for ident in ["noseparator", "#toolbox", "alice#"] {
            let (status, message) = err_of(
                package_detail(
                    State(h.state.clone()),
                    axum::extract::Path(ident.to_string()),
                )
                .await,
            );
            assert_eq!(status, StatusCode::BAD_REQUEST, "{ident}");
            assert!(message.contains("<owner>#<package>"), "{ident}: {message}");
        }
    }

    /// plan-61-B §4: the audit route exposes an **inclusion proof** per publish,
    /// not just an index. A rendered log index nobody can verify proves nothing;
    /// the proof against the checkpoint in the same response is what lets a
    /// third-party monitor catch a registry equivocating.
    #[tokio::test]
    async fn package_audit_returns_a_checkpoint_with_verifiable_inclusion_proofs() {
        let h = harness();
        register_owner_with_all_keys(&h.store, "alice");
        let alice_id = h.store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
        for version in ["1.0.0", "2.0.0"] {
            h.store
                .publish_package_version(
                    alice_id,
                    "alice#toolbox",
                    version,
                    &format!("hash-{version}"),
                    &format!("data/{version}.mfp"),
                    "{}",
                    &[],
                    &crate::store::PublishMetadata::default(),
                )
                .unwrap();
        }
        h.store
            .set_release_state("alice#toolbox", "1.0.0", "yanked")
            .unwrap();

        let audit = package_audit(
            State(h.state.clone()),
            axum::extract::Path("alice#toolbox".to_string()),
        )
        .await
        .expect("audit served")
        .0;

        assert_eq!(audit.ident, "alice#toolbox");
        assert_eq!(audit.owner, "alice");
        assert_eq!(audit.publishes.len(), 2);
        assert!(audit.log_checkpoint.size >= 2);

        // Every proof actually verifies against the checkpoint served
        // alongside it — the property that makes the tab worth anything.
        let root: [u8; 32] = hex::decode(&audit.log_checkpoint.root_hash)
            .unwrap()
            .try_into()
            .unwrap();
        for entry in &audit.publishes {
            let leaf: [u8; 32] = hex::decode(&entry.leaf_hash).unwrap().try_into().unwrap();
            let path: Vec<[u8; 32]> = entry
                .proof
                .iter()
                .map(|hop| hex::decode(hop).unwrap().try_into().unwrap())
                .collect();
            crate::log::verify_inclusion(
                entry.index as usize,
                audit.log_checkpoint.size as usize,
                &leaf,
                &path,
                &root,
            )
            .unwrap_or_else(|err| {
                panic!(
                    "inclusion proof for {} must verify against the served checkpoint: {err}",
                    entry.version,
                )
            });
        }

        // The yank shows up as a release-state transition.
        assert!(
            audit
                .state_changes
                .iter()
                .any(|change| change.version == "1.0.0" && change.state == "yanked"),
            "{:?}",
            audit.state_changes,
        );
    }

    /// plan-61-B Phase 1 acceptance: the read routes are anonymous, and a
    /// request carrying a valid credential behaves **identically** to one
    /// carrying none.
    ///
    /// Asserting equality both ways matters. "No credential required" would
    /// still hold if the route quietly read a token and widened what it
    /// returned; that is the shape this rules out. The handlers take no token
    /// parameter at all, and this pins that as behaviour rather than as a
    /// property of the current signature.
    #[tokio::test]
    async fn the_read_routes_ignore_credentials_entirely() {
        use tower::ServiceExt;
        let h = harness();
        let keys = register_owner_with_all_keys(&h.store, "alice");
        let token = open_session(&h.store, "alice", &keys.auth_private);
        let alice_id = h.store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
        h.store
            .publish_package_version(
                alice_id,
                "alice#toolbox",
                "1.0.0",
                "hash",
                "data/1.0.0.mfp",
                "{}",
                &[],
                &crate::store::PublishMetadata::default(),
            )
            .unwrap();

        let router = build_router(h.state.clone());
        let fetch = |authorized: bool| {
            let router = router.clone();
            let token = token.clone();
            async move {
                let mut request = axum::http::Request::builder()
                    .method("GET")
                    .uri("/packages/alice%23toolbox");
                if authorized {
                    request = request
                        .header("Authorization", format!("Bearer {token}"))
                        .header("Cookie", "session=whatever");
                }
                let response = router
                    .oneshot(request.body(axum::body::Body::empty()).unwrap())
                    .await
                    .unwrap();
                let status = response.status();
                let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                    .await
                    .unwrap();
                (status, body)
            }
        };

        let (anon_status, anon_body) = fetch(false).await;
        let (auth_status, auth_body) = fetch(true).await;
        assert_eq!(anon_status, StatusCode::OK);
        assert_eq!(
            anon_status, auth_status,
            "a credential must not change the status",
        );
        assert_eq!(
            anon_body, auth_body,
            "a credential must not change the body — the route is anonymous in \
             both directions, not merely credential-optional",
        );
    }

    /// Seed a handful of packages for the search tests.
    fn seed_packages(h: &Harness, idents: &[&str]) {
        for ident in idents {
            let owner = ident.split('#').next().unwrap();
            if h.store.owner_with_ident_key(owner).unwrap().is_none() {
                register_owner_with_all_keys(&h.store, owner);
            }
            let owner_id = h.store.owner_with_ident_key(owner).unwrap().unwrap().0.id;
            h.store
                .publish_package_version(
                    owner_id,
                    ident,
                    "1.0.0",
                    &format!("hash-{ident}"),
                    &format!("data/{ident}.mfp"),
                    "{}",
                    &[],
                    &crate::store::PublishMetadata::default(),
                )
                .unwrap();
        }
    }

    async fn search_for(h: &Harness, uri: &str) -> SearchResponse {
        let query = axum::extract::Query::try_from_uri(&uri.parse().unwrap()).unwrap();
        search(State(h.state.clone()), peer("203.0.113.9"), query)
            .await
            .expect("search served")
            .0
    }

    /// plan-61-B §3: exact beats prefix beats substring, and the owner match is
    /// the tail of the ranked set.
    #[tokio::test]
    async fn search_ranks_exact_then_prefix_then_substring() {
        let h = harness();
        seed_packages(
            &h,
            &["alice#sql", "alice#sqlite", "alice#mysqlclient", "bob#tool"],
        );

        let response = search_for(&h, "http://x/search?q=sql").await;
        let idents: Vec<&str> = response
            .results
            .iter()
            .map(|result| result.ident.as_str())
            .collect();
        assert_eq!(
            idents,
            vec!["alice#sql", "alice#sqlite", "alice#mysqlclient"],
            "exact, then prefix, then substring",
        );
        assert_eq!(response.query, "sql");
        assert_eq!(response.total, 3);
        assert_eq!(response.results[0].latest_version.as_deref(), Some("1.0.0"));
        // `description` is in the shape from day one and null until plan-61-E.
        assert_eq!(response.results[0].description, None);

        // An owner-name query finds that owner's packages.
        let response = search_for(&h, "http://x/search?q=bob").await;
        assert_eq!(response.results.len(), 1);
        assert_eq!(response.results[0].ident, "bob#tool");
    }

    /// An empty or whitespace-only query returns nothing — never the whole
    /// table. This is an anonymous route; "no filter" must not mean "enumerate
    /// the registry".
    #[tokio::test]
    async fn search_with_an_empty_query_returns_nothing() {
        let h = harness();
        seed_packages(&h, &["alice#sql", "alice#tool"]);

        for uri in [
            "http://x/search",
            "http://x/search?q=",
            "http://x/search?q=%20%20",
        ] {
            let response = search_for(&h, uri).await;
            assert!(
                response.results.is_empty(),
                "{uri} must not enumerate the registry: {:?}",
                response.results,
            );
        }
    }

    /// plan-61-B Phase 2 acceptance: `?limit` above the cap is **clamped**, not
    /// honoured — an uncapped limit on an anonymous enumerate route is a
    /// resource-exhaustion lever.
    #[tokio::test]
    async fn search_clamps_an_over_cap_limit_and_pages_by_offset() {
        let h = harness();
        let idents: Vec<String> = (0..(SEARCH_LIMIT_MAX + 5))
            .map(|n| format!("alice#pkg{n:03}"))
            .collect();
        seed_packages(&h, &idents.iter().map(String::as_str).collect::<Vec<_>>());

        let response = search_for(&h, "http://x/search?q=pkg&limit=100000").await;
        assert_eq!(
            response.results.len() as i64,
            SEARCH_LIMIT_MAX,
            "an over-cap limit is clamped to the cap, not honoured",
        );

        // A negative limit clamps to zero rather than reaching SQLite as -1,
        // which SQLite reads as "no limit".
        let response = search_for(&h, "http://x/search?q=pkg&limit=-1").await;
        assert!(response.results.is_empty());

        // Offset pages within the capped window.
        let first = search_for(&h, "http://x/search?q=pkg&limit=2").await;
        let second = search_for(&h, "http://x/search?q=pkg&limit=2&offset=2").await;
        assert_eq!(first.results.len(), 2);
        assert_eq!(second.results.len(), 2);
        assert_ne!(first.results[0].ident, second.results[0].ident);
    }

    /// A query full of SQL/LIKE metacharacters must be treated as literal text.
    ///
    /// `%` and `_` are LIKE wildcards: unescaped, `?q=%` would match every
    /// package and hand an anonymous caller the whole registry through a route
    /// whose empty-query case deliberately returns nothing. `'` and the escape
    /// character itself are the injection shapes.
    #[tokio::test]
    async fn search_treats_like_and_sql_metacharacters_as_literal_text() {
        let h = harness();
        seed_packages(&h, &["alice#sql", "alice#tool", "bob#thing"]);

        for query in ["%", "_", "%%", "'", "' OR 1=1 --", "\\", "%_\\"] {
            let encoded: String = query.bytes().map(|byte| format!("%{byte:02X}")).collect();
            let response = search_for(&h, &format!("http://x/search?q={encoded}")).await;
            assert!(
                response.results.is_empty(),
                "query {query:?} must match literally and return nothing, not \
                 wildcard-match the whole registry: {:?}",
                response.results,
            );
        }

        // A package whose ident genuinely contains an underscore is still
        // findable — escaping must not break literal matching.
        seed_packages(&h, &["alice#with_underscore"]);
        let response = search_for(&h, "http://x/search?q=with_under").await;
        assert_eq!(response.results.len(), 1);
        assert_eq!(response.results[0].ident, "alice#with_underscore");
    }

    /// The edit-distance-1 fuzzy tail runs only when the ranked query finds
    /// nothing — it is an unindexed full table scan (`store.rs`
    /// `typosquat_candidates` shape), so it must not sit on the common path.
    #[tokio::test]
    async fn search_falls_back_to_the_fuzzy_tail_only_when_nothing_matched() {
        let h = harness();
        seed_packages(&h, &["alice#sqlite"]);

        // A typo one edit away from the full ident finds it.
        let response = search_for(&h, "http://x/search?q=alice%23sqlit").await;
        assert_eq!(response.results.len(), 1);
        assert_eq!(response.results[0].ident, "alice#sqlite");

        // Something far away still finds nothing.
        let response = search_for(&h, "http://x/search?q=zzzzzzzz").await;
        assert!(response.results.is_empty());
    }

    /// `/search` is anonymous but rate-limited per peer IP, following the
    /// `REGISTER_PER_IP_MAX` precedent — there is no `claims.sub` to key on.
    #[tokio::test]
    async fn search_is_rate_limited_per_peer_ip() {
        let h = harness();
        seed_packages(&h, &["alice#sql"]);
        let query = || {
            axum::extract::Query::try_from_uri(&"http://x/search?q=sql".parse().unwrap()).unwrap()
        };

        for _ in 0..SEARCH_PER_IP_MAX {
            let _ = search(State(h.state.clone()), peer("198.51.100.7"), query())
                .await
                .expect("within the per-IP window");
        }
        let (status, _message) =
            err_of(search(State(h.state.clone()), peer("198.51.100.7"), query()).await);
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);

        // A different client is unaffected: the bucket is per IP, not global.
        let _ = search(State(h.state.clone()), peer("198.51.100.8"), query())
            .await
            .expect("a different peer has its own bucket");
    }

    /// Drive a GET through the real router and return `(status, headers, body)`.
    async fn get_page(state: &AppState, uri: &str) -> (StatusCode, axum::http::HeaderMap, String) {
        use tower::ServiceExt;
        // `serve` supplies `ConnectInfo` via
        // `into_make_service_with_connect_info`; `oneshot` bypasses that layer,
        // so the rate-limited handlers would fail their extractor with a 500.
        // Inject it as a request extension, which is where that layer puts it.
        let mut request = axum::http::Request::builder()
            .method("GET")
            .uri(uri)
            .body(axum::body::Body::empty())
            .unwrap();
        request.extensions_mut().insert(ConnectInfo(
            "203.0.113.1:40000".parse::<SocketAddr>().unwrap(),
        ));
        let response = build_router(state.clone()).oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, headers, String::from_utf8_lossy(&body).into_owned())
    }

    /// plan-61-C Phase 2: adding a `/` handler and the `.html` routes must not
    /// shadow any existing JSON route. `/health` and `/ident` are called out
    /// specifically because a catch-all or a misplaced `/` route is exactly how
    /// they would silently start returning HTML.
    #[tokio::test]
    async fn the_html_routes_do_not_shadow_any_json_route() {
        let h = harness();

        for (uri, expected) in [
            ("/health", "application/json"),
            ("/ident", "application/json"),
            ("/log/checkpoint", "application/json"),
            ("/search?q=x", "application/json"),
        ] {
            let (status, headers, _body) = get_page(&h.state, uri).await;
            assert_eq!(status, StatusCode::OK, "{uri}");
            assert!(
                headers
                    .get(header::CONTENT_TYPE)
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .starts_with(expected),
                "{uri} must still be JSON, not HTML",
            );
        }

        // And the HTML routes are HTML, with the CSP.
        for uri in ["/", "/search.html?q=x"] {
            let (status, headers, _body) = get_page(&h.state, uri).await;
            assert_eq!(status, StatusCode::OK, "{uri}");
            assert!(headers
                .get(header::CONTENT_TYPE)
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("text/html"));
            assert_eq!(
                headers
                    .get(header::CONTENT_SECURITY_POLICY)
                    .unwrap()
                    .to_str()
                    .unwrap(),
                crate::web::CONTENT_SECURITY_POLICY,
                "{uri}",
            );
        }

        // The stylesheet is a real route, which is what `style-src 'self'`
        // with no `'unsafe-inline'` requires.
        let (status, headers, body) = get_page(&h.state, "/style.css").await;
        assert_eq!(status, StatusCode::OK);
        assert!(headers
            .get(header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("text/css"));
        assert!(
            body.contains("--font-sans"),
            "the real stylesheet is served"
        );
    }

    /// The landing page shows the **root** fingerprint, worded as something to
    /// compare rather than as a trust claim (plan-61-C §4). Showing the
    /// `/ident` server fingerprint above a `mfb repo trust` command — which
    /// consumes the *root* fingerprint — would read as a verified copy-paste
    /// and then fail.
    #[tokio::test]
    async fn the_landing_page_presents_the_root_fingerprint_as_a_thing_to_compare() {
        let h = harness();
        h.store
            .init_registry_root("reg.example", 4_102_444_800)
            .unwrap();
        let config = h.store.registry_config().unwrap().unwrap();
        let root_fingerprint = crypto::fingerprint(&config.root_public);
        let server_fingerprint = crypto::fingerprint(&h.store.server_keypair().unwrap().0);

        let (status, _headers, body) = get_page(&h.state, "/").await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            body.contains(&root_fingerprint),
            "the root fingerprint must be shown",
        );
        assert!(
            !body.contains(&server_fingerprint),
            "the /ident server fingerprint is a different value from a different \
             key and must not appear next to `mfb repo trust`",
        );
        assert!(body.contains("mfb repo trust reg.example"));

        // The copy must not claim the page authenticates itself.
        assert!(body.contains("Compare this"));
        assert!(body.contains("cannot prove that it is the real mfb-repo"));
        // Scoped to the fingerprint section: the footer legitimately says
        // content is "unverified by the registry", which contains "verified".
        let section = body
            .split("class=\"fingerprint\"")
            .nth(1)
            .expect("the fingerprint section is present")
            .split("</section>")
            .next()
            .unwrap();
        for overclaim in [
            "is verified",
            "Verified",
            "secure connection",
            "trusted registry",
            "authentic",
        ] {
            assert!(
                !section.contains(overclaim),
                "the fingerprint block must not overclaim with {overclaim:?}",
            );
        }

        // No script, no inline style, and a real stylesheet link.
        assert!(!body.contains("<script"));
        assert!(!body.contains("style="));
        assert!(body.contains("href=\"/style.css\""));
    }

    /// A search that matches nothing renders a "no results" page with **HTTP
    /// 200**, not a 404: the request succeeded and the answer is "none".
    #[tokio::test]
    async fn the_search_page_renders_all_three_states() {
        let h = harness();
        seed_packages(&h, &["alice#toolbox"]);

        // Results.
        let (status, _headers, body) = get_page(&h.state, "/search.html?q=toolbox").await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("alice#toolbox"));
        assert!(
            body.contains("/p/alice%23toolbox"),
            "results link to the package page"
        );
        assert!(body.contains("1</strong> results"), "{body}");

        // No results — 200, not 404.
        let (status, _headers, body) = get_page(&h.state, "/search.html?q=xyzzy-nope").await;
        assert_eq!(
            status,
            StatusCode::OK,
            "an empty result set is a successful request, not a missing page",
        );
        assert!(body.contains("No packages match this query."));

        // Empty query — the form, and no enumeration of the whole table.
        let (status, _headers, body) = get_page(&h.state, "/search.html").await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("Search the registry"));
        assert!(
            !body.contains("alice#toolbox"),
            "an empty query must not list the registry",
        );
    }

    /// A hostile ident renders escaped in the results list. The ident charset is
    /// restricted at publish, so this drives the renderer directly — the
    /// escaping must not depend on an upstream validator that a future change
    /// could relax.
    #[tokio::test]
    async fn search_results_escape_html_metacharacters_in_publisher_values() {
        let rows = [crate::web::SearchRow {
            ident: "alice#<script>alert(1)</script>".to_string(),
            owner: "<img src=x onerror=alert(1)>".to_string(),
            latest_version: Some("1.0.0\"><script>".to_string()),
            description: Some("<b>bold</b>".to_string()),
            published_at: Some(1_700_000_000),
        }];
        let rendered = crate::web::search_page("reg", "<script>", &rows).into_string();

        // Assert the absence of live *markup*, not of scary substrings: an
        // escaped `&lt;img src=x onerror=alert(1)&gt;` legitimately still
        // contains the text "onerror=", and asserting on that would be a test
        // that fails for the wrong reason.
        assert!(!rendered.contains("<script"), "{rendered}");
        assert!(!rendered.contains("<img"), "{rendered}");
        assert!(!rendered.contains("<b>bold"), "{rendered}");
        assert!(rendered.contains("&lt;script&gt;"));
        assert!(
            rendered.contains("&lt;img src=x onerror=alert(1)&gt;"),
            "{rendered}"
        );
        // The echoed query is publisher-independent but still user-controlled.
        assert!(rendered.contains("query-echo"));
    }

    /// **The XSS regression test** (plan-61-C Phase 3) — the single most
    /// important test in this sub-plan.
    ///
    /// Publishes a package whose `author` is `<script>alert(1)</script>` and
    /// whose `url` is `javascript:alert(1)`, then asserts the rendered page:
    /// shows the author as visible escaped text, contains no `<script`
    /// substring anywhere, and renders the hostile url as text rather than an
    /// anchor. This is the same hostile fixture the mockups use, kept in sync
    /// as §3.1 requires.
    ///
    /// It drives the **real route through the real router**, not the template
    /// function, so it covers the whole path including the CSP header.
    #[tokio::test]
    async fn a_hostile_author_and_url_render_inert() {
        let h = harness();
        register_owner_with_all_keys(&h.store, "alice");
        let alice_id = h.store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
        h.store
            .record_native_blob("vendorhash", "data/vendorhash.bin")
            .unwrap();
        h.store
            .publish_package_version(
                alice_id,
                "alice#toolbox",
                "1.0.0",
                "hash-1",
                "data/1.mfp",
                "{}",
                &[crate::abi::VendorBlobRef {
                    logical: "<script>alert('logical')</script>".to_string(),
                    source: "<img src=x onerror=alert('source')>".to_string(),
                    hash: "vendorhash".to_string(),
                    os: "linux".to_string(),
                    arch: None,
                    libc: None,
                    lib_type: "vendor".to_string(),
                }],
                &crate::store::PublishMetadata {
                    author: Some("<script>alert(1)</script>".to_string()),
                    url: Some("javascript:alert(1)".to_string()),
                    // plan-61-E extends this fixture to the description too.
                    description: Some("<img src=x onerror=alert('desc')>".to_string()),
                },
            )
            .unwrap();

        let (status, headers, body) = get_page(&h.state, "/p/alice%23toolbox").await;
        assert_eq!(status, StatusCode::OK);

        // 1. No live script anywhere on the page — from any of the four
        //    publisher-controlled fields.
        assert!(!body.contains("<script"), "{body}");
        assert!(!body.contains("<img"), "{body}");
        // NB: the *string* `javascript:` is expected to appear — §2 requires
        // the hostile url to render as visible inert text, so asserting its
        // absence would contradict the design. What must never appear is the
        // url in an attribute position; that is asserted below.

        // 2. The hostile author is *visible*, escaped. Hiding it would keep the
        //    hostile value from the reader best placed to notice it.
        assert!(
            body.contains("&lt;script&gt;alert(1)&lt;/script&gt;"),
            "the author must render as visible escaped text: {body}",
        );

        // 3. The hostile url is text, not an anchor, and is annotated as
        //    withheld rather than silently dropped.
        assert!(
            body.contains("url-inert"),
            "the javascript: url must render inert: {body}",
        );
        assert!(body.contains("link withheld"), "{body}");
        assert!(
            body.contains(">javascript:alert(1)<"),
            "the hostile url must still be visible to the reader: {body}",
        );
        assert!(
            !body.contains("href=\"javascript"),
            "the hostile url must never become an href: {body}",
        );
        assert!(
            !body.contains("=\"javascript:"),
            "the hostile url must never reach any attribute: {body}",
        );

        // 4. plan-61-E: the description is publisher-controlled too, and gets
        //    the same treatment — visible, escaped, and never a live attribute.
        assert!(
            body.contains("&lt;img src=x onerror=alert(&#39;desc&#39;)&gt;")
                || body.contains("&lt;img src=x onerror=alert('desc')&gt;"),
            "the hostile description must render as visible escaped text: {body}",
        );

        // 5. The CSP is present, so even a total escaping failure could not
        //    execute.
        assert_eq!(
            headers
                .get(header::CONTENT_SECURITY_POLICY)
                .unwrap()
                .to_str()
                .unwrap(),
            crate::web::CONTENT_SECURITY_POLICY,
        );
    }

    /// The rendered version table shows every state, and the target table
    /// renders a NULL arch as "any" rather than as a blank cell that would read
    /// as missing data (plan-61-C Phase 3).
    #[tokio::test]
    async fn the_package_page_shows_yanked_versions_and_the_target_matrix() {
        let h = harness();
        register_owner_with_all_keys(&h.store, "alice");
        let alice_id = h.store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
        h.store
            .record_native_blob("vendorhash", "data/vendorhash.bin")
            .unwrap();
        let targets = [
            crate::abi::VendorBlobRef {
                logical: "snd".to_string(),
                source: "libsnd.dylib".to_string(),
                hash: "vendorhash".to_string(),
                os: "macos".to_string(),
                arch: None,
                libc: None,
                lib_type: "vendor".to_string(),
            },
            crate::abi::VendorBlobRef {
                logical: "snd".to_string(),
                source: "libsnd.a".to_string(),
                hash: "vendorhash".to_string(),
                os: "linux".to_string(),
                arch: Some("x86_64".to_string()),
                libc: Some("musl".to_string()),
                lib_type: "vendor".to_string(),
            },
        ];
        for (version, blobs) in [("1.0.0", &targets[..]), ("2.0.0", &[][..])] {
            h.store
                .publish_package_version(
                    alice_id,
                    "alice#toolbox",
                    version,
                    &format!("hash-{version}"),
                    &format!("data/{version}.mfp"),
                    "{}",
                    blobs,
                    &crate::store::PublishMetadata::default(),
                )
                .unwrap();
        }
        h.store
            .set_release_state("alice#toolbox", "1.0.0", "yanked")
            .unwrap();

        let (status, _headers, body) = get_page(&h.state, "/p/alice%23toolbox").await;
        assert_eq!(status, StatusCode::OK);

        // Every version present, the yanked one labeled and visually marked.
        assert!(
            body.contains("Versions <span class=\"muted\">(2)"),
            "{body}"
        );
        assert!(body.contains("state--yanked"), "{body}");
        assert!(body.contains(">yanked<"), "{body}");
        assert!(body.contains("state--available"), "{body}");

        // Two target rows, and the wildcard arch reads as "any".
        assert!(body.contains("Native targets — 2 for v1.0.0"), "{body}");
        assert!(
            body.contains(">any<"),
            "a NULL arch renders as \"any\": {body}"
        );
        assert!(body.contains("x86_64"));
        assert!(body.contains("musl"));

        // No script on the page at all.
        assert!(!body.contains("<script"));
    }

    /// The audit tab renders the checkpoint, the per-publish inclusion proof
    /// path, and links the raw JSON so a monitor can script against it.
    #[tokio::test]
    async fn the_audit_tab_renders_proofs_and_links_the_raw_json() {
        let h = harness();
        register_owner_with_all_keys(&h.store, "alice");
        let alice_id = h.store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
        for version in ["1.0.0", "2.0.0"] {
            h.store
                .publish_package_version(
                    alice_id,
                    "alice#toolbox",
                    version,
                    &format!("hash-{version}"),
                    &format!("data/{version}.mfp"),
                    "{}",
                    &[],
                    &crate::store::PublishMetadata::default(),
                )
                .unwrap();
        }
        h.store
            .set_release_state("alice#toolbox", "1.0.0", "yanked")
            .unwrap();

        let (status, _headers, body) = get_page(&h.state, "/p/alice%23toolbox/audit").await;
        assert_eq!(status, StatusCode::OK);

        assert!(body.contains("Log checkpoint"));
        assert!(body.contains("entries"));
        assert!(body.contains("inclusion proof"));
        assert!(
            body.contains("proof-path"),
            "the proof path renders: {body}"
        );
        // The raw JSON endpoint is linked for third-party monitors.
        assert!(
            body.contains("/packages/alice%23toolbox/audit"),
            "the raw JSON endpoint must be linked: {body}",
        );
        // The yank appears as a state transition.
        assert!(body.contains("State changes"));
        assert!(body.contains("state--yanked"));
        // The copy frames this as evidence, not assurance.
        assert!(body.contains("evidence for you to check"));
        assert!(!body.contains("<script"));
    }

    /// An unknown package renders a 404 **page**, not a bare status — and that
    /// page still carries the CSP, because it is built by the shared builder.
    #[tokio::test]
    async fn an_unknown_package_renders_a_404_page_with_the_csp() {
        let h = harness();
        for uri in ["/p/alice%23nope", "/p/alice%23nope/audit"] {
            let (status, headers, body) = get_page(&h.state, uri).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
            assert!(body.contains("Package not found"), "{uri}: {body}");
            assert!(body.contains("<html"), "{uri} must render a page");
            assert_eq!(
                headers
                    .get(header::CONTENT_SECURITY_POLICY)
                    .unwrap()
                    .to_str()
                    .unwrap(),
                crate::web::CONTENT_SECURITY_POLICY,
                "{uri}",
            );
        }
    }

    /// Seed a package whose newest version carries `description`.
    fn seed_with_description(h: &Harness, ident: &str, description: &str) {
        let owner = ident.split('#').next().unwrap();
        if h.store.owner_with_ident_key(owner).unwrap().is_none() {
            register_owner_with_all_keys(&h.store, owner);
        }
        let owner_id = h.store.owner_with_ident_key(owner).unwrap().unwrap().0.id;
        h.store
            .publish_package_version(
                owner_id,
                ident,
                "1.0.0",
                &format!("hash-{ident}"),
                &format!("data/{ident}.mfp"),
                "{}",
                &[],
                &crate::store::PublishMetadata {
                    description: Some(description.to_string()),
                    ..Default::default()
                },
            )
            .unwrap();
    }

    /// plan-61-E Phase 1/3: a description flows database → JSON → HTML, and a
    /// package without one stays `null` and renders no stray "None".
    #[tokio::test]
    async fn a_description_reaches_the_json_and_the_page() {
        let h = harness();
        seed_with_description(&h, "alice#toolbox", "Dense matrix primitives.");
        seed_packages(&h, &["bob#plain"]);

        let detail = package_detail(
            State(h.state.clone()),
            axum::extract::Path("alice#toolbox".to_string()),
        )
        .await
        .expect("detail served")
        .0;
        assert_eq!(
            detail.description.as_deref(),
            Some("Dense matrix primitives.")
        );

        let (_status, _headers, body) = get_page(&h.state, "/p/alice%23toolbox").await;
        assert!(body.contains("Dense matrix primitives."), "{body}");

        // A package with no description: null in JSON, and the page shows the
        // placeholder rather than an empty element or a stray "None".
        let detail = package_detail(
            State(h.state.clone()),
            axum::extract::Path("bob#plain".to_string()),
        )
        .await
        .expect("detail served")
        .0;
        assert_eq!(detail.description, None);
        let (_status, _headers, body) = get_page(&h.state, "/p/bob%23plain").await;
        assert!(body.contains("No description provided."), "{body}");
        assert!(
            !body.contains("None"),
            "no stray Rust Option rendering: {body}"
        );
    }

    /// plan-61-E Phase 2: a package findable **only** by a word in its
    /// description is returned, and an ident match still outranks a description
    /// match for the same query.
    #[tokio::test]
    async fn search_matches_descriptions_but_ranks_them_below_idents() {
        let h = harness();
        seed_with_description(&h, "alice#toolbox", "A library for zygomorphic layouts.");
        seed_packages(&h, &["bob#zygomorphic"]);

        // Findable only by a description word.
        let response = search_for(&h, "http://x/search?q=layouts").await;
        assert_eq!(response.results.len(), 1, "{:?}", response.results);
        assert_eq!(response.results[0].ident, "alice#toolbox");

        // For a term appearing in one package's *ident* and another's
        // *description*, the ident wins.
        let response = search_for(&h, "http://x/search?q=zygomorphic").await;
        let idents: Vec<&str> = response
            .results
            .iter()
            .map(|result| result.ident.as_str())
            .collect();
        assert_eq!(
            idents,
            vec!["bob#zygomorphic", "alice#toolbox"],
            "an ident match must outrank a description match",
        );
    }

    /// A search result carries a **clamped** description; the package page
    /// carries the full text. Without the clamp, one page of 50 results could
    /// ship 50 × 4096 bytes on an anonymous route.
    #[tokio::test]
    async fn search_results_clamp_long_descriptions_on_a_character_boundary() {
        let h = harness();
        // Multi-byte characters, so a byte clamp would split one and produce
        // invalid UTF-8 or a broken glyph.
        let long: String = "é".repeat(SEARCH_DESCRIPTION_PREVIEW_CHARS + 50);
        seed_with_description(&h, "alice#toolbox", &long);

        let response = search_for(&h, "http://x/search?q=toolbox").await;
        let preview = response.results[0].description.as_ref().unwrap();
        assert_eq!(
            preview.chars().count(),
            SEARCH_DESCRIPTION_PREVIEW_CHARS + 1,
            "clamped to the preview length plus the ellipsis",
        );
        assert!(preview.ends_with('…'));
        assert!(preview.starts_with('é'), "no split character: {preview}");

        // The package page still shows the whole thing.
        let detail = package_detail(
            State(h.state.clone()),
            axum::extract::Path("alice#toolbox".to_string()),
        )
        .await
        .expect("detail served")
        .0;
        assert_eq!(detail.description.as_deref(), Some(long.as_str()));

        // A description at or under the limit is returned untouched, with no
        // gratuitous ellipsis.
        let exact: String = "x".repeat(SEARCH_DESCRIPTION_PREVIEW_CHARS);
        seed_with_description(&h, "alice#exact", &exact);
        let response = search_for(&h, "http://x/search?q=exact").await;
        assert_eq!(
            response.results[0].description.as_deref(),
            Some(exact.as_str())
        );
    }

    fn peer(ip: &str) -> ConnectInfo<SocketAddr> {
        ConnectInfo(format!("{ip}:40000").parse().unwrap())
    }

    /// The (status, message) of an expected error response.
    fn err_of<T>(result: Result<T, (StatusCode, Json<ErrorResponse>)>) -> (StatusCode, String) {
        let (status, body) = result
            .map(|_| ())
            .err()
            .expect("expected an error response");
        (status, body.0.error)
    }

    fn bearer_headers(token: &str) -> axum::http::HeaderMap {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {token}").parse().unwrap(),
        );
        headers
    }

    /// A base64url string that is not valid base64url, for the decode branches.
    const NOT_BASE64: &str = "not base64!";

    #[tokio::test]
    async fn health_and_ident_endpoints_report_the_registry_key() {
        let h = harness();
        assert!(health().await.0.ok);

        let ident = server_ident(State(h.state)).await.expect("ident served").0;
        let expected = h.store.server_public_key().unwrap();
        assert_eq!(
            crypto::decode_bytes(&ident.server_key, "serverKey").unwrap(),
            expected,
        );
        assert_eq!(ident.server_fingerprint, crypto::fingerprint(&expected));
        // The published key is the one attestations are signed with, so a
        // client that pins it can verify /signing output.
        let (keypair_public, _private) = h.store.server_keypair().unwrap();
        assert_eq!(keypair_public, expected);
    }

    fn registration_payload(owner: &str) -> (RegisterRequest, Vec<u8>, Vec<u8>) {
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
        (
            RegisterRequest {
                owner: owner.to_string(),
                auth_key: crypto::encode_bytes(&auth_public),
                ident_key: crypto::encode_bytes(&ident_public),
                proofs: RegisterProofs {
                    auth: crypto::encode_bytes(&auth_proof),
                    ident: crypto::encode_bytes(&ident_proof),
                },
            },
            auth_public,
            ident_public,
        )
    }

    #[tokio::test]
    async fn register_handler_creates_the_account_and_rejects_bad_keys() {
        let h = harness();
        let (request, auth_public, ident_public) = registration_payload("alice");
        let response = register(State(h.state.clone()), peer("127.0.0.1"), Json(request))
            .await
            .expect("registration accepted")
            .0;
        assert_eq!(response.owner, "alice");
        assert_eq!(response.auth_fingerprint, crypto::fingerprint(&auth_public));
        assert_eq!(
            response.ident_fingerprint,
            crypto::fingerprint(&ident_public)
        );
        // The account really landed, bound to the ident key it named.
        let (owner, ident_key) = h.store.owner_with_ident_key("alice").unwrap().unwrap();
        assert_eq!(owner.owner_display, "alice");
        assert_eq!(ident_key.public_key, ident_public);

        // Re-registering a taken name is a 409, not a 400 and not a silent
        // takeover of the existing account's keys.
        let (again, _, _) = registration_payload("alice");
        let (status, message) =
            err_of(register(State(h.state.clone()), peer("127.0.0.2"), Json(again)).await);
        assert_eq!(status, StatusCode::CONFLICT);
        assert!(message.contains("already in use"), "{message}");
        assert_eq!(
            h.store
                .owner_with_ident_key("alice")
                .unwrap()
                .unwrap()
                .1
                .public_key,
            ident_public,
        );

        // Every base64 field is decoded, and the diagnostic names which one.
        for (field, index) in [
            ("authKey", 0usize),
            ("identKey", 1),
            ("auth proof", 2),
            ("ident proof", 3),
        ] {
            let (mut request, _, _) = registration_payload("carol");
            match index {
                0 => request.auth_key = NOT_BASE64.to_string(),
                1 => request.ident_key = NOT_BASE64.to_string(),
                2 => request.proofs.auth = NOT_BASE64.to_string(),
                _ => request.proofs.ident = NOT_BASE64.to_string(),
            }
            let (status, message) =
                err_of(register(State(h.state.clone()), peer("127.0.0.3"), Json(request)).await);
            assert_eq!(status, StatusCode::BAD_REQUEST);
            assert_eq!(message, format!("malformed {field}"));
        }

        // Role separation: a proof made for the ident role cannot stand in as
        // the auth proof (and vice versa), so one leaked proof is not two.
        let (mut swapped, _, _) = registration_payload("dave");
        std::mem::swap(&mut swapped.proofs.auth, &mut swapped.proofs.ident);
        let (status, message) =
            err_of(register(State(h.state.clone()), peer("127.0.0.4"), Json(swapped)).await);
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(message.contains("proof-of-possession"), "{message}");
        assert!(h.store.owner_with_ident_key("dave").unwrap().is_none());
    }

    #[tokio::test]
    async fn challenge_handler_issues_a_nonce_and_refuses_unknown_owners() {
        let h = harness();
        let keys = register_owner_with_all_keys(&h.store, "alice");
        let auth_public = crypto::public_from_private(&keys.auth_private).unwrap();

        let (status, message) = err_of(
            challenge(
                State(h.state.clone()),
                Json(ChallengeRequest {
                    owner: "nobody".to_string(),
                    auth_fingerprint: String::new(),
                }),
            )
            .await,
        );
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(message, "unknown owner");

        let issued = challenge(
            State(h.state.clone()),
            Json(ChallengeRequest {
                owner: "alice".to_string(),
                auth_fingerprint: crypto::fingerprint(&auth_public),
            }),
        )
        .await
        .expect("challenge issued")
        .0;
        assert!(!issued.challenge_id.is_empty());
        assert_eq!(
            crypto::decode_bytes(&issued.nonce, "nonce").unwrap().len(),
            32,
        );
        assert!(issued.expires_at > now_unix());

        // A challenge is bound to one specific machine key: an unknown
        // fingerprint gets no nonce to sign.
        let (status, _) = err_of(
            challenge(
                State(h.state),
                Json(ChallengeRequest {
                    owner: "alice".to_string(),
                    auth_fingerprint: crypto::fingerprint(&crypto::generate_keypair().0),
                }),
            )
            .await,
        );
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn login_handler_mints_a_verifiable_session_and_refuses_replay() {
        let h = harness();
        let keys = register_owner_with_all_keys(&h.store, "alice");
        let auth_public = crypto::public_from_private(&keys.auth_private).unwrap();
        let fingerprint = crypto::fingerprint(&auth_public);

        let issued = challenge(
            State(h.state.clone()),
            Json(ChallengeRequest {
                owner: "alice".to_string(),
                auth_fingerprint: fingerprint.clone(),
            }),
        )
        .await
        .unwrap()
        .0;
        let nonce = crypto::decode_bytes(&issued.nonce, "nonce").unwrap();
        let signature = crypto::sign(
            &keys.auth_private,
            &crypto::challenge_message(&issued.challenge_id, &nonce),
        )
        .unwrap();

        // A malformed signature is a 400 and never reaches the store.
        let (status, message) = err_of(
            login(
                State(h.state.clone()),
                peer("10.0.0.1"),
                Json(LoginRequest {
                    challenge_id: issued.challenge_id.clone(),
                    signature: NOT_BASE64.to_string(),
                }),
            )
            .await,
        );
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(message, "malformed signature");

        // A signature by the wrong key does not open a session.
        let (other_public, other_private) = crypto::generate_keypair();
        let _ = other_public;
        let wrong = crypto::sign(
            &other_private,
            &crypto::challenge_message(&issued.challenge_id, &nonce),
        )
        .unwrap();
        let (status, _) = err_of(
            login(
                State(h.state.clone()),
                peer("10.0.0.1"),
                Json(LoginRequest {
                    challenge_id: issued.challenge_id.clone(),
                    signature: crypto::encode_bytes(&wrong),
                }),
            )
            .await,
        );
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let before = now_unix();
        let session = login(
            State(h.state.clone()),
            peer("10.0.0.1"),
            Json(LoginRequest {
                challenge_id: issued.challenge_id.clone(),
                signature: crypto::encode_bytes(&signature),
            }),
        )
        .await
        .expect("login succeeds")
        .0;
        assert_eq!(session.owner, "alice");
        assert!(session.expires_at >= before + 3600);
        let claims = verify_session_token(&h.store, &session.session_token)
            .expect("the minted token is a live session");
        assert_eq!(claims.sub, "alice");
        assert_eq!(claims.auth_fingerprint, fingerprint);
        assert_eq!(claims.iss, SESSION_TOKEN_ISSUER);
        assert_eq!(claims.aud, SESSION_TOKEN_AUDIENCE);
        assert_eq!(claims.exp, session.expires_at);

        // A challenge is single-use: replaying the same signed nonce is a 409,
        // so a captured login cannot be replayed for a second session.
        let (status, message) = err_of(
            login(
                State(h.state),
                peer("10.0.0.1"),
                Json(LoginRequest {
                    challenge_id: issued.challenge_id,
                    signature: crypto::encode_bytes(&signature),
                }),
            )
            .await,
        );
        assert_eq!(status, StatusCode::CONFLICT);
        assert!(message.contains("reused challenge"), "{message}");
    }

    #[tokio::test]
    async fn login_is_throttled_per_client_ip() {
        let h = harness();
        // The gate runs before signature decoding, so malformed attempts still
        // spend the abuser's own bucket — and only their own.
        let attempt = |ip: &'static str| {
            login(
                State(h.state.clone()),
                peer(ip),
                Json(LoginRequest {
                    challenge_id: "no-such-challenge".to_string(),
                    signature: NOT_BASE64.to_string(),
                }),
            )
        };
        let mut last = StatusCode::OK;
        for _ in 0..(LOGIN_PER_IP_MAX + 1) {
            last = err_of(attempt("10.1.1.1").await).0;
        }
        assert_eq!(last, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(err_of(attempt("10.1.1.2").await).0, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn machine_link_relays_an_opaque_blob_exactly_once() {
        let h = harness();
        let keys = register_owner_with_all_keys(&h.store, "alice");
        let token = open_session(&h.store, "alice", &keys.auth_private);
        register_owner_with_all_keys(&h.store, "bob");

        let code = crypto::generate_pairing_code();
        let lookup = crypto::pairing_lookup(&code);
        // The relayed ciphertext is opaque to the server: assert it comes back
        // byte-identical rather than re-deriving it here.
        let blob = vec![7u8; 128];
        let salt = vec![9u8; 16];
        let good_blob = crypto::encode_bytes(&blob);
        let good_salt = crypto::encode_bytes(&salt);
        let start =
            |owner: &str, session: &str, lookup: &str, blob: &str, salt: &str| LinkStartRequest {
                owner: owner.to_string(),
                lookup: lookup.to_string(),
                blob: blob.to_string(),
                salt: salt.to_string(),
                session_token: session.to_string(),
            };

        // An anonymous caller cannot park a blob on an account.
        assert_eq!(
            err_of(
                link_start(
                    State(h.state.clone()),
                    Json(start("alice", "bad.token", &lookup, &good_blob, &good_salt)),
                )
                .await
            )
            .1,
            "expired or malformed session token",
        );
        // Nor can alice's session park one on bob.
        assert_eq!(
            err_of(
                link_start(
                    State(h.state.clone()),
                    Json(start("bob", &token, &lookup, &good_blob, &good_salt)),
                )
                .await
            )
            .1,
            "session owner does not match requested owner",
        );
        for bad_lookup in ["short", &"A".repeat(64)] {
            assert_eq!(
                err_of(
                    link_start(
                        State(h.state.clone()),
                        Json(start("alice", &token, bad_lookup, &good_blob, &good_salt)),
                    )
                    .await
                )
                .1,
                "malformed pairing lookup",
                "{bad_lookup}",
            );
        }
        assert_eq!(
            err_of(
                link_start(
                    State(h.state.clone()),
                    Json(start("alice", &token, &lookup, NOT_BASE64, &good_salt)),
                )
                .await
            )
            .1,
            "malformed blob",
        );
        assert_eq!(
            err_of(
                link_start(
                    State(h.state.clone()),
                    Json(start("alice", &token, &lookup, &good_blob, NOT_BASE64)),
                )
                .await
            )
            .1,
            "malformed salt",
        );
        // Size bounds: an empty blob and an oversized one are both refused, so
        // the relay cannot be used as free storage.
        for bad in [Vec::new(), vec![0u8; 4097]] {
            assert_eq!(
                err_of(
                    link_start(
                        State(h.state.clone()),
                        Json(start(
                            "alice",
                            &token,
                            &lookup,
                            &crypto::encode_bytes(&bad),
                            &good_salt,
                        )),
                    )
                    .await
                )
                .1,
                "malformed pairing blob",
            );
        }

        let started = link_start(
            State(h.state.clone()),
            Json(start("alice", &token, &lookup, &good_blob, &good_salt)),
        )
        .await
        .expect("pairing blob stored")
        .0;
        assert_eq!(started.owner, "alice");
        assert!(started.expires_at > now_unix());

        let (new_public, new_private) = crypto::generate_keypair();
        let proof = crypto::encode_bytes(
            &crypto::sign(
                &new_private,
                &crypto::registration_message(crypto::ROLE_AUTH, "alice", &new_public),
            )
            .unwrap(),
        );
        let key = crypto::encode_bytes(&new_public);
        let fetch = |owner: &str, lookup: &str, auth_key: &str, proof: &str| LinkFetchRequest {
            owner: owner.to_string(),
            lookup: lookup.to_string(),
            auth_key: auth_key.to_string(),
            proof: proof.to_string(),
        };

        assert_eq!(
            err_of(
                link_fetch(
                    State(h.state.clone()),
                    Json(fetch("alice", &lookup, NOT_BASE64, &proof)),
                )
                .await
            )
            .1,
            "malformed authKey",
        );
        assert_eq!(
            err_of(
                link_fetch(
                    State(h.state.clone()),
                    Json(fetch("alice", &lookup, &key, NOT_BASE64)),
                )
                .await
            )
            .1,
            "malformed proof",
        );
        assert_eq!(
            err_of(
                link_fetch(
                    State(h.state.clone()),
                    Json(fetch("nobody", &lookup, &key, &proof)),
                )
                .await
            )
            .1,
            "unknown owner",
        );
        // A proof-of-possession bound to a different account name does not
        // unlock this one (the message is owner-scoped).
        let bob_proof = crypto::encode_bytes(
            &crypto::sign(
                &new_private,
                &crypto::registration_message(crypto::ROLE_AUTH, "bob", &new_public),
            )
            .unwrap(),
        );
        assert_eq!(
            err_of(
                link_fetch(
                    State(h.state.clone()),
                    Json(fetch("alice", &lookup, &key, &bob_proof)),
                )
                .await
            )
            .1,
            "invalid auth proof-of-possession signature",
        );
        // A wrong code finds nothing — and crucially does not burn the pending
        // pairing, which the successful fetch below proves.
        assert_eq!(
            err_of(
                link_fetch(
                    State(h.state.clone()),
                    Json(fetch("alice", &"b".repeat(64), &key, &proof)),
                )
                .await
            )
            .1,
            "unknown, used, or expired pairing code",
        );

        let fetched = link_fetch(
            State(h.state.clone()),
            Json(fetch("alice", &lookup, &key, &proof)),
        )
        .await
        .expect("pairing relayed")
        .0;
        assert_eq!(fetched.owner, "alice");
        assert_eq!(crypto::decode_bytes(&fetched.blob, "blob").unwrap(), blob);
        assert_eq!(crypto::decode_bytes(&fetched.salt, "salt").unwrap(), salt);
        assert_eq!(fetched.auth_fingerprint, crypto::fingerprint(&new_public));

        // The new machine's key is now a first-class credential on the account.
        let new_session =
            open_session_for_key(&h.store, "alice", &new_private, &fetched.auth_fingerprint);
        assert_eq!(
            verify_session_token(&h.store, &new_session).unwrap().sub,
            "alice",
        );
        // And the blob is single-use — a second fetch gets nothing.
        assert_eq!(
            err_of(link_fetch(State(h.state), Json(fetch("alice", &lookup, &key, &proof)),).await)
                .1,
            "unknown, used, or expired pairing code",
        );
    }

    #[tokio::test]
    async fn machine_revocation_is_ident_authorized_and_kills_the_session() {
        let h = harness();
        let keys = register_owner_with_all_keys(&h.store, "alice");
        let token = open_session(&h.store, "alice", &keys.auth_private);
        let auth_public = crypto::public_from_private(&keys.auth_private).unwrap();
        let fingerprint = crypto::fingerprint(&auth_public);

        assert_eq!(
            err_of(
                revoke_challenge(
                    State(h.state.clone()),
                    Json(RevokeChallengeRequest {
                        owner: "nobody".to_string(),
                    }),
                )
                .await
            )
            .1,
            "unknown owner",
        );

        // A fresh ident challenge per attempt: a failed attempt must not be
        // able to reuse a nonce.
        let new_challenge = |state: AppState| async move {
            revoke_challenge(
                State(state),
                Json(RevokeChallengeRequest {
                    owner: "alice".to_string(),
                }),
            )
            .await
            .expect("ident challenge issued")
            .0
        };

        let issued = new_challenge(h.state.clone()).await;
        assert!(issued.expires_at > now_unix());
        assert_eq!(
            err_of(
                revoke_machine(
                    State(h.state.clone()),
                    Json(RevokeRequest {
                        challenge_id: issued.challenge_id.clone(),
                        auth_fingerprint: fingerprint.clone(),
                        ident_signature: NOT_BASE64.to_string(),
                    }),
                )
                .await
            )
            .1,
            "malformed identSignature",
        );

        // The AUTH key is a live credential but NOT the revocation authority:
        // signing the revocation with it is refused (plan-23 §3.6).
        let issued = new_challenge(h.state.clone()).await;
        let nonce = crypto::decode_bytes(&issued.nonce, "nonce").unwrap();
        let by_auth = crypto::sign(
            &keys.auth_private,
            &crypto::revocation_message(&issued.challenge_id, &nonce, &fingerprint),
        )
        .unwrap();
        let (status, _) = err_of(
            revoke_machine(
                State(h.state.clone()),
                Json(RevokeRequest {
                    challenge_id: issued.challenge_id,
                    auth_fingerprint: fingerprint.clone(),
                    ident_signature: crypto::encode_bytes(&by_auth),
                }),
            )
            .await,
        );
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(
            verify_session_token(&h.store, &token).is_ok(),
            "a refused revocation must leave the session alone",
        );

        // An ident-signed revocation of a fingerprint that is not a current
        // auth key is reported rather than silently accepted.
        let issued = new_challenge(h.state.clone()).await;
        let nonce = crypto::decode_bytes(&issued.nonce, "nonce").unwrap();
        let stranger = crypto::fingerprint(&crypto::generate_keypair().0);
        let signature = crypto::sign(
            &keys.ident_private,
            &crypto::revocation_message(&issued.challenge_id, &nonce, &stranger),
        )
        .unwrap();
        assert_eq!(
            err_of(
                revoke_machine(
                    State(h.state.clone()),
                    Json(RevokeRequest {
                        challenge_id: issued.challenge_id,
                        auth_fingerprint: stranger,
                        ident_signature: crypto::encode_bytes(&signature),
                    }),
                )
                .await
            )
            .1,
            "no current auth key with that fingerprint",
        );

        // The happy path: the ident key revokes the machine, and the machine's
        // live session dies with it.
        let issued = new_challenge(h.state.clone()).await;
        let nonce = crypto::decode_bytes(&issued.nonce, "nonce").unwrap();
        let signature = crypto::sign(
            &keys.ident_private,
            &crypto::revocation_message(&issued.challenge_id, &nonce, &fingerprint),
        )
        .unwrap();
        let response = revoke_machine(
            State(h.state),
            Json(RevokeRequest {
                challenge_id: issued.challenge_id,
                auth_fingerprint: fingerprint.clone(),
                ident_signature: crypto::encode_bytes(&signature),
            }),
        )
        .await
        .expect("ident-authorized revocation accepted")
        .0;
        assert_eq!(response.owner, "alice");
        assert_eq!(response.auth_fingerprint, fingerprint);
        assert!(response.revoked);
        assert!(h
            .store
            .owner_auth_key_by_fingerprint("alice", &fingerprint)
            .unwrap()
            .is_none());
        assert!(
            verify_session_token(&h.store, &token).is_err(),
            "revoking the key must invalidate its outstanding session",
        );
    }

    #[tokio::test]
    async fn signed_metadata_is_absent_until_the_root_ceremony_then_verifies() {
        let h = harness();
        // Before the ceremony every signed-metadata endpoint 404s rather than
        // serving an unsigned or empty document.
        assert_eq!(
            err_of(root_metadata(State(h.state.clone())).await).0,
            StatusCode::NOT_FOUND,
        );
        assert_eq!(
            err_of(snapshot_metadata(State(h.state.clone())).await).0,
            StatusCode::NOT_FOUND,
        );
        let (status, message) = err_of(timestamp_metadata(State(h.state.clone())).await);
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(
            message.contains("root of trust is not initialized"),
            "{message}",
        );

        let expires = now_unix() + 86_400;
        h.store
            .init_registry_root("test-registry", expires)
            .unwrap();
        let config = h.store.registry_config().unwrap().unwrap();

        let root = root_metadata(State(h.state.clone()))
            .await
            .expect("root served")
            .0;
        assert_eq!(
            root.root_fingerprint,
            crypto::fingerprint(&config.root_public)
        );
        crypto::verify(
            &crypto::decode_bytes(&root.root_key, "rootKey").unwrap(),
            &crypto::root_signing_input(root.signed.as_bytes()),
            &crypto::decode_bytes(&root.signature, "signature").unwrap(),
        )
        .expect("root.json verifies under the published root key");
        let root_doc: serde_json::Value = serde_json::from_str(&root.signed).unwrap();
        assert_eq!(root_doc["type"], "root");
        assert_eq!(root_doc["registryId"], "test-registry");
        assert_eq!(root_doc["expires"], expires);

        let snapshot = snapshot_metadata(State(h.state.clone()))
            .await
            .expect("snapshot served")
            .0;
        crypto::verify(
            &config.snapshot_public,
            &crypto::snapshot_signing_input(snapshot.signed.as_bytes()),
            &crypto::decode_bytes(&snapshot.signature, "signature").unwrap(),
        )
        .expect("snapshot verifies under the root-delegated snapshot key");
        let snapshot_doc: serde_json::Value = serde_json::from_str(&snapshot.signed).unwrap();
        assert_eq!(snapshot_doc["type"], "snapshot");
        assert_eq!(snapshot_doc["registryId"], "test-registry");
        assert_eq!(
            snapshot_doc["indexHash"],
            h.store.index_canonical_hash().unwrap(),
        );
        assert_eq!(snapshot_doc["version"], h.store.log_size().unwrap());
        assert_eq!(
            snapshot_doc["checkpoint"]["size"],
            h.store.log_leaf_hashes(None).unwrap().len() as i64,
        );
        assert!(snapshot_doc["expires"].as_i64().unwrap() > now_unix());

        let timestamp = timestamp_metadata(State(h.state))
            .await
            .expect("timestamp served")
            .0;
        crypto::verify(
            &config.timestamp_public,
            &crypto::timestamp_signing_input(timestamp.signed.as_bytes()),
            &crypto::decode_bytes(&timestamp.signature, "signature").unwrap(),
        )
        .expect("timestamp verifies under the root-delegated timestamp key");
        let timestamp_doc: serde_json::Value = serde_json::from_str(&timestamp.signed).unwrap();
        assert_eq!(timestamp_doc["type"], "timestamp");
        // The timestamp points at the snapshot the registry is serving now.
        assert_eq!(timestamp_doc["snapshotVersion"], snapshot_doc["version"]);
        assert_eq!(timestamp_doc["indexHash"], snapshot_doc["indexHash"]);
        // ...and it is the short-lived half of the pair.
        assert!(
            timestamp_doc["expires"].as_i64().unwrap() < snapshot_doc["expires"].as_i64().unwrap()
        );
    }

    #[tokio::test]
    async fn log_publish_entry_and_proof_bounds_are_enforced() {
        let h = harness();
        let keys = register_owner_with_all_keys(&h.store, "alice");
        let token = open_session(&h.store, "alice", &keys.auth_private);

        assert_eq!(
            err_of(
                log_publish_entry(
                    State(h.state.clone()),
                    axum::extract::Query(PublishEntryQuery {
                        ident: "alice#toolbox".to_string(),
                        version: "1.0.0".to_string(),
                    }),
                )
                .await
            )
            .1,
            "no publish log entry for that package",
        );

        publish_valid_package(&h.state, &keys, &token, "1.0.0").await;
        let entry = log_publish_entry(
            State(h.state.clone()),
            axum::extract::Query(PublishEntryQuery {
                ident: "alice#toolbox".to_string(),
                version: "1.0.0".to_string(),
            }),
        )
        .await
        .expect("publish entry served")
        .0;
        let stored = h
            .store
            .publish_log_entry("alice#toolbox", "1.0.0")
            .unwrap()
            .unwrap();
        assert_eq!(entry.index, stored.index);
        assert_eq!(entry.leaf_hash, hex::encode(stored.leaf_hash));
        let leaves = h.store.log_leaf_hashes(None).unwrap();
        assert_eq!(hex::encode(leaves[entry.index as usize]), entry.leaf_hash);

        // An index outside the tree is a 400, never a panic on the slice.
        let size = leaves.len() as i64;
        for index in [-1, size] {
            let (status, message) = err_of(
                log_inclusion_proof(
                    State(h.state.clone()),
                    axum::extract::Path(index),
                    axum::extract::Query(ProofQuery { size: None }),
                )
                .await,
            );
            assert_eq!(status, StatusCode::BAD_REQUEST, "index {index}");
            assert_eq!(message, "log entry index is outside the tree");
        }

        // `?size=` serves a proof against a historic head, and the bound moves
        // with it: index 1 is outside a one-leaf tree.
        let historic = log_inclusion_proof(
            State(h.state.clone()),
            axum::extract::Path(0),
            axum::extract::Query(ProofQuery { size: Some(1) }),
        )
        .await
        .expect("historic proof served")
        .0;
        assert_eq!(historic.size, 1);
        assert!(historic.path.is_empty());
        assert_eq!(historic.leaf_hash, hex::encode(leaves[0]));
        assert_eq!(
            err_of(
                log_inclusion_proof(
                    State(h.state.clone()),
                    axum::extract::Path(1),
                    axum::extract::Query(ProofQuery { size: Some(1) }),
                )
                .await
            )
            .1,
            "log entry index is outside the tree",
        );

        for from in [-1, size + 1] {
            let (status, message) = err_of(
                log_consistency_proof(
                    State(h.state.clone()),
                    axum::extract::Query(ConsistencyQuery { from, to: None }),
                )
                .await,
            );
            assert_eq!(status, StatusCode::BAD_REQUEST, "from {from}");
            assert_eq!(message, "consistency proof sizes are invalid");
        }
    }

    #[tokio::test]
    async fn package_index_and_ident_chain_reject_malformed_lookups() {
        let h = harness();
        for ident in ["noseparator", "#toolbox", "alice#"] {
            let (status, message) = err_of(
                package_index(
                    State(h.state.clone()),
                    axum::extract::Path(ident.to_string()),
                )
                .await,
            );
            assert_eq!(status, StatusCode::BAD_REQUEST, "{ident}");
            assert_eq!(message, "ident must use <owner>#<package>");
        }
        assert_eq!(
            err_of(ident_chain(State(h.state), axum::extract::Path("nobody".to_string())).await).1,
            "unknown owner",
        );
    }

    /// Mint a session token carrying `claims` and back it with a live session
    /// row owned by `row_owner_id`/`key_id`. Lets a test present a *validly
    /// signed* token whose claims disagree with the database, which is exactly
    /// what the handlers' cross-checks exist to catch.
    fn session_with_claims(
        store: &Store,
        row_owner_id: i64,
        key_id: i64,
        claims: SessionClaims,
    ) -> String {
        let token = encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(&store.server_secret().unwrap()),
        )
        .unwrap();
        store
            .insert_session(&NewSession {
                owner_id: row_owner_id,
                key_id,
                jwt_id: claims.jti.clone(),
                issued_at: claims.iat,
                expires_at: claims.exp,
            })
            .unwrap();
        token
    }

    #[tokio::test]
    async fn session_preamble_cross_checks_jwt_claims_against_the_database() {
        let h = harness();
        let keys = register_owner_with_all_keys(&h.store, "alice");
        register_owner_with_all_keys(&h.store, "bob");
        let (owner, key) = h.store.owner_with_auth_key("alice").unwrap().unwrap();
        let good = open_session(&h.store, "alice", &keys.auth_private);
        let now = now_unix();
        let claims = |auth_fingerprint: String, owner_id: i64| SessionClaims {
            sub: "alice".to_string(),
            owner_id,
            auth_fingerprint,
            iat: now,
            exp: now + 3600,
            jti: Uuid::new_v4().to_string(),
            iss: SESSION_TOKEN_ISSUER.to_string(),
            aud: SESSION_TOKEN_AUDIENCE.to_string(),
        };
        let stale_key = session_with_claims(
            &h.store,
            owner.id,
            key.id,
            claims(crypto::fingerprint(&crypto::generate_keypair().0), owner.id),
        );
        let wrong_owner_id = session_with_claims(
            &h.store,
            owner.id,
            key.id,
            claims(key.fingerprint.clone(), owner.id + 10_000),
        );

        // `/tokens/revoke` funnels through `session_and_ident`, so it is the
        // cheapest probe of that shared preamble.
        let request = |session: &str, owner: &str| TokenRevokeRequest {
            owner: owner.to_string(),
            token_fingerprint: "0".repeat(64),
            session_token: session.to_string(),
            ident_signature: crypto::encode_bytes(&[0u8; 64]),
        };
        assert_eq!(
            err_of(revoke_token(State(h.state.clone()), Json(request("bad.token", "alice"))).await)
                .1,
            "expired or malformed session token",
        );
        assert_eq!(
            err_of(revoke_token(State(h.state.clone()), Json(request(&good, "bob"))).await).1,
            "session owner does not match requested owner",
        );
        assert_eq!(
            err_of(revoke_token(State(h.state.clone()), Json(request(&stale_key, "alice"))).await)
                .1,
            "session key is not a current auth key",
        );
        assert_eq!(
            err_of(
                revoke_token(
                    State(h.state.clone()),
                    Json(request(&wrong_owner_id, "alice")),
                )
                .await
            )
            .1,
            "session owner does not match requested owner",
        );

        // A well-formed session still needs a real ident signature.
        assert_eq!(
            err_of(revoke_token(State(h.state.clone()), Json(request(&good, "alice"))).await).1,
            "invalid token revocation ident signature",
        );
        let mut malformed = request(&good, "alice");
        malformed.ident_signature = NOT_BASE64.to_string();
        assert_eq!(
            err_of(revoke_token(State(h.state.clone()), Json(malformed)).await).1,
            "malformed identSignature",
        );
        // ...and revoking a fingerprint that names no live token is reported.
        let mut unknown = request(&good, "alice");
        unknown.ident_signature = crypto::encode_bytes(
            &crypto::sign(
                &keys.ident_private,
                &crypto::token_revoke_message("alice", &"0".repeat(64)),
            )
            .unwrap(),
        );
        assert_eq!(
            err_of(revoke_token(State(h.state), Json(unknown)).await).1,
            "no active token with that fingerprint",
        );
    }

    #[tokio::test]
    async fn issue_token_enforces_scope_ownership_and_signature() {
        let h = harness();
        let keys = register_owner_with_all_keys(&h.store, "alice");
        let token = open_session(&h.store, "alice", &keys.auth_private);
        let (token_public, token_private) = crypto::generate_keypair();
        let token_fingerprint = crypto::fingerprint(&token_public);
        let proof = crypto::encode_bytes(
            &crypto::sign(
                &token_private,
                &crypto::registration_message(crypto::ROLE_AUTH, "alice", &token_public),
            )
            .unwrap(),
        );
        let request = |scope: &str| TokenIssueRequest {
            owner: "alice".to_string(),
            token_key: crypto::encode_bytes(&token_public),
            proof: proof.clone(),
            scope: scope.to_string(),
            ttl_seconds: 3600,
            session_token: token.clone(),
            ident_signature: crypto::encode_bytes(
                &crypto::sign(
                    &keys.ident_private,
                    &crypto::token_issue_message("alice", &token_fingerprint, scope),
                )
                .unwrap(),
            ),
        };

        let mut bad_key = request("alice#*");
        bad_key.token_key = NOT_BASE64.to_string();
        assert_eq!(
            err_of(issue_token(State(h.state.clone()), Json(bad_key)).await).1,
            "malformed tokenKey",
        );
        let mut bad_proof = request("alice#*");
        bad_proof.proof = NOT_BASE64.to_string();
        assert_eq!(
            err_of(issue_token(State(h.state.clone()), Json(bad_proof)).await).1,
            "malformed proof",
        );
        let mut bad_signature = request("alice#*");
        bad_signature.ident_signature = NOT_BASE64.to_string();
        assert_eq!(
            err_of(issue_token(State(h.state.clone()), Json(bad_signature)).await).1,
            "malformed identSignature",
        );
        // A signature over a *different* scope cannot authorize this one.
        let mut swapped_scope = request("alice#toolbox");
        swapped_scope.scope = "alice#*".to_string();
        assert_eq!(
            err_of(issue_token(State(h.state.clone()), Json(swapped_scope)).await).1,
            "invalid token issuance ident signature",
        );
        // Scope must live under the issuing owner — including a scope with no
        // owner separator at all.
        for scope in ["bob#*", "noseparator"] {
            assert_eq!(
                err_of(issue_token(State(h.state.clone()), Json(request(scope))).await).1,
                "token scope must be within the issuing owner",
                "{scope}",
            );
        }
        // Store-level validation still applies (a zero TTL is refused).
        let mut zero_ttl = request("alice#*");
        zero_ttl.ttl_seconds = 0;
        let (status, message) = err_of(issue_token(State(h.state.clone()), Json(zero_ttl)).await);
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(message.contains("ttl"), "{message}");

        let issued = issue_token(State(h.state.clone()), Json(request("alice#*")))
            .await
            .expect("wildcard token issued")
            .0;
        assert_eq!(issued.owner, "alice");
        assert_eq!(issued.token_fingerprint, token_fingerprint);
        assert_eq!(issued.scope, "alice#*");
        assert!(issued.expires_at > now_unix());

        // A wildcard token may attest any package of its owner...
        let session = open_session_for_key(&h.store, "alice", &token_private, &token_fingerprint);
        for ident in ["alice#toolbox", "alice#widgets"] {
            let (signing_public, _private) = crypto::generate_keypair();
            let _ = signing(
                State(h.state.clone()),
                Json(SigningRequest {
                    owner: "alice".to_string(),
                    ident: ident.to_string(),
                    version: "1.0.0".to_string(),
                    signing_fingerprint: crypto::fingerprint(&signing_public),
                    session_token: session.clone(),
                }),
            )
            .await
            .unwrap_or_else(|_| panic!("wildcard scope permits {ident}"));
        }
        // ...but never another owner's, even though the ident owner check
        // would otherwise be satisfied by the session owner alone.
        let (signing_public, _private) = crypto::generate_keypair();
        assert_eq!(
            err_of(
                signing(
                    State(h.state),
                    Json(SigningRequest {
                        owner: "alice".to_string(),
                        ident: "bob#toolbox".to_string(),
                        version: "1.0.0".to_string(),
                        signing_fingerprint: crypto::fingerprint(&signing_public),
                        session_token: session,
                    }),
                )
                .await
            )
            .1,
            "publish token scope does not permit this package",
        );
    }

    #[tokio::test]
    async fn an_expired_publish_token_cannot_attest() {
        let h = harness();
        let keys = register_owner_with_all_keys(&h.store, "alice");
        let token = open_session(&h.store, "alice", &keys.auth_private);
        let (token_public, token_private) = crypto::generate_keypair();
        let token_fingerprint = crypto::fingerprint(&token_public);
        let proof = crypto::sign(
            &token_private,
            &crypto::registration_message(crypto::ROLE_AUTH, "alice", &token_public),
        )
        .unwrap();
        let _ = issue_token(
            State(h.state.clone()),
            Json(TokenIssueRequest {
                owner: "alice".to_string(),
                token_key: crypto::encode_bytes(&token_public),
                proof: crypto::encode_bytes(&proof),
                scope: "alice#*".to_string(),
                ttl_seconds: 1,
                session_token: token,
                ident_signature: crypto::encode_bytes(
                    &crypto::sign(
                        &keys.ident_private,
                        &crypto::token_issue_message("alice", &token_fingerprint, "alice#*"),
                    )
                    .unwrap(),
                ),
            }),
        )
        .await
        .expect("one-second token issued");
        let session = open_session_for_key(&h.store, "alice", &token_private, &token_fingerprint);

        // The token's own session JWT is still live (an hour), so only the
        // token expiry can refuse this — which is the point of the check.
        tokio::time::sleep(std::time::Duration::from_millis(2100)).await;
        assert!(verify_session_token(&h.store, &session).is_ok());
        let (signing_public, _private) = crypto::generate_keypair();
        assert_eq!(
            err_of(
                signing(
                    State(h.state),
                    Json(SigningRequest {
                        owner: "alice".to_string(),
                        ident: "alice#toolbox".to_string(),
                        version: "1.0.0".to_string(),
                        signing_fingerprint: crypto::fingerprint(&signing_public),
                        session_token: session,
                    }),
                )
                .await
            )
            .1,
            "publish token has expired",
        );
    }

    #[tokio::test]
    async fn transfer_and_org_endpoints_reject_malformed_and_wrong_signatures() {
        let h = harness();
        let alice = register_owner_with_all_keys(&h.store, "alice");
        let bob = register_owner_with_all_keys(&h.store, "bob");
        let alice_token = open_session(&h.store, "alice", &alice.auth_private);
        let bob_token = open_session(&h.store, "bob", &bob.auth_private);

        let mut offer = TransferOfferRequest {
            ident: "alice#toolbox".to_string(),
            from_owner: "alice".to_string(),
            to_owner: "bob".to_string(),
            session_token: alice_token.clone(),
            ident_signature: NOT_BASE64.to_string(),
        };
        assert_eq!(
            err_of(transfer_offer(State(h.state.clone()), Json(offer)).await).1,
            "malformed identSignature",
        );
        // A signature over a different recipient does not authorize this one.
        offer = TransferOfferRequest {
            ident: "alice#toolbox".to_string(),
            from_owner: "alice".to_string(),
            to_owner: "bob".to_string(),
            session_token: alice_token,
            ident_signature: crypto::encode_bytes(
                &crypto::sign(
                    &alice.ident_private,
                    &crypto::transfer_offer_message("alice#toolbox", "alice", "carol"),
                )
                .unwrap(),
            ),
        };
        assert_eq!(
            err_of(transfer_offer(State(h.state.clone()), Json(offer)).await).1,
            "invalid transfer offer ident signature",
        );

        let mut accept = TransferAcceptRequest {
            ident: "alice#toolbox".to_string(),
            to_owner: "bob".to_string(),
            session_token: bob_token.clone(),
            ident_signature: NOT_BASE64.to_string(),
        };
        assert_eq!(
            err_of(transfer_accept(State(h.state.clone()), Json(accept)).await).1,
            "malformed identSignature",
        );
        // Alice cannot sign bob's acceptance for him.
        accept = TransferAcceptRequest {
            ident: "alice#toolbox".to_string(),
            to_owner: "bob".to_string(),
            session_token: bob_token.clone(),
            ident_signature: crypto::encode_bytes(
                &crypto::sign(
                    &alice.ident_private,
                    &crypto::transfer_accept_message("alice#toolbox", "bob"),
                )
                .unwrap(),
            ),
        };
        assert_eq!(
            err_of(transfer_accept(State(h.state.clone()), Json(accept)).await).1,
            "invalid transfer accept ident signature",
        );
        // A correctly signed acceptance with no pending offer is still refused.
        let unoffered = TransferAcceptRequest {
            ident: "alice#toolbox".to_string(),
            to_owner: "bob".to_string(),
            session_token: bob_token.clone(),
            ident_signature: crypto::encode_bytes(
                &crypto::sign(
                    &bob.ident_private,
                    &crypto::transfer_accept_message("alice#toolbox", "bob"),
                )
                .unwrap(),
            ),
        };
        assert_eq!(
            err_of(transfer_accept(State(h.state.clone()), Json(unoffered)).await).0,
            StatusCode::BAD_REQUEST,
        );

        // The org endpoint decodes and verifies its ident signature the same way.
        let mut member = OrgMemberRequest {
            org: "bob".to_string(),
            grantor: "bob".to_string(),
            member: "alice".to_string(),
            role: "admin".to_string(),
            action: "grant".to_string(),
            session_token: bob_token.clone(),
            ident_signature: NOT_BASE64.to_string(),
        };
        assert_eq!(
            err_of(org_members(State(h.state.clone()), Json(member)).await).1,
            "malformed identSignature",
        );
        member = OrgMemberRequest {
            org: "bob".to_string(),
            grantor: "bob".to_string(),
            member: "alice".to_string(),
            role: "admin".to_string(),
            action: "grant".to_string(),
            session_token: bob_token,
            ident_signature: crypto::encode_bytes(
                &crypto::sign(
                    &bob.ident_private,
                    // signed for the `owner` role, presented as `admin`
                    &crypto::org_role_message("bob", "alice", "owner"),
                )
                .unwrap(),
            ),
        };
        assert_eq!(
            err_of(org_members(State(h.state.clone()), Json(member)).await).1,
            "invalid org role ident signature",
        );
        assert!(h.store.org_member_role("bob", "alice").unwrap().is_none());
    }

    #[tokio::test]
    async fn release_state_rejects_every_malformed_request_shape() {
        let h = harness();
        let keys = register_owner_with_all_keys(&h.store, "alice");
        register_owner_with_all_keys(&h.store, "bob");
        let token = open_session(&h.store, "alice", &keys.auth_private);
        let request = |owner: &str, ident: &str, state: &str, session: &str| ReleaseStateRequest {
            owner: owner.to_string(),
            ident: ident.to_string(),
            version: "1.0.0".to_string(),
            state: state.to_string(),
            session_token: session.to_string(),
            ident_signature: crypto::encode_bytes(
                &crypto::sign(
                    &keys.ident_private,
                    &crypto::release_state_message(ident, "1.0.0", state),
                )
                .unwrap(),
            ),
        };

        assert_eq!(
            err_of(
                release_state(
                    State(h.state.clone()),
                    Json(request("alice", "alice#toolbox", "yanked", "bad.token")),
                )
                .await
            )
            .1,
            "expired or malformed session token",
        );
        assert_eq!(
            err_of(
                release_state(
                    State(h.state.clone()),
                    Json(request("bob", "bob#toolbox", "yanked", &token)),
                )
                .await
            )
            .1,
            "session owner does not match requested owner",
        );
        assert_eq!(
            err_of(
                release_state(
                    State(h.state.clone()),
                    Json(request("alice", "alice#toolbox", "bogus", &token)),
                )
                .await
            )
            .1,
            "state must be one of available, deprecated, or yanked",
        );
        assert_eq!(
            err_of(
                release_state(
                    State(h.state.clone()),
                    Json(request("alice", "noseparator", "yanked", &token)),
                )
                .await
            )
            .1,
            "ident must use <owner>#<package>",
        );
        // An empty package half, and another owner's package, are both refused
        // even though the session itself is valid.
        for ident in ["alice#", "bob#toolbox"] {
            assert_eq!(
                err_of(
                    release_state(
                        State(h.state.clone()),
                        Json(request("alice", ident, "yanked", &token)),
                    )
                    .await
                )
                .1,
                "ident owner does not match session owner",
                "{ident}",
            );
        }
        let mut malformed = request("alice", "alice#toolbox", "yanked", &token);
        malformed.ident_signature = NOT_BASE64.to_string();
        assert_eq!(
            err_of(release_state(State(h.state.clone()), Json(malformed)).await).1,
            "malformed identSignature",
        );
        // A correctly signed change for a version that was never published is
        // refused by the store, and nothing is logged.
        let before = h.store.log_size().unwrap();
        assert_eq!(
            err_of(
                release_state(
                    State(h.state.clone()),
                    Json(request("alice", "alice#toolbox", "yanked", &token)),
                )
                .await
            )
            .0,
            StatusCode::BAD_REQUEST,
        );
        assert_eq!(h.store.log_size().unwrap(), before);
    }

    #[tokio::test]
    async fn rotate_ident_rejects_bad_sessions_and_malformed_material() {
        let h = harness();
        let keys = register_owner_with_all_keys(&h.store, "alice");
        register_owner_with_all_keys(&h.store, "bob");
        let token = open_session(&h.store, "alice", &keys.auth_private);
        let ident_fingerprint = crypto::fingerprint(&keys.ident_public);
        let (new_public, new_private) = crypto::generate_keypair();
        let chain = crypto::encode_bytes(
            &crypto::sign(
                &keys.ident_private,
                &crypto::ident_rotation_message("alice", &ident_fingerprint, &new_public),
            )
            .unwrap(),
        );
        let possession = crypto::encode_bytes(
            &crypto::sign(
                &new_private,
                &crypto::registration_message(crypto::ROLE_IDENT, "alice", &new_public),
            )
            .unwrap(),
        );
        let request = |owner: &str, session: &str| RotateRequest {
            owner: owner.to_string(),
            new_ident_key: crypto::encode_bytes(&new_public),
            chain_signature: chain.clone(),
            possession_proof: possession.clone(),
            session_token: session.to_string(),
        };

        assert_eq!(
            err_of(rotate_ident(State(h.state.clone()), Json(request("alice", "bad.token"))).await)
                .1,
            "expired or malformed session token",
        );
        assert_eq!(
            err_of(rotate_ident(State(h.state.clone()), Json(request("bob", &token))).await).1,
            "session owner does not match requested owner",
        );
        for field in ["newIdentKey", "chainSignature", "possessionProof"] {
            let mut bad = request("alice", &token);
            match field {
                "newIdentKey" => bad.new_ident_key = NOT_BASE64.to_string(),
                "chainSignature" => bad.chain_signature = NOT_BASE64.to_string(),
                _ => bad.possession_proof = NOT_BASE64.to_string(),
            }
            assert_eq!(
                err_of(rotate_ident(State(h.state.clone()), Json(bad)).await).1,
                format!("malformed {field}"),
            );
        }
        // A chain link signed by something other than the retiring ident key
        // cannot re-anchor the account.
        let mut forged = request("alice", &token);
        forged.chain_signature = crypto::encode_bytes(
            &crypto::sign(
                &new_private,
                &crypto::ident_rotation_message("alice", &ident_fingerprint, &new_public),
            )
            .unwrap(),
        );
        assert_eq!(
            err_of(rotate_ident(State(h.state.clone()), Json(forged)).await).0,
            StatusCode::BAD_REQUEST,
        );
        assert_eq!(
            h.store
                .owner_with_ident_key("alice")
                .unwrap()
                .unwrap()
                .1
                .fingerprint,
            ident_fingerprint,
            "a refused rotation must leave the ident binding alone",
        );
    }

    #[tokio::test]
    async fn put_blob_requires_a_well_formed_bearer_header() {
        let h = harness();
        let body = b"payload".to_vec();
        let hash = hex::encode(crypto::sha256(&body));
        let call = |headers: axum::http::HeaderMap| {
            put_blob(
                State(h.state.clone()),
                axum::extract::Path(hash.clone()),
                headers,
                axum::body::Bytes::from(body.clone()),
            )
        };

        assert_eq!(
            err_of(call(axum::http::HeaderMap::new()).await),
            (
                StatusCode::UNAUTHORIZED,
                "missing Authorization header".to_string()
            ),
        );

        let mut basic = axum::http::HeaderMap::new();
        basic.insert(
            axum::http::header::AUTHORIZATION,
            "Basic dXNlcjpwdw".parse().unwrap(),
        );
        assert_eq!(
            err_of(call(basic).await),
            (
                StatusCode::UNAUTHORIZED,
                "Authorization header must be `Bearer <token>`".to_string()
            ),
        );

        let mut blank = axum::http::HeaderMap::new();
        blank.insert(
            axum::http::header::AUTHORIZATION,
            "Bearer    ".parse().unwrap(),
        );
        assert_eq!(
            err_of(call(blank).await).1,
            "Authorization header must be `Bearer <token>`",
        );

        let mut binary = axum::http::HeaderMap::new();
        binary.insert(
            axum::http::header::AUTHORIZATION,
            axum::http::HeaderValue::from_bytes(b"Bearer \xff\xfe").unwrap(),
        );
        assert_eq!(
            err_of(call(binary).await).1,
            "malformed Authorization header"
        );

        // None of the refused shapes stored anything.
        assert!(!h
            .state
            .blob_store
            .exists(&hash, BlobKind::Native)
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn put_blob_is_throttled_per_owner() {
        let h = harness();
        let keys = register_owner_with_all_keys(&h.store, "alice");
        let token = open_session(&h.store, "alice", &keys.auth_private);
        let headers = bearer_headers(&token);

        for index in 0..BLOB_UPLOAD_PER_OWNER_MAX {
            let body = format!("vendored-blob-{index}").into_bytes();
            let hash = hex::encode(crypto::sha256(&body));
            let status = put_blob(
                State(h.state.clone()),
                axum::extract::Path(hash),
                headers.clone(),
                axum::body::Bytes::from(body),
            )
            .await
            .expect("uploads under the cap succeed");
            assert_eq!(status, StatusCode::CREATED);
        }
        let body = b"one too many".to_vec();
        let hash = hex::encode(crypto::sha256(&body));
        let (status, message) = err_of(
            put_blob(
                State(h.state.clone()),
                axum::extract::Path(hash.clone()),
                headers,
                axum::body::Bytes::from(body),
            )
            .await,
        );
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert!(message.contains("rate limit"), "{message}");
        // A throttled upload writes nothing to the datapath.
        assert!(!h
            .state
            .blob_store
            .exists(&hash, BlobKind::Native)
            .await
            .unwrap());
        assert!(h.store.blob_kind(&hash).unwrap().is_none());
    }

    fn put_u16(dst: &mut Vec<u8>, value: u16) {
        dst.extend_from_slice(&value.to_le_bytes());
    }

    fn put_u32(dst: &mut Vec<u8>, value: u32) {
        dst.extend_from_slice(&value.to_le_bytes());
    }

    fn put_u64(dst: &mut Vec<u8>, value: u64) {
        dst.extend_from_slice(&value.to_le_bytes());
    }

    fn put_field(dst: &mut Vec<u8>, value: &[u8]) {
        put_u32(dst, value.len() as u32);
        dst.extend_from_slice(value);
    }

    fn mfpc_string_pool(strings: &[&str]) -> Vec<u8> {
        let mut bytes = Vec::new();
        put_u32(&mut bytes, strings.len() as u32);
        for value in strings {
            put_u32(&mut bytes, value.len() as u32);
            bytes.extend_from_slice(value.as_bytes());
        }
        bytes
    }

    /// A section-10 native-library table declaring `entry_count` entries but
    /// only ever writing one, whose single `vendor` locator names `hash`. An
    /// inflated `entry_count` is how a malformed table is built.
    fn mfpc_vendor_table(entry_count: u32, hash: &[u8; 32]) -> Vec<u8> {
        let mut bytes = Vec::new();
        put_u32(&mut bytes, entry_count);
        put_u32(&mut bytes, 0); // logical name -> string 0
        put_u32(&mut bytes, 1); // one locator
        put_u32(&mut bytes, 2); // os -> string 2
        put_u32(&mut bytes, 3); // arch -> string 3
        bytes.push(0); // libc
        bytes.push(1); // vendor locator
        put_u32(&mut bytes, 1); // source filename -> string 1
        bytes.extend_from_slice(hash);
        bytes
    }

    /// A MANIFEST section 1 record naming `author` and `url` by string-pool id.
    /// The section is a fixed positional record; only those two ids matter to
    /// the registry, so the identity fields are interned id 0.
    fn mfpc_manifest_section(author_id: u32, url_id: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        for _ in 0..6 {
            put_u32(&mut bytes, 0); // name, ident, version, identKey, and both fingerprints
        }
        put_u32(&mut bytes, author_id);
        put_u32(&mut bytes, url_id);
        for _ in 0..6 {
            put_u16(&mut bytes, 0); // binaryRepr / language / minimumRuntime versions
        }
        for _ in 0..5 {
            put_u32(&mut bytes, 0); // dependency, nativeLink, export counts; entry fn and flags
        }
        bytes
    }

    fn mfpc_abi_section(exports: &[(u32, [u8; 32])]) -> Vec<u8> {
        let mut bytes = Vec::new();
        put_u16(&mut bytes, 1); // format version
        put_u16(&mut bytes, 0); // reserved
        put_u32(&mut bytes, exports.len() as u32);
        for (name_index, hash) in exports {
            put_u32(&mut bytes, *name_index);
            put_u16(&mut bytes, 1); // kind
            bytes.extend_from_slice(hash);
        }
        put_u32(&mut bytes, 0); // zero dependency edges
        bytes
    }

    /// Assemble the minimal MFPC container the registry reads out of a
    /// package's `packageBinaryRepr`.
    fn mfpc_container(sections: &[(u16, Vec<u8>)]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"MFPC");
        put_u16(&mut bytes, 2); // major
        put_u16(&mut bytes, 0); // minor
        put_u32(&mut bytes, 0); // flags
        put_u32(&mut bytes, sections.len() as u32);
        let mut data_offset = 16 + sections.len() * 24;
        for (id, data) in sections {
            put_u16(&mut bytes, *id);
            put_u16(&mut bytes, 0);
            put_u32(&mut bytes, 0);
            put_u64(&mut bytes, data_offset as u64);
            put_u64(&mut bytes, data.len() as u64);
            data_offset += data.len();
        }
        for (_id, data) in sections {
            bytes.extend_from_slice(data);
        }
        bytes
    }

    fn clone_request(request: &PackageArtifactRequest) -> PackageArtifactRequest {
        PackageArtifactRequest {
            ident: request.ident.clone(),
            version: request.version.clone(),
            artifact: request.artifact.clone(),
            content_hash: request.content_hash.clone(),
            ident_fingerprint: request.ident_fingerprint.clone(),
            signing_fingerprint: request.signing_fingerprint.clone(),
            session_token: request.session_token.clone(),
        }
    }

    /// Craft a fully valid `alice#toolbox` package around `payload`, with a
    /// genuine `/signing` attestation, and return the artifact plus the wire
    /// request describing it. Nothing is published.
    async fn signed_request(
        state: &AppState,
        keys: &TestOwnerKeys,
        token: &str,
        version: &str,
        payload: Vec<u8>,
    ) -> (Vec<u8>, PackageArtifactRequest) {
        signed_request_with_header_metadata(state, keys, token, version, payload, "alice", "").await
    }

    /// `signed_request`, but with the **header** `author`/`url` under test
    /// control. plan-61-A §4 reads those two fields from the signed MANIFEST
    /// and refuses a package whose header disagrees, so proving that check
    /// works at all requires being able to make the two copies differ.
    async fn signed_request_with_header_metadata(
        state: &AppState,
        keys: &TestOwnerKeys,
        token: &str,
        version: &str,
        payload: Vec<u8>,
        header_author: &str,
        header_url: &str,
    ) -> (Vec<u8>, PackageArtifactRequest) {
        let (signing_public, signing_private) = crypto::generate_keypair();
        let ident_fingerprint = crypto::fingerprint(&keys.ident_public);
        let signing_fingerprint = crypto::fingerprint(&signing_public);
        let (attestation, attestation_sig) =
            real_attestation(state, token, version, &signing_fingerprint).await;
        let proof = format!(
            "{{\"owner\":\"alice\",\"ident\":\"alice#toolbox\",\"version\":\"{version}\",\"identFingerprint\":\"{ident_fingerprint}\",\"signingFingerprint\":\"{signing_fingerprint}\"}}",
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
                version: version.to_string(),
                author: header_author.to_string(),
                url: header_url.to_string(),
                payload,
                ident_key: format!("ed25519:{}", crypto::encode_bytes(&keys.ident_public)),
                signing_key: format!("ed25519:{}", crypto::encode_bytes(&signing_public)),
                proof,
                proof_sig,
                attestation,
                attestation_sig,
            },
            &signing_private,
        );
        let parsed = package::parse_mfp_package(&artifact).expect("crafted package parses");
        let request = PackageArtifactRequest {
            ident: parsed.ident.clone(),
            version: parsed.version.clone(),
            artifact: crypto::encode_bytes(&artifact),
            content_hash: parsed.content_hash_hex(),
            ident_fingerprint: parsed.ident_fingerprint().unwrap(),
            signing_fingerprint: parsed.signing_fingerprint().unwrap(),
            session_token: token.to_string(),
        };
        (artifact, request)
    }

    #[tokio::test]
    async fn validate_handler_reports_every_request_field_mismatch() {
        let h = harness();
        let keys = register_owner_with_all_keys(&h.store, "alice");
        let token = open_session(&h.store, "alice", &keys.auth_private);
        let (_artifact, valid) = signed_request(
            &h.state,
            &keys,
            &token,
            "1.0.0",
            b"MFPCtestpayload".to_vec(),
        )
        .await;

        // The baseline validates through the public handler.
        let report = validate_package(State(h.state.clone()), Json(clone_request(&valid)))
            .await
            .expect("validation ran")
            .0;
        assert!(report.valid, "{:?}", report.diagnostics);
        assert_eq!(report.content_hash, valid.content_hash);

        // Every wire field is cross-checked against the signed header, so a
        // lying request cannot make the registry index the wrong thing.
        let lying = PackageArtifactRequest {
            ident: "alice#other".to_string(),
            version: "9.9.9".to_string(),
            artifact: valid.artifact.clone(),
            content_hash: "0".repeat(64),
            ident_fingerprint: "1".repeat(64),
            signing_fingerprint: "2".repeat(64),
            session_token: token.clone(),
        };
        let report = validate_package(State(h.state.clone()), Json(lying))
            .await
            .expect("validation ran")
            .0;
        assert!(!report.valid);
        for expected in [
            "request contentHash does not match package content hash",
            "request ident does not match package ident",
            "request version does not match package version",
            "request identFingerprint does not match package header",
            "request signingFingerprint does not match package header",
        ] {
            assert!(
                report.diagnostics.iter().any(|d| d == expected),
                "missing `{expected}` in {:?}",
                report.diagnostics,
            );
        }
        // The report still names the real content hash, not the claimed one.
        assert_eq!(report.content_hash, valid.content_hash);

        // A non-base64 artifact is a hard 400; base64 garbage is a diagnostic.
        let mut undecodable = clone_request(&valid);
        undecodable.artifact = NOT_BASE64.to_string();
        let (status, message) =
            err_of(validate_package(State(h.state.clone()), Json(undecodable)).await);
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(message, "malformed artifact");

        let mut garbage = clone_request(&valid);
        garbage.artifact = crypto::encode_bytes(b"not an mfp package at all");
        let report = validate_package(State(h.state), Json(garbage))
            .await
            .expect("validation ran")
            .0;
        assert!(!report.valid);
        assert_eq!(report.content_hash, "");
        assert_eq!(report.abi_index, serde_json::json!({}));
        assert_eq!(report.diagnostics.len(), 1);
    }

    #[tokio::test]
    async fn validate_reports_owner_author_and_duplicate_problems() {
        let h = harness();
        let keys = register_owner_with_all_keys(&h.store, "alice");
        let token = open_session(&h.store, "alice", &keys.auth_private);
        let (owner, key) = h.store.owner_with_auth_key("alice").unwrap().unwrap();

        // A package whose ident owner is not registered at all.
        let (unregistered_public, unregistered_private) = crypto::generate_keypair();
        let (sign_public, sign_private) = crypto::generate_keypair();
        let stranger = package::test_support::serialize(
            &package::test_support::TestPackage {
                name: "toolbox".to_string(),
                ident: "mallory#toolbox".to_string(),
                version: "1.0.0".to_string(),
                author: "mallory".to_string(),
                url: String::new(),
                payload: b"MFPCtestpayload".to_vec(),
                ident_key: format!("ed25519:{}", crypto::encode_bytes(&unregistered_public)),
                signing_key: format!("ed25519:{}", crypto::encode_bytes(&sign_public)),
                proof: "{}".to_string(),
                proof_sig: crypto::sign(&unregistered_private, b"unused").unwrap(),
                attestation: "{}".to_string(),
                attestation_sig: vec![0u8; 64],
            },
            &sign_private,
        );
        let parsed = package::parse_mfp_package(&stranger).unwrap();
        let report = validate_package(
            State(h.state.clone()),
            Json(PackageArtifactRequest {
                ident: parsed.ident.clone(),
                version: parsed.version.clone(),
                artifact: crypto::encode_bytes(&stranger),
                content_hash: parsed.content_hash_hex(),
                ident_fingerprint: parsed.ident_fingerprint().unwrap_or_default(),
                signing_fingerprint: parsed.signing_fingerprint().unwrap_or_default(),
                session_token: token.clone(),
            }),
        )
        .await
        .expect("validation ran")
        .0;
        assert!(!report.valid);
        assert!(
            report
                .diagnostics
                .iter()
                .any(|d| d == "session owner does not match package ident owner"),
            "{:?}",
            report.diagnostics,
        );
        assert!(
            report
                .diagnostics
                .iter()
                .any(|d| d == "package ident owner is not registered"),
            "{:?}",
            report.diagnostics,
        );

        // A validly signed session whose auth fingerprint is no longer one of
        // the owner's current keys stops at "session key does not match".
        let now = now_unix();
        let stale = session_with_claims(
            &h.store,
            owner.id,
            key.id,
            SessionClaims {
                sub: "alice".to_string(),
                owner_id: owner.id,
                auth_fingerprint: crypto::fingerprint(&crypto::generate_keypair().0),
                iat: now,
                exp: now + 3600,
                jti: Uuid::new_v4().to_string(),
                iss: SESSION_TOKEN_ISSUER.to_string(),
                aud: SESSION_TOKEN_AUDIENCE.to_string(),
            },
        );
        let (_artifact, mut request) = signed_request(
            &h.state,
            &keys,
            &token,
            "1.0.0",
            b"MFPCtestpayload".to_vec(),
        )
        .await;
        request.session_token = stale;
        let report = validate_package(State(h.state.clone()), Json(request))
            .await
            .expect("validation ran")
            .0;
        assert!(!report.valid);
        assert_eq!(
            report.diagnostics,
            vec!["session key does not match current owner key".to_string()],
        );

        // An author string that is not the owner's display name is flagged.
        let (signing_public, signing_private) = crypto::generate_keypair();
        let signing_fingerprint = crypto::fingerprint(&signing_public);
        let (attestation, attestation_sig) =
            real_attestation(&h.state, &token, "1.0.0", &signing_fingerprint).await;
        let ident_fingerprint = crypto::fingerprint(&keys.ident_public);
        let proof = format!(
            "{{\"owner\":\"alice\",\"ident\":\"alice#toolbox\",\"version\":\"1.0.0\",\"identFingerprint\":\"{ident_fingerprint}\",\"signingFingerprint\":\"{signing_fingerprint}\"}}",
        );
        let proof_sig = crypto::sign(
            &keys.ident_private,
            &crypto::proof_signing_input(proof.as_bytes()),
        )
        .unwrap();
        let impostor = package::test_support::serialize(
            &package::test_support::TestPackage {
                name: "toolbox".to_string(),
                ident: "alice#toolbox".to_string(),
                version: "1.0.0".to_string(),
                author: "someone-else".to_string(),
                url: String::new(),
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
        let parsed = package::parse_mfp_package(&impostor).unwrap();
        let report = validate_package(
            State(h.state.clone()),
            Json(PackageArtifactRequest {
                ident: parsed.ident.clone(),
                version: parsed.version.clone(),
                artifact: crypto::encode_bytes(&impostor),
                content_hash: parsed.content_hash_hex(),
                ident_fingerprint: parsed.ident_fingerprint().unwrap(),
                signing_fingerprint: parsed.signing_fingerprint().unwrap(),
                session_token: token.clone(),
            }),
        )
        .await
        .expect("validation ran")
        .0;
        assert!(!report.valid);
        assert!(
            report
                .diagnostics
                .iter()
                .any(|d| d == "package author does not match owner name"),
            "{:?}",
            report.diagnostics,
        );

        // Once a version is published, re-validating the same artifact reports
        // the collision instead of silently accepting a re-publish.
        publish_valid_package(&h.state, &keys, &token, "2.0.0").await;
        let (_artifact, republish) = signed_request(
            &h.state,
            &keys,
            &token,
            "2.0.0",
            b"MFPCtestpayload".to_vec(),
        )
        .await;
        let report = validate_package(State(h.state), Json(republish))
            .await
            .expect("validation ran")
            .0;
        assert!(!report.valid);
        assert!(
            report
                .diagnostics
                .iter()
                .any(|d| d == "package version alice#toolbox@2.0.0 is already published"),
            "{:?}",
            report.diagnostics,
        );
    }

    #[tokio::test]
    async fn validate_refuses_an_unsigned_container() {
        let h = harness();
        let keys = register_owner_with_all_keys(&h.store, "alice");
        let token = open_session(&h.store, "alice", &keys.auth_private);

        // `test_support::serialize` always signs, so build the unsigned shape
        // (signatureType 0 / zero-length signature) here.
        let payload = b"MFPCtestpayload".to_vec();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&[0x4d, 0x46, 0x50, 0x0d, 0x0a, 0x1a, 0x0a, 0x00]);
        put_u16(&mut bytes, 1);
        put_u16(&mut bytes, 0);
        put_u16(&mut bytes, 1);
        put_u16(&mut bytes, 0);
        put_u32(&mut bytes, 0);
        put_field(&mut bytes, b"toolbox");
        put_field(&mut bytes, b"alice#toolbox");
        put_field(&mut bytes, b"1.0.0");
        put_field(&mut bytes, b"alice");
        put_field(&mut bytes, b"");
        // An unsigned container may carry none of the trust-chain fields, so
        // every one of them is empty here.
        put_field(&mut bytes, b"");
        put_field(&mut bytes, b"");
        put_field(&mut bytes, b"");
        put_field(&mut bytes, &[]);
        put_field(&mut bytes, b"");
        put_field(&mut bytes, &[]);
        bytes.extend_from_slice(&crypto::sha256(&payload));
        put_u64(&mut bytes, payload.len() as u64);
        put_u16(&mut bytes, 0); // signatureType: unsigned
        put_u32(&mut bytes, 0); // signatureLength
        bytes.extend_from_slice(&payload);

        let parsed = package::parse_mfp_package(&bytes).expect("unsigned container parses");
        assert_eq!(parsed.signature_type, 0);
        let report = validate_package(
            State(h.state),
            Json(PackageArtifactRequest {
                ident: parsed.ident.clone(),
                version: parsed.version.clone(),
                artifact: crypto::encode_bytes(&bytes),
                content_hash: parsed.content_hash_hex(),
                ident_fingerprint: parsed.ident_fingerprint().unwrap_or_default(),
                signing_fingerprint: parsed.signing_fingerprint().unwrap_or_default(),
                session_token: token,
            }),
        )
        .await
        .expect("validation ran")
        .0;
        assert!(!report.valid);
        assert!(
            report
                .diagnostics
                .iter()
                .any(|d| d == "registry publishes require an Ed25519-signed package"),
            "{:?}",
            report.diagnostics,
        );
    }

    #[tokio::test]
    async fn validate_requires_every_vendor_blob_to_be_uploaded_first() {
        let h = harness();
        let keys = register_owner_with_all_keys(&h.store, "alice");
        let token = open_session(&h.store, "alice", &keys.auth_private);

        let vendor_bytes = b"\x7fELF vendored archive".to_vec();
        let vendor_hash = crypto::sha256(&vendor_bytes);
        let vendor_hex = hex::encode(vendor_hash);
        let strings = mfpc_string_pool(&["snd", "libsnd.a", "linux", "aarch64"]);
        let payload = mfpc_container(&[
            (2, strings.clone()),
            (10, mfpc_vendor_table(1, &vendor_hash)),
            (15, mfpc_abi_section(&[(0, [0xab; 32])])),
        ]);

        // The blob is not uploaded yet: the section-10 reference is reported
        // per locator, naming the logical library and the source file.
        let (_artifact, request) =
            signed_request(&h.state, &keys, &token, "1.0.0", payload.clone()).await;
        let report = validate_package(State(h.state.clone()), Json(request))
            .await
            .expect("validation ran")
            .0;
        assert!(!report.valid);
        assert!(
            report.diagnostics.iter().any(|d| d
                == &format!(
                    "native library 'snd' references vendor blob {vendor_hex} (libsnd.a) that is not uploaded"
                )),
            "{:?}",
            report.diagnostics,
        );
        // The ABI index is still parsed and served out of the same payload.
        assert_eq!(report.abi_index["snd"], hex::encode([0xab; 32]));

        // Upload the blob, then the same package validates and publishes, and
        // the version→blob edge makes the vendor blob reachable.
        put_blob(
            State(h.state.clone()),
            axum::extract::Path(vendor_hex.clone()),
            bearer_headers(&token),
            axum::body::Bytes::from(vendor_bytes),
        )
        .await
        .expect("vendor blob uploaded");
        let (_artifact, request) = signed_request(&h.state, &keys, &token, "1.0.0", payload).await;
        let published = publish_package(State(h.state.clone()), Json(request))
            .await
            .expect("publish succeeds once the vendor blob exists")
            .0;
        assert_eq!(published.version, "1.0.0");
        assert!(published.blob_stored);
        assert!(
            h.store.blob_is_reachable(&vendor_hex).unwrap(),
            "the publish must record the version -> vendor blob edge",
        );
        // The served index carries the ABI map the payload declared.
        let index = package_index(
            State(h.state.clone()),
            axum::extract::Path("alice#toolbox".to_string()),
        )
        .await
        .expect("index served")
        .0;
        assert_eq!(
            index.versions[0].abi_map().get("snd").map(String::as_str),
            Some(hex::encode([0xab; 32]).as_str()),
        );

        // A section-10 table that declares more entries than it can hold is a
        // hard diagnostic, not a silently empty vendor list.
        let malformed =
            mfpc_container(&[(2, strings), (10, mfpc_vendor_table(2000, &vendor_hash))]);
        let (_artifact, request) =
            signed_request(&h.state, &keys, &token, "2.0.0", malformed).await;
        let report = validate_package(State(h.state), Json(request))
            .await
            .expect("validation ran")
            .0;
        assert!(!report.valid);
        assert!(
            report
                .diagnostics
                .iter()
                .any(|d| d.starts_with("native library table is malformed")),
            "{:?}",
            report.diagnostics,
        );
    }

    /// plan-61-A Phase 3: `author`/`url` round-trip from the **signed** MANIFEST
    /// into `package_versions`, and an empty value is stored as NULL rather than
    /// `''` — "the publisher set nothing" is one fact, not two.
    #[tokio::test]
    async fn author_and_url_round_trip_from_the_signed_manifest() {
        let h = harness();
        let keys = register_owner_with_all_keys(&h.store, "alice");
        let token = open_session(&h.store, "alice", &keys.auth_private);

        // strings: 0="", 1="alice", 2="https://example.invalid/toolbox"
        let strings = mfpc_string_pool(&["", "alice", "https://example.invalid/toolbox"]);
        let payload = mfpc_container(&[(1, mfpc_manifest_section(1, 2)), (2, strings)]);
        let (_artifact, request) = signed_request_with_header_metadata(
            &h.state,
            &keys,
            &token,
            "1.0.0",
            payload,
            "alice",
            "https://example.invalid/toolbox",
        )
        .await;
        let _ = publish_package(State(h.state.clone()), Json(request))
            .await
            .expect("publish succeeds");

        let (author, url) = h.store.version_metadata_for_test("alice#toolbox", "1.0.0");
        assert_eq!(author.as_deref(), Some("alice"));
        assert_eq!(url.as_deref(), Some("https://example.invalid/toolbox"));

        // An unset `url` stores NULL, not "".
        //
        // Only `url` is exercised here: an empty *author* cannot reach this
        // code at all, because `validate_package_request` already rejects a
        // package whose header author differs from the owner name
        // (`server.rs:2243`), and no owner is named "". The plan asked for
        // "publish with both empty"; that half is unbuildable. See §Corrections.
        let strings = mfpc_string_pool(&["", "alice"]);
        let payload = mfpc_container(&[(1, mfpc_manifest_section(1, 0)), (2, strings)]);
        let (_artifact, request) = signed_request_with_header_metadata(
            &h.state, &keys, &token, "2.0.0", payload, "alice", "",
        )
        .await;
        let _ = publish_package(State(h.state.clone()), Json(request))
            .await
            .expect("publish succeeds with no url");

        let (author, url) = h.store.version_metadata_for_test("alice#toolbox", "2.0.0");
        assert_eq!(author.as_deref(), Some("alice"));
        assert_eq!(url, None, "an empty url is NULL, not the empty string");
    }

    /// plan-61-A §4: the header copy of `author`/`url` is a plaintext fast-scan
    /// convenience; only the MANIFEST copy is covered by the signature. When
    /// the two disagree the artifact is malformed or tampered, and the publish
    /// is refused naming both values — never silently resolved in favour of
    /// either, which is how a transparency registry ends up rendering a string
    /// nobody signed.
    #[tokio::test]
    async fn a_header_manifest_metadata_mismatch_is_refused() {
        let h = harness();
        let keys = register_owner_with_all_keys(&h.store, "alice");
        let token = open_session(&h.store, "alice", &keys.auth_private);

        // The signed manifest says "mallory"; the plaintext header says "alice".
        let strings = mfpc_string_pool(&["", "mallory"]);
        let payload = mfpc_container(&[(1, mfpc_manifest_section(1, 0)), (2, strings)]);
        let (_artifact, request) = signed_request_with_header_metadata(
            &h.state, &keys, &token, "1.0.0", payload, "alice", "",
        )
        .await;

        let (status, body) = publish_package(State(h.state.clone()), Json(request))
            .await
            .expect_err("a metadata mismatch must not publish");
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(
            body.error.contains("header and signed manifest disagree")
                && body.error.contains("alice")
                && body.error.contains("mallory"),
            "the error must name both values: {}",
            body.error,
        );
        // Nothing was persisted: the refusal happens before the version row.
        assert!(h
            .store
            .list_package_versions("alice#toolbox")
            .unwrap()
            .is_empty());
    }

    /// plan-61-A Phase 2: `POST /validate` is a **dry run**. It reads section-10
    /// locators — that is how it reports missing vendor blobs before the
    /// publisher uploads anything — but it holds no transaction and no
    /// `package_version_id`, and it must never write a target row. Persisting
    /// there would invert its documented non-mutating contract, so the guard is
    /// a test rather than a comment.
    #[tokio::test]
    async fn validate_writes_no_target_rows_but_publish_does() {
        let h = harness();
        let keys = register_owner_with_all_keys(&h.store, "alice");
        let token = open_session(&h.store, "alice", &keys.auth_private);

        let vendor_bytes = b"\x7fELF vendored archive".to_vec();
        let vendor_hash = crypto::sha256(&vendor_bytes);
        let vendor_hex = hex::encode(vendor_hash);
        let strings = mfpc_string_pool(&["snd", "libsnd.a", "linux", "aarch64"]);
        let payload = mfpc_container(&[
            (2, strings),
            (10, mfpc_vendor_table(1, &vendor_hash)),
            (15, mfpc_abi_section(&[(0, [0xab; 32])])),
        ]);

        // Upload the blob first, so validation passes and the only reason the
        // table could stay empty is that `/validate` does not write to it.
        put_blob(
            State(h.state.clone()),
            axum::extract::Path(vendor_hex),
            bearer_headers(&token),
            axum::body::Bytes::from(vendor_bytes),
        )
        .await
        .expect("vendor blob uploaded");

        let (_artifact, request) =
            signed_request(&h.state, &keys, &token, "1.0.0", payload.clone()).await;
        let report = validate_package(State(h.state.clone()), Json(request))
            .await
            .expect("validation ran")
            .0;
        assert!(report.valid, "{:?}", report.diagnostics);

        assert!(
            h.store.target_rows_for_test().is_empty(),
            "a dry run must not persist targets",
        );

        // The same artifact through `/publish` does write them — otherwise the
        // assertion above would pass for the wrong reason.
        let (_artifact, request) = signed_request(&h.state, &keys, &token, "1.0.0", payload).await;
        let _ = publish_package(State(h.state.clone()), Json(request))
            .await
            .expect("publish succeeds");

        let rows = h.store.target_rows_for_test();
        assert_eq!(rows.len(), 1);
        let (os, arch, libc, lib_type, source) = &rows[0];
        assert_eq!(os, "linux");
        assert_eq!(arch.as_deref(), Some("aarch64"));
        assert_eq!(*libc, None, "the fixture locator declares no libc");
        assert_eq!(lib_type, "vendor");
        assert_eq!(source, "libsnd.a");
    }

    #[tokio::test]
    async fn publish_refuses_invalid_packages_and_tolerates_a_pre_staged_blob() {
        let h = harness();
        let keys = register_owner_with_all_keys(&h.store, "alice");
        let token = open_session(&h.store, "alice", &keys.auth_private);

        // A package that fails validation never reaches the store.
        let (_artifact, mut broken) = signed_request(
            &h.state,
            &keys,
            &token,
            "1.0.0",
            b"MFPCtestpayload".to_vec(),
        )
        .await;
        broken.content_hash = "0".repeat(64);
        let (status, message) = err_of(publish_package(State(h.state.clone()), Json(broken)).await);
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(
            message.starts_with("package validation failed: "),
            "{message}"
        );
        assert!(h
            .store
            .list_package_versions("alice#toolbox")
            .unwrap()
            .is_empty());

        // A blob left behind by an earlier, half-finished publish is reused
        // rather than re-staged: the version row still commits, and the
        // response reports that no new blob was written.
        let (artifact, request) = signed_request(
            &h.state,
            &keys,
            &token,
            "1.0.0",
            b"MFPCtestpayload".to_vec(),
        )
        .await;
        let hash = request.content_hash.clone();
        let staged = h
            .state
            .blob_store
            .stage(&hash, BlobKind::Package, artifact.clone())
            .await
            .unwrap();
        h.state.blob_store.promote(staged).await.unwrap();
        let published = publish_package(State(h.state.clone()), Json(request))
            .await
            .expect("publish succeeds over an existing blob")
            .0;
        assert!(!published.blob_stored);
        assert_eq!(published.hash, hash);
        assert_eq!(published.state, "available");
        assert!(published.warnings.is_empty());
        // ...and the bytes are still exactly the artifact.
        let response = package_blob(State(h.state.clone()), axum::extract::Path(hash.clone()))
            .await
            .expect("blob served");
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(body.as_ref(), artifact.as_slice());

        // Re-uploading that same content hash as a *native* blob is a no-op:
        // the row already serves it as a package, so promoting a `.bin` would
        // leave a second, unreferenced copy behind (bug-276 R5).
        let status = put_blob(
            State(h.state.clone()),
            axum::extract::Path(hash.clone()),
            bearer_headers(&token),
            axum::body::Bytes::from(artifact),
        )
        .await
        .expect("the no-op upload reports success");
        assert_eq!(status, StatusCode::OK);
        assert!(h.packages_dir.join(format!("{hash}.mfp")).exists());
        assert!(
            !h.packages_dir.join(format!("{hash}.bin")).exists(),
            "a package blob must not gain a duplicate .bin copy",
        );
        assert_eq!(
            h.store.blob_kind(&hash).unwrap().as_deref(),
            Some("package")
        );
    }

    #[tokio::test]
    async fn publish_aborts_the_staged_blob_when_the_ident_moved_owner() {
        let h = harness();
        let alice = register_owner_with_all_keys(&h.store, "alice");
        let bob = register_owner_with_all_keys(&h.store, "bob");
        let alice_token = open_session(&h.store, "alice", &alice.auth_private);
        let bob_token = open_session(&h.store, "bob", &bob.auth_private);
        publish_valid_package(&h.state, &alice, &alice_token, "1.0.0").await;

        // Hand the package to bob.
        let _ = transfer_offer(
            State(h.state.clone()),
            Json(TransferOfferRequest {
                ident: "alice#toolbox".to_string(),
                from_owner: "alice".to_string(),
                to_owner: "bob".to_string(),
                session_token: alice_token.clone(),
                ident_signature: crypto::encode_bytes(
                    &crypto::sign(
                        &alice.ident_private,
                        &crypto::transfer_offer_message("alice#toolbox", "alice", "bob"),
                    )
                    .unwrap(),
                ),
            }),
        )
        .await
        .expect("offer");
        let _ = transfer_accept(
            State(h.state.clone()),
            Json(TransferAcceptRequest {
                ident: "alice#toolbox".to_string(),
                to_owner: "bob".to_string(),
                session_token: bob_token,
                ident_signature: crypto::encode_bytes(
                    &crypto::sign(
                        &bob.ident_private,
                        &crypto::transfer_accept_message("alice#toolbox", "bob"),
                    )
                    .unwrap(),
                ),
            }),
        )
        .await
        .expect("accept");

        // Alice's session still validates the package (she still holds the
        // ident named in the header), but the store refuses the write — and the
        // staged blob must be aborted, leaving no orphan in the datapath.
        let (_artifact, request) = signed_request(
            &h.state,
            &alice,
            &alice_token,
            "2.0.0",
            b"MFPCtestpayload".to_vec(),
        )
        .await;
        let hash = request.content_hash.clone();
        let (status, message) =
            err_of(publish_package(State(h.state.clone()), Json(request)).await);
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(message.contains("owned by another owner"), "{message}");
        assert!(
            !h.packages_dir.join(format!("{hash}.mfp")).exists(),
            "a refused publish must not leave a servable blob behind",
        );
        assert!(!h
            .state
            .blob_store
            .exists(&hash, BlobKind::Package)
            .await
            .unwrap());
        assert!(!h
            .store
            .package_version_exists("alice#toolbox", "2.0.0")
            .unwrap());
    }

    #[tokio::test]
    async fn signing_validates_ident_version_and_fingerprint_shapes() {
        let h = harness();
        let keys = register_owner_with_all_keys(&h.store, "alice");
        let token = open_session(&h.store, "alice", &keys.auth_private);
        let (signing_public, _private) = crypto::generate_keypair();
        let fingerprint = crypto::fingerprint(&signing_public);
        let request = |ident: &str, version: &str, signing_fingerprint: &str| SigningRequest {
            owner: "alice".to_string(),
            ident: ident.to_string(),
            version: version.to_string(),
            signing_fingerprint: signing_fingerprint.to_string(),
            session_token: token.clone(),
        };

        assert_eq!(
            err_of(
                signing(
                    State(h.state.clone()),
                    Json(request("noseparator", "1.0.0", &fingerprint)),
                )
                .await
            )
            .1,
            "ident must use <owner>#<package>",
        );
        // An empty package half, and an over-long ident, are both malformed.
        for ident in ["alice#".to_string(), format!("alice#{}", "p".repeat(250))] {
            assert_eq!(
                err_of(
                    signing(
                        State(h.state.clone()),
                        Json(request(&ident, "1.0.0", &fingerprint)),
                    )
                    .await
                )
                .1,
                "malformed ident",
                "{ident}",
            );
        }
        for version in ["".to_string(), "9".repeat(65)] {
            assert_eq!(
                err_of(
                    signing(
                        State(h.state.clone()),
                        Json(request("alice#toolbox", &version, &fingerprint)),
                    )
                    .await
                )
                .1,
                "malformed version",
                "{version}",
            );
        }
        // The one-off key fingerprint the server is about to put its name on
        // must be 64 lowercase hex characters.
        for bad in ["deadbeef".to_string(), fingerprint.to_uppercase()] {
            assert_eq!(
                err_of(
                    signing(
                        State(h.state.clone()),
                        Json(request("alice#toolbox", "1.0.0", &bad)),
                    )
                    .await
                )
                .1,
                "signingFingerprint must be 64 lowercase hex characters",
                "{bad}",
            );
        }
        // Nothing above was logged: a refused request never records a signing
        // intent, let alone issues an attestation.
        assert_eq!(h.store.log_size().unwrap(), 1);
    }

    #[tokio::test]
    async fn signing_is_rate_limited_per_owner() {
        let h = harness();
        register_owner_with_all_keys(&h.store, "alice");
        // The gate runs before session verification, so even rejected requests
        // spend the owner's budget — the point of the cap is CPU, not success.
        let attempt = || {
            signing(
                State(h.state.clone()),
                Json(SigningRequest {
                    owner: "alice".to_string(),
                    ident: "alice#toolbox".to_string(),
                    version: "1.0.0".to_string(),
                    signing_fingerprint: "0".repeat(64),
                    session_token: "bad.token".to_string(),
                }),
            )
        };
        let mut last = StatusCode::OK;
        for _ in 0..61 {
            last = err_of(attempt().await).0;
        }
        assert_eq!(last, StatusCode::TOO_MANY_REQUESTS);
        // A different owner has its own bucket.
        let other = signing(
            State(h.state),
            Json(SigningRequest {
                owner: "bob".to_string(),
                ident: "bob#toolbox".to_string(),
                version: "1.0.0".to_string(),
                signing_fingerprint: "0".repeat(64),
                session_token: "bad.token".to_string(),
            }),
        )
        .await;
        assert_eq!(err_of(other).0, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn token_scope_matching_is_owner_folded_and_separator_strict() {
        assert!(scope_owner_matches("alice#toolbox", "alice"));
        assert!(scope_owner_matches("Alice#toolbox", "alice"));
        assert!(!scope_owner_matches("bob#toolbox", "alice"));
        assert!(!scope_owner_matches("noseparator", "alice"));

        assert!(scope_permits("alice#*", "alice#toolbox"));
        assert!(scope_permits("alice#toolbox", "alice#toolbox"));
        assert!(!scope_permits("alice#toolbox", "alice#other"));
        assert!(!scope_permits("alice#*", "bob#toolbox"));
        // A scope or an ident without the separator matches nothing.
        assert!(!scope_permits("noseparator", "alice#toolbox"));
        assert!(!scope_permits("alice#*", "noseparator"));
    }

    #[test]
    fn rate_limiter_prune_drops_only_fully_elapsed_windows() {
        let limiter = RateLimiter::new();
        assert!(limiter.allow("key", 1, 60));
        assert!(!limiter.allow("key", 1, 60), "the cap is one per window");

        // A prune with a window the entry still falls inside keeps counting it.
        limiter.prune(3600);
        assert!(!limiter.allow("key", 1, 60));

        // A prune whose window has fully elapsed drops the key entirely.
        limiter.prune(0);
        assert!(
            limiter.allow("key", 1, 60),
            "the elapsed window was released"
        );
    }

    #[test]
    fn error_helpers_map_store_messages_to_status_codes() {
        assert_eq!(bad_request("nope".to_string()).0, StatusCode::BAD_REQUEST);
        assert_eq!(
            internal("boom".to_string()).0,
            StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_eq!(unauthorized("no").0, StatusCode::UNAUTHORIZED);
        // Only the two collision messages become a 409; everything else is a
        // client error, so a store failure is never reported as a conflict.
        assert_eq!(
            conflict_or_bad_request("owner name 'alice' is already in use".to_string()).0,
            StatusCode::CONFLICT,
        );
        assert_eq!(
            conflict_or_bad_request("reused challenge".to_string()).0,
            StatusCode::CONFLICT,
        );
        assert_eq!(
            conflict_or_bad_request("unknown owner".to_string()).0,
            StatusCode::BAD_REQUEST,
        );
        assert_eq!(too_many_requests().0, StatusCode::TOO_MANY_REQUESTS);
    }

    #[test]
    fn blob_hash_paths_must_be_lowercase_hex() {
        validate_blob_hash(&"a".repeat(64)).expect("64 lowercase hex characters are accepted");
        for bad in [
            "a".repeat(63),
            "a".repeat(65),
            "A".repeat(64),
            "g".repeat(64),
        ] {
            assert_eq!(
                err_of(validate_blob_hash(&bad)),
                (
                    StatusCode::BAD_REQUEST,
                    "blob hash must be 64 lowercase hex characters".to_string()
                ),
                "{bad}",
            );
        }
    }

    /// Drive the real `serve()` entry point over a loopback socket: the router
    /// wiring, the `ConnectInfo` plumbing the per-IP throttles depend on, and
    /// the JSON codec on the wire are only exercised through an actual request.
    #[tokio::test]
    async fn serve_binds_the_router_and_answers_over_http() {
        let h = harness();
        // `serve()` runs its accept loop forever, so it never reports the port
        // it bound; take a free one from the OS, release it, and hand it over.
        let probe = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = probe.local_addr().unwrap();
        drop(probe);

        let store = h.store.clone();
        let blob_store = BlobStore::local(h.packages_dir.clone());
        tokio::spawn(async move {
            let _ = serve(store, blob_store, addr).await;
        });

        let base = format!("http://{addr}");
        let client = reqwest::Client::new();
        // Wait for the accept loop to come up.
        let mut health = None;
        for _ in 0..100 {
            match client.get(format!("{base}/health")).send().await {
                Ok(response) => {
                    health = Some(response);
                    break;
                }
                Err(_) => tokio::time::sleep(std::time::Duration::from_millis(20)).await,
            }
        }
        let health = health.expect("the served router accepts connections");
        assert_eq!(health.status(), reqwest::StatusCode::OK);
        assert_eq!(
            health.json::<serde_json::Value>().await.unwrap(),
            serde_json::json!({ "ok": true }),
        );

        // A GET route reading state.
        let ident: ServerIdentResponse = client
            .get(format!("{base}/ident"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(
            ident.server_fingerprint,
            crypto::fingerprint(&h.store.server_public_key().unwrap()),
        );

        // The full anonymous onboarding chain over the wire: register, then
        // challenge, then login. `/accounts/register` and `/auth/login` are the
        // two routes that need `ConnectInfo`, so this also proves the
        // connect-info make-service is wired up.
        let (payload, auth_private) = registration_payload_with_auth_private("alice");
        let registered = client
            .post(format!("{base}/accounts/register"))
            .json(&payload)
            .send()
            .await
            .unwrap();
        assert_eq!(registered.status(), reqwest::StatusCode::OK);
        let registered: RegisterResponse = registered.json().await.unwrap();
        assert_eq!(registered.owner, "alice");

        let challenge: ChallengeResponse = client
            .post(format!("{base}/auth/challenge"))
            .json(&ChallengeRequest {
                owner: "alice".to_string(),
                auth_fingerprint: registered.auth_fingerprint.clone(),
            })
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let nonce = crypto::decode_bytes(&challenge.nonce, "nonce").unwrap();
        let signature = crypto::sign(
            &auth_private,
            &crypto::challenge_message(&challenge.challenge_id, &nonce),
        )
        .unwrap();
        let session: LoginResponse = client
            .post(format!("{base}/auth/login"))
            .json(&LoginRequest {
                challenge_id: challenge.challenge_id,
                signature: crypto::encode_bytes(&signature),
            })
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(
            verify_session_token(&h.store, &session.session_token)
                .expect("the token minted over HTTP is a live session")
                .sub,
            "alice",
        );

        // An unrouted path is a 404 rather than a 500 or a hang.
        assert_eq!(
            client
                .get(format!("{base}/no/such/route"))
                .send()
                .await
                .unwrap()
                .status(),
            reqwest::StatusCode::NOT_FOUND,
        );
        // ...and an unknown blob hash 404s through the real `/blob/:hash` route.
        assert_eq!(
            client
                .get(format!("{base}/blob/{}", "0".repeat(64)))
                .send()
                .await
                .unwrap()
                .status(),
            reqwest::StatusCode::NOT_FOUND,
        );
    }

    /// Like `registration_payload`, but also hands back the auth private key so
    /// the caller can go on to sign a login challenge.
    fn registration_payload_with_auth_private(owner: &str) -> (RegisterRequest, Vec<u8>) {
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
        (
            RegisterRequest {
                owner: owner.to_string(),
                auth_key: crypto::encode_bytes(&auth_public),
                ident_key: crypto::encode_bytes(&ident_public),
                proofs: RegisterProofs {
                    auth: crypto::encode_bytes(&auth_proof),
                    ident: crypto::encode_bytes(&ident_proof),
                },
            },
            auth_private,
        )
    }
}
