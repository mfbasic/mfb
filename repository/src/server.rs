use crate::blobstore::{BlobFetch, BlobKind, BlobStore};
use crate::store::{now_unix, NewSession, PairingFetch, Store};
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
    /// Base64url Ed25519 PUBLIC half of the code-derived approval keypair
    /// (bug-583). The old machine derives it from the code it just generated;
    /// the server keeps it as the verifier for the fetch half. It is public
    /// by construction — holding it does not let the relay sign anything.
    #[serde(rename = "approvalKey")]
    pub approval_key: String,
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
    /// Base64url Ed25519 signature over
    /// `crypto::pairing_approval_message(lookup, authKey)`, made with the
    /// private half derived from the typed pairing code (bug-583). This — not
    /// the server-visible lookup — is the pairing approval.
    pub approval: String,
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

#[derive(Debug, Serialize, Deserialize, Clone)]
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

/// Two-sided ownership transfer (plan-10-D1, signed terms since plan-168-A).
/// Both halves are ident-signed over the same v2 terms and logged; the accepted
/// pair is kept in the append-only transfer history `/index` serves, and
/// already-published versions keep verifying against the old ident.
///
/// `sequence` and `releaseDigest` are the terms the offer signs, which the
/// client computes from `/index` (`transfers.len()` and
/// `crypto::release_digest` over `versions`); the registry refuses terms that
/// no longer match its own.
#[derive(Debug, Serialize, Deserialize)]
pub struct TransferOfferRequest {
    pub ident: String,
    #[serde(rename = "fromOwner")]
    pub from_owner: String,
    #[serde(rename = "toOwner")]
    pub to_owner: String,
    pub sequence: i64,
    #[serde(rename = "releaseDigest")]
    pub release_digest: String,
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
    /// The package's accepted ownership transfers, oldest first (plan-168-A).
    /// Empty for a package that never moved, and absent from an older
    /// registry's response, which reads the same way: the prefix owner (the
    /// top-level `owner`) is the only owner.
    #[serde(default)]
    pub transfers: Vec<TransferLink>,
}

/// One accepted ownership transfer as `/index` serves it (plan-168-A §4.4).
///
/// `offerSignature` is the from-owner's signature under `fromIdentKey`, and
/// `acceptSignature` the to-owner's under `toIdentKey`, both over the v2
/// messages (`crypto::transfer_offer_message_v2` / `_accept_`) on `(ident,
/// sequence, fromOwner, fp(fromIdentKey), toOwner, releaseDigest)`. A link
/// whose keys, digest and signatures are all null is a LEGACY link replayed
/// from the transparency log: nothing about it can be authenticated.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct TransferLink {
    pub sequence: i64,
    #[serde(rename = "fromOwner")]
    pub from_owner: String,
    /// `ed25519:<base64url>`, or null for a legacy link.
    #[serde(rename = "fromIdentKey")]
    pub from_ident_key: Option<String>,
    #[serde(rename = "toOwner")]
    pub to_owner: String,
    /// `ed25519:<base64url>`, or null for a legacy link.
    #[serde(rename = "toIdentKey")]
    pub to_ident_key: Option<String>,
    #[serde(rename = "releaseDigest")]
    pub release_digest: Option<String>,
    #[serde(rename = "offerSignature")]
    pub offer_signature: Option<String>,
    #[serde(rename = "acceptSignature")]
    pub accept_signature: Option<String>,
    #[serde(rename = "offeredAt")]
    pub offered_at: i64,
    #[serde(rename = "acceptedAt")]
    pub accepted_at: i64,
    /// The to-owner's CURRENT ident under a fresh server name binding, so a
    /// client can trust an intermediate owner's newest key without a second,
    /// unsigned `/idents` fetch.
    #[serde(rename = "toBinding")]
    pub to_binding: OwnerBinding,
}

/// A server-signed name binding: `nameBindingSignature` is the registry
/// server key's signature over `crypto::name_binding_message(owner,
/// identFingerprint)` (plan-168-A §4.4).
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct OwnerBinding {
    pub owner: String,
    /// `ed25519:<base64url>`.
    #[serde(rename = "identKey")]
    pub ident_key: String,
    #[serde(rename = "identFingerprint")]
    pub ident_fingerprint: String,
    #[serde(rename = "nameBindingSignature")]
    pub name_binding_signature: String,
}

/// Who published one release (plan-168-A §4.4). `identFingerprint` is null for
/// a release published before the registry recorded publisher keys; clients
/// verify the downloaded header's `identKey`, never this field.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct Publisher {
    pub owner: String,
    #[serde(rename = "identFingerprint")]
    pub ident_fingerprint: Option<String>,
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
    /// The newest **active** release (plan-126-A) — `null` when every published
    /// version is yanked, blocked or legal-tombstoned, which is a statement
    /// about the package rather than missing data. `null` was already a possible
    /// value here (a package with no versions), so no consumer gains a new shape.
    #[serde(rename = "latestVersion")]
    pub latest_version: Option<String>,
    /// The release state of `latestVersion`, so a `deprecated` headline is
    /// visibly deprecated rather than reading as current. `null` exactly when
    /// `latestVersion` is (plan-126-A).
    #[serde(rename = "latestState")]
    pub latest_state: Option<String>,
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
    /// The publishing owner (plan-168-A); absent from an older registry.
    #[serde(default)]
    pub publisher: Option<Publisher>,
    /// The number of accepted transfers before this release was published
    /// (plan-168-A); 0 from an older registry, which never recorded one.
    #[serde(default)]
    pub epoch: u32,
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

/// The `/validate` diagnostic for a release by a later owner of a package that
/// moved through an unsigned (legacy) transfer (plan-168-B, B-D3).
const LEGACY_TRANSFER_PUBLISH_REFUSED: &str = "package ownership moved before signed transfers, \
     so clients cannot verify a release by a later owner";

/// Maximum inline request body (plan-10-D2): caps the base64 artifact carried
/// by `/validate` and `/publish` so a single upload cannot exhaust memory.
const MAX_BODY_BYTES: usize = 64 * 1024 * 1024;

/// Anonymous transparency-log routes, shared per-IP budget (bug-579).
///
/// Sized from the WORST LEGITIMATE CLIENT, not from a round number. A single
/// `verify_publish_inclusion` makes three log requests (`/log/checkpoint`,
/// `/log/publish`, `/log/proof/:index`) and `mfb pkg install --proof` calls it
/// once PER DEPENDENCY, so a 200-dependency install is a ~600-request burst in
/// seconds — and every developer behind one NAT or one CI egress IP shares this
/// key. A tight budget here would not harden the registry, it would break
/// installs. 1200/minute leaves 2x headroom over that worst case while still
/// converting "unbounded" into "bounded".
///
/// Note this grew with bug-582: a client now fetches a consistency proof on
/// every pin advance, so the log is contacted more than it used to be. Any
/// future tightening has to be re-derived from the client, not guessed.
const LOG_PER_IP_MAX: usize = 1200;

/// Per-client (peer-IP) rate caps on the anonymous auth endpoints, replacing the
/// old global-string buckets that let one client lock the whole user base out of
/// registration/login (audit-2 REPO-12 / bug-188). A generous global ceiling is
/// kept as a secondary backstop so extreme aggregate abuse is still bounded, but
/// it is high enough never to trip on one abuser's traffic.
const REGISTER_PER_IP_MAX: usize = 20;
const LOGIN_PER_IP_MAX: usize = 30;
/// Per-IP caps on `/auth/challenge` and `/signing` (bug-419). Both were
/// previously keyed on the **unauthenticated** `request.owner`, so an anonymous
/// attacker could exhaust a targeted victim's bucket — locking that account out
/// of login (a challenge is the mandatory login prerequisite) or of publish
/// attestations. Re-keyed to the peer IP, following `REGISTER_PER_IP_MAX` /
/// `LOGIN_PER_IP_MAX`, with `AUTH_GLOBAL_CEILING` as the secondary backstop.
/// Same numeric budgets the owner-keyed buckets used, now spent per client.
const CHALLENGE_PER_IP_MAX: usize = 20;
const SIGNING_PER_IP_MAX: usize = 60;
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
        .route("/p/:ident/docs", get(package_docs_html))
        .route("/search", get(search))
        .route("/index/:ident", get(package_index))
        // plan-61-B: anonymous read surface. These read no credential of any
        // kind — no session token, no bearer header, no cookie — and are `GET`
        // only. `:ident` sits beside the static `/packages/transfer/*` routes
        // below; matchit resolves static before param.
        .route("/packages/:ident", get(package_detail))
        .route("/packages/:ident/audit", get(package_audit))
        .route("/packages/:ident/docs", get(package_docs))
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
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Json(request): Json<ChallengeRequest>,
) -> Result<Json<ChallengeResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Per-client bucket keyed by peer IP (bug-419): the key was the
    // unauthenticated `request.owner`, letting an anonymous attacker spend a
    // targeted victim's challenge budget and block the victim's own login,
    // since a challenge is the mandatory prerequisite for `/auth/login`. The
    // global ceiling is a secondary backstop, as on `/register` and `/login`.
    if !state.rate_limiter.allow(
        &format!("challenge:{}", peer.ip()),
        CHALLENGE_PER_IP_MAX,
        60,
    ) || !state
        .rate_limiter
        .allow("challenge", AUTH_GLOBAL_CEILING, 60)
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
struct ProofQuery {
    size: Option<i64>,
}

async fn log_inclusion_proof(
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
    /// The newest **active** release (plan-126-A) — `null` for a package with no
    /// published version, and for one whose every version is yanked, blocked or
    /// legal-tombstoned.
    #[serde(rename = "latestVersion")]
    pub latest_version: Option<String>,
    /// The release state of `latestVersion`, so a consumer can tell a
    /// `deprecated` headline from a current one. `null` exactly when
    /// `latestVersion` is (plan-126-A).
    #[serde(rename = "latestState")]
    pub latest_state: Option<String>,
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
            latest_state: row.latest_state,
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
            latest_state: row.latest_state,
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

/// `GET /p/:ident/docs` — the Docs tab (plan-126-F).
///
/// Resolves the package through `package_detail` first, so an unknown ident gets
/// the identical not-found page and status the Overview and Audit tabs give. It
/// then reads the documentation of the **latest active** release
/// (`Store::latest_active_version_docs`, built on plan-126-A's selection) — the
/// same release `detail.latest_version` names, so the version shown is the
/// version documented.
///
/// The stored bytes were decode-validated when they were recorded, so a decode
/// failure here means the database no longer matches what was written. That is
/// reported as an explicit error page, never a panic, and never rendered as
/// "no documentation", which would hide it.
async fn package_docs_html(
    State(state): State<AppState>,
    axum::extract::Path(ident): axum::extract::Path<String>,
) -> Response {
    let (registry_id, _fingerprint) = registry_identity(&state);
    let view = match lookup_package_docs(&state, ident).await {
        Ok(view) => view,
        Err(DocsLookupError::Package((status, Json(error)))) => {
            return crate::web::html_response(
                status,
                crate::web::message_page(&registry_id, "Package not found", &error.error),
            )
        }
        Err(DocsLookupError::Unavailable(message)) => {
            return crate::web::html_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                crate::web::message_page(&registry_id, "Documentation unavailable", message),
            )
        }
    };
    crate::web::html_response(StatusCode::OK, crate::web::docs_page(&registry_id, &view))
}

/// Why the Docs tab's lookup produced no view.
enum DocsLookupError {
    /// `package_detail` refused the ident (malformed, or no such package) —
    /// passed through unchanged so every surface reports it identically.
    Package((StatusCode, Json<ErrorResponse>)),
    /// The package exists but its stored documentation could not be read.
    Unavailable(&'static str),
}

/// The one lookup behind both `GET /p/:ident/docs` and `GET /packages/:ident/docs`,
/// so the HTML page and its JSON mirror cannot disagree about which release is
/// documented or what its documentation says.
///
/// Resolves the package through `package_detail` (same 400/404 as the other
/// package routes), then reads the **latest active** release's stored section
/// 17 (`Store::latest_active_version_docs`, plan-126-A's selection) — the release
/// `detail.latest_version` names, so the version reported is the one documented.
async fn lookup_package_docs(
    state: &AppState,
    ident: String,
) -> Result<crate::web::DocsView, DocsLookupError> {
    let Json(detail) = package_detail(State(state.clone()), axum::extract::Path(ident))
        .await
        .map_err(DocsLookupError::Package)?;

    let fallback_name = detail
        .ident
        .split_once('#')
        .map(|(_, package)| package.to_string())
        .unwrap_or_else(|| detail.ident.clone());
    let page = match state.store.latest_active_version_docs(&detail.ident) {
        Ok(Some((_version, section))) => match mfb_wire::docs::read_doc_table(&section) {
            Ok(docs) => Some(mfb_wire::docpage::from_package(docs, &fallback_name)),
            Err(_) => {
                return Err(DocsLookupError::Unavailable(
                    "The stored documentation for this release could not be decoded.",
                ))
            }
        },
        Ok(None) => None,
        Err(_) => {
            return Err(DocsLookupError::Unavailable(
                "The registry could not load this package's documentation.",
            ))
        }
    };

    Ok(crate::web::DocsView {
        ident: detail.ident,
        version: detail.latest_version,
        page,
    })
}

/// `GET /packages/:ident/docs` — the Docs tab as JSON (plan-126-F Phase 3).
///
/// Anonymous and read-only, like `GET /packages/:ident`. Serializes the same
/// `DocPage` model the HTML tab renders — not the raw wire structures — so the
/// two surfaces agree on grouping, order and anchors by construction.
/// `documentation` is `null` exactly when the HTML tab shows the
/// no-documentation statement.
async fn package_docs(
    State(state): State<AppState>,
    axum::extract::Path(ident): axum::extract::Path<String>,
) -> Result<Json<PackageDocsResponse>, (StatusCode, Json<ErrorResponse>)> {
    match lookup_package_docs(&state, ident).await {
        Ok(view) => Ok(Json(PackageDocsResponse::from_view(view))),
        Err(DocsLookupError::Package(error)) => Err(error),
        Err(DocsLookupError::Unavailable(message)) => Err(internal(message.to_string())),
    }
}

/// `GET /packages/:ident/docs` (plan-126-F Phase 3).
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PackageDocsResponse {
    pub ident: String,
    /// The latest active release — the same value `GET /packages/:ident` reports
    /// as `latestVersion`, and the release whose documentation this is. `null`
    /// when no release is active.
    pub version: Option<String>,
    /// `null` when that release carries no documentation.
    pub documentation: Option<DocPageResponse>,
}

/// A `DocPage` as JSON. Carries only the **public** groups: declarations the
/// author marked `INTERNAL` are omitted here exactly as on the HTML tab
/// (`crate::web::docs_page`).
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DocPageResponse {
    #[serde(rename = "packageName")]
    pub package_name: String,
    pub subtitle: String,
    pub intro: Vec<DocProseResponse>,
    pub deprecated: Option<String>,
    pub groups: Vec<DocGroupResponse>,
}

/// One prose block; `kind` is `desc`, `warn`, `info` or `sec`.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DocProseResponse {
    pub kind: String,
    pub text: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DocGroupResponse {
    pub title: String,
    pub declarations: Vec<DocDeclResponse>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DocDeclResponse {
    /// The HTML tab's element id for this declaration
    /// (`crate::web::doc_element_id`), so a client can link to
    /// `/p/<ident>/docs#<anchor>`.
    pub anchor: String,
    pub kind: String,
    pub name: String,
    pub signature: String,
    pub description: Vec<DocProseResponse>,
    pub parameters: Vec<DocEntryResponse>,
    /// `Fields` / `Variants` / `Members` for a type-like declaration, else `null`.
    #[serde(rename = "memberLabel")]
    pub member_label: Option<String>,
    pub members: Vec<DocEntryResponse>,
    pub returns: String,
    /// `name` is the error code.
    pub errors: Vec<DocEntryResponse>,
    pub example: String,
    pub deprecated: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DocEntryResponse {
    pub name: String,
    pub description: String,
}

impl PackageDocsResponse {
    fn from_view(view: crate::web::DocsView) -> Self {
        fn prose(blocks: Vec<mfb_wire::docpage::Prose>) -> Vec<DocProseResponse> {
            blocks
                .into_iter()
                .map(|block| DocProseResponse {
                    kind: block.kind.label().to_string(),
                    text: block.text,
                })
                .collect()
        }
        fn entries(rows: Vec<(String, String)>) -> Vec<DocEntryResponse> {
            rows.into_iter()
                .map(|(name, description)| DocEntryResponse { name, description })
                .collect()
        }
        let documentation = view.page.map(|page| DocPageResponse {
            package_name: page.package_name,
            subtitle: page.subtitle,
            intro: prose(page.intro),
            deprecated: page.package_deprecated,
            groups: page
                .public
                .into_iter()
                .map(|group| DocGroupResponse {
                    title: group.title,
                    declarations: group
                        .decls
                        .into_iter()
                        .map(|decl| DocDeclResponse {
                            anchor: crate::web::doc_element_id(&decl.anchor),
                            kind: decl.kind_label.to_string(),
                            name: decl.name,
                            signature: decl.signature,
                            description: prose(decl.desc),
                            parameters: entries(decl.args),
                            member_label: decl.member_label.map(str::to_string),
                            members: entries(decl.props),
                            returns: decl.ret,
                            errors: entries(decl.errors),
                            example: decl.example,
                            deprecated: decl.deprecated,
                        })
                        .collect(),
                })
                .collect(),
        });
        PackageDocsResponse {
            ident: view.ident,
            version: view.version,
            documentation,
        }
    }
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
        latest_state: detail.latest_state,
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
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    axum::extract::Path(ident): axum::extract::Path<String>,
) -> Response {
    let (registry_id, _fingerprint) = registry_identity(&state);
    // Shares the JSON route's budget on purpose: the HTML page does the same
    // full-log work, so exempting it would leave the bypass open.
    let audit = match package_audit(
        State(state.clone()),
        ConnectInfo(peer),
        axum::extract::Path(ident.clone()),
    )
    .await
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
    split_ident(&ident)?;
    let Some(detail) = state.store.package_detail(&ident).map_err(internal)? else {
        // One 404 for "no such package", whatever the reason. Distinguishing
        // "unknown owner" from "unknown package under a known owner" would
        // turn this route into an owner-enumeration oracle.
        return Err(not_found("unknown package".to_string()));
    };
    // The key shown is the STORED owner's (plan-168-A), the account
    // `detail.owner` names. After a transfer that is no longer the ident
    // prefix, and pairing it with the prefix owner's key described an account
    // that does not exist.
    let Some((owner_record, ident_key)) = state
        .store
        .owner_with_ident_key(&detail.owner)
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
    // plan-126-A: the headline version is the newest **active** release, not
    // simply the newest row. `versions.first()` had no state predicate, so a
    // package whose newest release was yanked advertised that release here — and
    // API consumers read this field as "the version to use". `versions` itself
    // stays complete and unfiltered above; only this selection narrows.
    let latest = state
        .store
        .latest_active_version(&ident)
        .map_err(internal)?;
    let (latest_version, latest_state) = match latest {
        Some((version, release_state)) => (Some(version), Some(release_state)),
        None => (None, None),
    };
    Ok(Json(PackageDetailResponse {
        latest_version,
        latest_state,
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
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    axum::extract::Path(ident): axum::extract::Path<String>,
) -> Result<Json<PackageAuditResponse>, (StatusCode, Json<ErrorResponse>)> {
    if !state
        .rate_limiter
        .allow(&format!("log:{}", peer.ip()), LOG_PER_IP_MAX, 60)
    {
        return Err(too_many_requests());
    }
    let (owner_part, _package_part) = split_ident(&ident)?;
    let Some(audit) = state.store.package_audit(&ident).map_err(internal)? else {
        return Err(not_found("unknown package".to_string()));
    };

    // Deliberately NOT `signed_checkpoint()`: this route needs the leaf vector
    // itself to build an inclusion path per publish, so the memo would save
    // nothing and would introduce a race — a head memoised at one size beside
    // paths built from a differently-sized leaf set. Computing both from ONE
    // read keeps the response internally coherent, which is the property the
    // bug's non-goals protect. It is the per-IP budget above that bounds this
    // route, not the cache.
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
        let epoch = u32::try_from(row.transfer_epoch)
            .map_err(|_| internal(format!("release {} has a negative epoch", row.version)))?;
        versions.push(IndexVersion {
            version: row.version,
            hash: row.hash,
            published_at: row.published_at,
            state: row.state,
            abi_index,
            log_entry,
            publisher: row.publisher_display.map(|owner| Publisher {
                owner,
                ident_fingerprint: row.publisher_ident_fingerprint,
            }),
            epoch,
        });
    }
    let (server_public, server_private) = state.store.server_keypair().map_err(internal)?;
    let name_binding = crypto::sign(
        &server_private,
        &crypto::name_binding_message(&owner_record.owner_display, &ident_key.fingerprint),
    )
    .map_err(internal)?;
    let mut transfers = Vec::new();
    for record in state.store.transfer_history(&ident).map_err(internal)? {
        let Some((to_record, to_key)) = state
            .store
            .owner_with_ident_key(&record.to_owner)
            .map_err(internal)?
        else {
            return Err(internal(format!(
                "transfer recipient {} has no current ident key",
                record.to_owner
            )));
        };
        let to_binding = crypto::sign(
            &server_private,
            &crypto::name_binding_message(&to_record.owner_display, &to_key.fingerprint),
        )
        .map_err(internal)?;
        transfers.push(TransferLink {
            sequence: record.sequence,
            from_owner: record.from_owner,
            from_ident_key: record.from_ident_key.as_deref().map(metadata_key),
            to_owner: record.to_owner,
            to_ident_key: record.to_ident_key.as_deref().map(metadata_key),
            release_digest: record.release_digest,
            offer_signature: record.offer_signature.as_deref().map(crypto::encode_bytes),
            accept_signature: record.accept_signature.as_deref().map(crypto::encode_bytes),
            offered_at: record.offered_at,
            accepted_at: record.accepted_at,
            to_binding: OwnerBinding {
                owner: to_record.owner_display,
                ident_key: metadata_key(&to_key.public_key),
                ident_fingerprint: to_key.fingerprint,
                name_binding_signature: crypto::encode_bytes(&to_binding),
            },
        });
    }
    Ok(Json(IndexResponse {
        ident: ident.clone(),
        owner: owner_record.owner_display,
        ident_key: metadata_key(&ident_key.public_key),
        ident_fingerprint: ident_key.fingerprint,
        name_binding_signature: crypto::encode_bytes(&name_binding),
        server_fingerprint: crypto::fingerprint(&server_public),
        versions,
        transfers,
    }))
}

/// A public key in project-metadata form, `ed25519:<base64url>`.
fn metadata_key(public_key: &[u8]) -> String {
    format!("ed25519:{}", crypto::encode_bytes(public_key))
}

/// Snapshot metadata lifetime (plan-10-C2): the snapshot pins the index state
/// for a week; the timestamp refreshes daily and pins the current snapshot.
const SNAPSHOT_TTL_SECS: i64 = 7 * 24 * 3600;
const TIMESTAMP_TTL_SECS: i64 = 24 * 3600;

/// The publish-token grant behind a session's auth key, if that key is a token
/// (bug-492 / audit-3 REPO-01).
///
/// A publish token (plan-10-D1) is a *delegated* credential: an ordinary
/// `keys` row plus a `publish_tokens` row naming its scope and expiry. Only
/// `/signing` used to read that row, so every other authenticated route treated
/// a token session as the account itself — a token could enrol a machine via
/// `/machines/link` and mint a permanent unscoped key that outlived its own
/// revocation. The scope and expiry are a property of the *session* now: this
/// is the one reader of the row, every route that accepts a session calls it,
/// and the expiry/revocation refusals live here so no route can forget them.
///
/// Returns `Ok(None)` for a machine key, `Ok(Some(scope))` for a live token,
/// and `Err` for a revoked or expired one. The session JWT is minted for an
/// hour and may well outlive a short token, which is why this re-reads the row
/// per request instead of trusting the JWT.
fn publish_token_scope(
    store: &Store,
    key_id: i64,
) -> Result<Option<String>, (StatusCode, Json<ErrorResponse>)> {
    let Some((scope, expires_at, revoked_at)) =
        store.publish_token_for_key(key_id).map_err(internal)?
    else {
        return Ok(None);
    };
    if revoked_at.is_some() {
        return Err(bad_request("publish token is revoked".to_string()));
    }
    if expires_at <= now_unix() {
        return Err(bad_request("publish token has expired".to_string()));
    }
    Ok(Some(scope))
}

/// Authorize a per-package operation on the package's **current** owner
/// (bug-494 / audit-3 REPO-03).
///
/// `/release-state` and `/signing` used to compare the ident's `owner#` prefix
/// to the session owner. That prefix never moves when ownership does —
/// `accept_transfer` rewrites `packages.owner_id`, and already-published
/// artifacts keep verifying against the ident string they were signed with —
/// so the former owner kept yank and attestation rights over a package she
/// handed over while the real owner was refused both. The `packages` row is
/// authoritative whenever it exists; the prefix decides only for a package that
/// has no row yet, which is the first-publish case `/signing` must keep
/// serving (and which keeps the error shape for a foreign ident unchanged).
fn require_package_owner(
    store: &Store,
    ident: &str,
    ident_owner: &str,
    session_owner: &str,
    session_owner_id: i64,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    match store.package_owner(ident).map_err(internal)? {
        Some(package_owner) => {
            if package_owner.id != session_owner_id {
                return Err(bad_request(
                    "package is not owned by the session owner".to_string(),
                ));
            }
        }
        None => {
            if crate::validation::fold_owner(ident_owner)
                != crate::validation::fold_owner(session_owner)
            {
                return Err(bad_request(
                    "ident owner does not match session owner".to_string(),
                ));
            }
        }
    }
    Ok(())
}

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
    let Some((owner_record, key)) = state
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
    // A token session still needs the ident signature these routes demand, but
    // an expired or revoked token must not get even that far (bug-492).
    publish_token_scope(&state.store, key.id)?;
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

    // bug-420 item 1: only an owner (or the org itself) may create or remove an
    // `owner`. An `admin` grantor passes the check above, so without this an
    // admin could grant itself the `owner` role or remove the org's existing
    // owner and seize the namespace. Admins keep authority over admin/publisher.
    let grantor_is_owner = is_org || grantor_role.as_deref() == Some("owner");
    if !grantor_is_owner {
        let touches_owner = if request.action == "remove" {
            state
                .store
                .org_member_role(&request.org, &request.member)
                .map_err(internal)?
                .as_deref()
                == Some("owner")
        } else {
            request.role == "owner"
        };
        if touches_owner {
            return Err(bad_request(
                "only an owner or the org itself may grant or remove the owner role".to_string(),
            ));
        }
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
    // Scope must belong to the issuing owner. An exact scope names one package,
    // which belongs to its STORED owner once it exists (plan-168-B): the
    // recipient of a transfer can scope a CI token to it, and the former owner
    // no longer can.
    let scope_is_issuers = match request.scope.split_once('#') {
        Some((_, package)) if package != "*" => {
            match state
                .store
                .package_owner(&request.scope)
                .map_err(internal)?
            {
                Some(current) => {
                    crate::validation::fold_owner(&current.owner_display)
                        == crate::validation::fold_owner(&request.owner)
                }
                None => scope_owner_matches(&request.scope, &request.owner),
            }
        }
        _ => scope_owner_matches(&request.scope, &request.owner),
    };
    if !scope_is_issuers {
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
/// package to a recipient, authorized by the current owner's ident signature
/// over the v2 offer terms (plan-168-A). The store verifies the signature and
/// the terms inside its writing transaction.
async fn transfer_offer(
    State(state): State<AppState>,
    Json(request): Json<TransferOfferRequest>,
) -> Result<Json<TransferResponse>, (StatusCode, Json<ErrorResponse>)> {
    let (_owner, _ident_public) =
        session_and_ident(&state, &request.from_owner, &request.session_token)?;
    // The offer is signed by the STORED owner under their display name, which
    // after a transfer is no longer the ident prefix. Name it on a mismatch so
    // the caller can see whose ident must sign.
    if let Some(current) = state
        .store
        .package_owner(&request.ident)
        .map_err(internal)?
    {
        if current.owner_display != request.from_owner {
            return Err(bad_request(format!(
                "offering owner does not currently own the package; it is owned by {}",
                current.owner_display
            )));
        }
    }
    let signature =
        crypto::decode_bytes(&request.ident_signature, "identSignature").map_err(bad_request)?;
    state
        .store
        .create_transfer_offer(
            &request.ident,
            &request.from_owner,
            &request.to_owner,
            request.sequence,
            &request.release_digest,
            &signature,
        )
        .map_err(bad_request)?;
    Ok(Json(TransferResponse {
        ident: request.ident,
        to_owner: request.to_owner,
        accepted: false,
    }))
}

/// `POST /packages/transfer/accept` (plan-10-D1): the recipient accepts a
/// pending offer by signing the v2 accept message over the same terms
/// (plan-168-A); the package is re-bound to them, the signed pair is appended
/// to the transfer history, and the acceptance is logged.
async fn transfer_accept(
    State(state): State<AppState>,
    Json(request): Json<TransferAcceptRequest>,
) -> Result<Json<TransferResponse>, (StatusCode, Json<ErrorResponse>)> {
    let (_owner, _ident_public) =
        session_and_ident(&state, &request.to_owner, &request.session_token)?;
    let signature =
        crypto::decode_bytes(&request.ident_signature, "identSignature").map_err(bad_request)?;
    state
        .store
        .accept_transfer(&request.ident, &request.to_owner, &signature)
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

/// Whether a token `scope` permits `ident` (`<owner>#<package>`), whose
/// governing owner is `governing_owner` (see [`governing_owner`]). `<owner>#*`
/// matches any package that owner governs — after a transfer that follows the
/// stored owner, not the ident prefix (plan-168-B, B-D2); otherwise the scope
/// must name the ident exactly.
fn scope_permits(scope: &str, ident: &str, governing_owner: &str) -> bool {
    let (Some((scope_owner, scope_pkg)), true) = (scope.split_once('#'), ident.contains('#'))
    else {
        return false;
    };
    if scope_pkg == "*" {
        return crate::validation::fold_owner(scope_owner)
            == crate::validation::fold_owner(governing_owner);
    }
    scope == ident
}

/// The account that governs `ident` for token scopes: its stored owner, or for
/// a package with no row yet (first publish) the ident's `owner#` prefix.
fn governing_owner(
    store: &Store,
    ident: &str,
) -> Result<String, (StatusCode, Json<ErrorResponse>)> {
    Ok(match store.package_owner(ident).map_err(internal)? {
        Some(current) => current.owner_display,
        None => ident
            .split_once('#')
            .map(|(owner, _)| owner.to_string())
            .unwrap_or_default(),
    })
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
    let Some((owner, key)) = state
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
    // An expired or revoked publish token is refused before the ident
    // signature is even looked at (bug-492).
    publish_token_scope(&state.store, key.id)?;
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
    if package_part.is_empty() {
        return Err(bad_request(
            "ident owner does not match session owner".to_string(),
        ));
    }
    // bug-494: the package's current owner decides, not the ident's name
    // prefix — a former owner must not keep yanking what she transferred.
    require_package_owner(
        &state.store,
        &request.ident,
        ident_owner,
        &request.owner,
        owner.id,
    )?;
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
        .set_release_state(owner.id, &request.ident, &request.version, &request.state)
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
    let Some((owner, key)) = state
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
    // bug-492: enrolling a machine is account authority. A publish token is a
    // delegated credential (one scope, a TTL, revocable); let it start a link
    // and it parks a blob it can fetch itself with a fresh keypair, minting a
    // permanent unscoped auth key that survives the token's revocation. The
    // fetch half proves only possession of the *new* key, so the refusal has
    // to be here.
    if publish_token_scope(&state.store, key.id)?.is_some() {
        return Err(bad_request(
            "a publish token session cannot link a machine".to_string(),
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
    // bug-583: the approval verifier must be a well-formed Ed25519 public key,
    // or the fetch half would have nothing real to check the code against.
    let approval_key =
        crypto::decode_bytes(&request.approval_key, "approvalKey").map_err(bad_request)?;
    if approval_key.len() != crypto::PUBLIC_KEY_LEN {
        return Err(bad_request("malformed pairing approval key".to_string()));
    }
    let expires_at = state
        .store
        .store_pairing_blob(owner.id, &request.lookup, &blob, &salt, &approval_key)
        .map_err(conflict_or_bad_request)?;
    Ok(Json(LinkStartResponse {
        owner: owner.owner_display,
        expires_at,
    }))
}

/// New-machine side of a link: the pairing approval is a signature made with
/// the key derived from the typed pairing CODE (bug-583) over this exact auth
/// public key. The lookup only *finds* the pending pairing — it is visible to
/// the relay and to anyone who can read the database, so it authorizes
/// nothing. On a valid approval the new machine's auth key is registered to
/// the account and the (single-use) blob handed over in the same exchange.
async fn link_fetch(
    State(state): State<AppState>,
    Json(request): Json<LinkFetchRequest>,
) -> Result<Json<LinkFetchResponse>, (StatusCode, Json<ErrorResponse>)> {
    let auth_key = crypto::decode_bytes(&request.auth_key, "authKey").map_err(bad_request)?;
    let proof = crypto::decode_bytes(&request.proof, "proof").map_err(bad_request)?;
    let approval = crypto::decode_bytes(&request.approval, "approval").map_err(bad_request)?;
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
    // The approval is checked inside the store transaction, so a caller that
    // cannot prove the code neither receives the blob nor spends the honest
    // machine's single-use pairing.
    let approval_message = crypto::pairing_approval_message(&request.lookup, &auth_key);
    let fetched = state
        .store
        .take_pairing_blob(&request.owner, &request.lookup, |approval_key| {
            crypto::verify(approval_key, &approval_message, &approval).is_ok()
        })
        .map_err(internal)?;
    let (blob, salt) = match fetched {
        PairingFetch::Relayed { blob, salt } => (blob, salt),
        PairingFetch::Unapproved => {
            return Err(bad_request(
                "pairing approval does not prove the pairing code".to_string(),
            ))
        }
        PairingFetch::Missing => {
            return Err(bad_request(
                "unknown, used, or expired pairing code".to_string(),
            ))
        }
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
        .map_err(|err| {
            // Refusing to orphan the account is the caller's answer (bug-493);
            // anything else is a store failure.
            if err.contains("last auth key") {
                bad_request(err)
            } else {
                internal(err)
            }
        })?;
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
    // bug-492 / audit-3 REPO-09: an expired or revoked publish token must not
    // open a session at all — the JWT minted below would outlive it by an hour.
    publish_token_scope(&state.store, key.id)?;
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
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Json(request): Json<SigningRequest>,
) -> Result<Json<SigningResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Per-client bucket keyed by peer IP (bug-419): the key was the
    // unauthenticated `request.owner` (checked before `verify_session_token`
    // below), letting an anonymous attacker exhaust a targeted victim's signing
    // budget and block that victim from obtaining publish attestations. The
    // global ceiling is a secondary backstop, as on `/register` and `/login`.
    if !state
        .rate_limiter
        .allow(&format!("signing:{}", peer.ip()), SIGNING_PER_IP_MAX, 60)
        || !state.rate_limiter.allow("signing", AUTH_GLOBAL_CEILING, 60)
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
    // only attest packages within its scope, and only until it expires (the
    // expiry/revocation refusals live in `publish_token_scope`, bug-492).
    if let Some(scope) = publish_token_scope(&state.store, key.id)? {
        if !scope_permits(
            &scope,
            &request.ident,
            &governing_owner(&state.store, &request.ident)?,
        ) {
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
    // bug-494: the package's current owner decides, not the ident's name
    // prefix; the prefix is consulted only for a first publish (no row yet).
    require_package_owner(
        &state.store,
        &request.ident,
        ident_owner,
        &request.owner,
        owner.id,
    )?;
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
    // plan-126-E: capture MFPC section 17 (the `DOC` table) from the payload this
    // handler already holds, best-effort. The RAW section bytes are what gets
    // stored -- `mfb_wire::docs` stays the only decoder -- but only once they
    // decode, so a malformed doc table is never recorded. It is never a reason to
    // refuse the publish either: documentation "does not affect execution or the
    // ABI" (src/binary_repr/writer.rs), and turning a doc-table defect into a
    // rejected signed package would be a new failure mode on a trust path. Same
    // posture as `parse_package_description` directly above.
    let docs = parsed.as_ref().and_then(|package| {
        let sections = mfb_wire::mfpc::read_section_table(&package.payload).ok()?;
        let section = sections.get(&mfb_wire::mfpc::SECTION_DOC_TABLE)?;
        mfb_wire::docs::read_doc_table(section).ok()?;
        Some(section.to_vec())
    });
    // plan-168-A §4.3: the publisher's ident key is the one the header names,
    // which validation has already required to equal `identFingerprint` and the
    // publisher's current ident.
    let publisher_ident_fingerprint = Some(request.ident_fingerprint.clone());
    let publish_metadata = match manifest_metadata {
        Some(meta) => crate::store::PublishMetadata {
            author: Some(meta.author).filter(|value| !value.is_empty()),
            url: Some(meta.url).filter(|value| !value.is_empty()),
            description,
            docs,
            publisher_ident_fingerprint,
        },
        // `docs` is set explicitly here too. Left to `..Default::default()` it
        // would silently become `None` for every package without a MANIFEST
        // section, dropping documentation the publisher did sign.
        None => crate::store::PublishMetadata {
            description,
            docs,
            publisher_ident_fingerprint,
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
    // bug-492: a publish token's scope and expiry bound `/validate` and
    // `/publish` exactly as they bound `/signing`. A hard refusal rather than a
    // diagnostic — the artifact may be perfectly valid; the credential is not —
    // and checked before any of the expensive work below. `request.ident` is
    // enough here: a package whose own ident differs fails the equality
    // diagnostic further down and can never publish.
    if let Some((_owner, key)) = state
        .store
        .owner_auth_key_by_fingerprint(&claims.sub, &claims.auth_fingerprint)
        .map_err(internal)?
    {
        if let Some(scope) = publish_token_scope(&state.store, key.id)? {
            if !scope_permits(
                &scope,
                &request.ident,
                &governing_owner(&state.store, &request.ident)?,
            ) {
                return Err(bad_request(
                    "publish token scope does not permit this package".to_string(),
                ));
            }
        }
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

    // §3.4 step 1 — the session owner publishes, and must be entitled to the
    // ident. bug-792 / plan-168-B: when the package exists its STORED owner
    // decides (the same rule `/signing` and `/release-state` apply through
    // `require_package_owner`); the `owner#` prefix decides only a first
    // publish. The prefix never moves when ownership does, so binding the
    // publisher to it stranded every transferred package.
    let Some((owner_part, _package_part)) = package.ident.split_once('#') else {
        diagnostics.push("package ident must use <owner>#<package>".to_string());
        return Ok(invalid_report(hash, diagnostics));
    };
    match state
        .store
        .package_owner(&package.ident)
        .map_err(internal)?
    {
        Some(current) => {
            if current.id != claims.owner_id {
                diagnostics.push("session owner is not the package's current owner".to_string());
            } else if crate::validation::fold_owner(owner_part)
                != crate::validation::fold_owner(&claims.sub)
                && state
                    .store
                    .has_legacy_transfer(&package.ident)
                    .map_err(internal)?
            {
                // Clients stop trusting owners at the first unsigned (legacy)
                // transfer link and fall back to the origin owner alone, so a
                // later owner's release would publish and then never install.
                diagnostics.push(LEGACY_TRANSFER_PUBLISH_REFUSED.to_string());
            }
        }
        None => {
            if crate::validation::fold_owner(owner_part)
                != crate::validation::fold_owner(&claims.sub)
            {
                diagnostics.push("session owner does not match package ident owner".to_string());
            }
            if state
                .store
                .owner_with_ident_key(owner_part)
                .map_err(internal)?
                .is_none()
            {
                diagnostics.push("package ident owner is not registered".to_string());
                return Ok(invalid_report(hash, diagnostics));
            }
        }
    }
    let Some((owner, _key)) = state
        .store
        .owner_auth_key_by_fingerprint(&claims.sub, &claims.auth_fingerprint)
        .map_err(internal)?
    else {
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
    // name↔ident binding of the PUBLISHER (the session owner, plan-168-B); a
    // stale attestation (pre-rotation) is refused.
    match state
        .store
        .owner_with_ident_key(&owner.owner_display)
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
mod tests;
