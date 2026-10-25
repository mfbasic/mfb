use super::*;

mod accounts;
mod browse_faults;
mod identity;
mod log_routes;
mod pages;
mod publish;
mod search_limits;
mod tokens;
mod transfers;
mod validate;
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
        peer("127.0.0.1"),
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

/// The v2 terms of the next transfer of `ident`, read from the real
/// `package_index` handler the way a client reads them (plan-168-A §4.5):
/// `(sequence, current owner, their bound fingerprint, release digest)`.
async fn index_transfer_terms(state: &AppState, ident: &str) -> (i64, String, String, String) {
    let index = package_index(State(state.clone()), axum::extract::Path(ident.to_string()))
        .await
        .expect("index served")
        .0;
    let (owner, fingerprint) = match index.transfers.last() {
        Some(link) => (
            link.to_binding.owner.clone(),
            link.to_binding.ident_fingerprint.clone(),
        ),
        None => (index.owner.clone(), index.ident_fingerprint.clone()),
    };
    let digest = crypto::release_digest(
        index
            .versions
            .iter()
            .map(|version| (version.version.as_str(), version.hash.as_str())),
    );
    (index.transfers.len() as i64, owner, fingerprint, digest)
}

/// A `/packages/transfer/offer` request of `ident` from `from_owner` to
/// `to_owner`, signed by `signer`'s ident over the current v2 terms.
async fn signed_offer(
    state: &AppState,
    ident: &str,
    from_owner: &str,
    signer: &TestOwnerKeys,
    from_token: &str,
    to_owner: &str,
) -> TransferOfferRequest {
    let (sequence, owner, fingerprint, digest) = index_transfer_terms(state, ident).await;
    let message = crypto::transfer_offer_message_v2(crypto::TransferTermsRef {
        ident,
        sequence,
        from_owner: &owner,
        from_ident_fingerprint: &fingerprint,
        to_owner,
        release_digest: &digest,
    });
    TransferOfferRequest {
        ident: ident.to_string(),
        from_owner: from_owner.to_string(),
        to_owner: to_owner.to_string(),
        sequence,
        release_digest: digest,
        session_token: from_token.to_string(),
        ident_signature: crypto::encode_bytes(
            &crypto::sign(&signer.ident_private, &message).unwrap(),
        ),
    }
}

/// A `/packages/transfer/accept` request of `ident` by `to_owner`, signed by
/// `signer`'s ident over the current v2 terms.
async fn signed_accept(
    state: &AppState,
    ident: &str,
    to_owner: &str,
    signer: &TestOwnerKeys,
    to_token: &str,
) -> TransferAcceptRequest {
    let (sequence, owner, fingerprint, digest) = index_transfer_terms(state, ident).await;
    let message = crypto::transfer_accept_message_v2(crypto::TransferTermsRef {
        ident,
        sequence,
        from_owner: &owner,
        from_ident_fingerprint: &fingerprint,
        to_owner,
        release_digest: &digest,
    });
    TransferAcceptRequest {
        ident: ident.to_string(),
        to_owner: to_owner.to_string(),
        session_token: to_token.to_string(),
        ident_signature: crypto::encode_bytes(
            &crypto::sign(&signer.ident_private, &message).unwrap(),
        ),
    }
}

/// The whole two-sided transfer through the real `transfer_offer` /
/// `transfer_accept` handlers, with terms read from `package_index`
/// (plan-168-A Phase 2; plan-168-B uses it too).
#[allow(clippy::too_many_arguments)]
async fn transfer_through_handlers(
    state: &AppState,
    ident: &str,
    from_owner: &str,
    from: &TestOwnerKeys,
    from_token: &str,
    to_owner: &str,
    to: &TestOwnerKeys,
    to_token: &str,
) {
    let offer = signed_offer(state, ident, from_owner, from, from_token, to_owner).await;
    let _ = transfer_offer(State(state.clone()), Json(offer))
        .await
        .expect("offer");
    let accept = signed_accept(state, ident, to_owner, to, to_token).await;
    let _ = transfer_accept(State(state.clone()), Json(accept))
        .await
        .expect("accept");
}

/// Open a session bound to a specific auth key (by fingerprint), so a
/// publish-token session can be exercised.
fn open_session_for_key(store: &Store, owner: &str, private: &[u8], fingerprint: &str) -> String {
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
        Store::open_repository(&temp.path().join("meta.db"), &temp.path().join("data")).unwrap();
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

// === plan-126-F: the Docs tab through the real router ==================

/// Publish `alice#toolbox@1.0.0` with optional documentation.
fn seed_toolbox(h: &Harness, docs: Option<Vec<u8>>) {
    register_owner_with_all_keys(&h.store, "alice");
    let alice_id = h.store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
    h.store
        .publish_package_version(
            alice_id,
            "alice#toolbox",
            "1.0.0",
            "hash-1.0.0",
            "data/1.0.0.mfp",
            "{}",
            &[],
            &crate::store::PublishMetadata {
                docs,
                ..Default::default()
            },
        )
        .unwrap();
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

fn put_u16(dst: &mut Vec<u8>, value: u16) {
    dst.extend_from_slice(&value.to_le_bytes());
}

fn put_u32(dst: &mut Vec<u8>, value: u32) {
    dst.extend_from_slice(&value.to_le_bytes());
}

fn put_u64(dst: &mut Vec<u8>, value: u64) {
    dst.extend_from_slice(&value.to_le_bytes());
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

/// A `/validate` request for `version` of `ident`, built the way the CLI
/// builds one for `owner`: a registry attestation for `owner`, a proof and
/// header naming `owner` as owner and author, signed under `owner`'s ident.
async fn owner_signed_request(
    state: &AppState,
    owner: &str,
    keys: &TestOwnerKeys,
    token: &str,
    ident: &str,
    version: &str,
) -> PackageArtifactRequest {
    owner_signed_request_attested_for(state, owner, keys, token, ident, ident, version).await
}

/// `owner_signed_request`, but with the registry attestation requested
/// for `attest_ident`, so a test can build a package for an ident the
/// registry would never attest for this owner.
async fn owner_signed_request_attested_for(
    state: &AppState,
    owner: &str,
    keys: &TestOwnerKeys,
    token: &str,
    ident: &str,
    attest_ident: &str,
    version: &str,
) -> PackageArtifactRequest {
    let (signing_public, signing_private) = crypto::generate_keypair();
    let ident_fingerprint = crypto::fingerprint(&keys.ident_public);
    let signing_fingerprint = crypto::fingerprint(&signing_public);
    let attestation = signing(
        State(state.clone()),
        peer("127.0.0.1"),
        Json(SigningRequest {
            owner: owner.to_string(),
            ident: attest_ident.to_string(),
            version: version.to_string(),
            signing_fingerprint: signing_fingerprint.clone(),
            session_token: token.to_string(),
        }),
    )
    .await
    .expect("attestation issued")
    .0;
    let proof = serde_json::json!({"owner": owner, "ident": ident, "version": version,
        "identFingerprint": ident_fingerprint, "signingFingerprint": signing_fingerprint})
    .to_string();
    let proof_sig = crypto::sign(
        &keys.ident_private,
        &crypto::proof_signing_input(proof.as_bytes()),
    )
    .unwrap();
    let package_name = ident.split_once('#').unwrap().1;
    let artifact = package::test_support::serialize(
        &package::test_support::TestPackage {
            name: package_name.to_string(),
            ident: ident.to_string(),
            version: version.to_string(),
            author: owner.to_string(),
            url: String::new(),
            payload: b"MFPCtestpayload".to_vec(),
            ident_key: format!("ed25519:{}", crypto::encode_bytes(&keys.ident_public)),
            signing_key: format!("ed25519:{}", crypto::encode_bytes(&signing_public)),
            proof,
            proof_sig,
            attestation: attestation.attestation,
            attestation_sig: crypto::decode_bytes(&attestation.attestation_signature, "sig")
                .unwrap(),
        },
        &signing_private,
    );
    let parsed = package::parse_mfp_package(&artifact).unwrap();
    PackageArtifactRequest {
        ident: parsed.ident.clone(),
        version: parsed.version.clone(),
        artifact: crypto::encode_bytes(&artifact),
        content_hash: parsed.content_hash_hex(),
        ident_fingerprint: parsed.ident_fingerprint().unwrap(),
        signing_fingerprint: parsed.signing_fingerprint().unwrap(),
        session_token: token.to_string(),
    }
}
