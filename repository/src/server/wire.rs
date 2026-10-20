//! plan-176-L: the registry server's wire code. `AppState`, `serve`, the router and the
//! error responses live in `repository/src/server/mod.rs`.

use super::*;

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
    pub(super) fn from_view(view: crate::web::DocsView) -> Self {
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
