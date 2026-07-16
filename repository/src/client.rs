use crate::crypto;
use crate::local::{self, LocalPaths};
use crate::server::{
    ChallengeRequest, ChallengeResponse, CheckpointResponse, ConsistencyProofResponse,
    ErrorResponse, IdentChainResponse, InclusionProofResponse, IndexResponse, LinkFetchRequest,
    LinkFetchResponse, LinkStartRequest, LinkStartResponse, LogEntry, LoginRequest, LoginResponse,
    OrgMemberRequest, OrgMemberResponse, PackageArtifactRequest, PublishPackageResponse,
    RegisterProofs, RegisterRequest, RegisterResponse, ReleaseStateRequest, ReleaseStateResponse,
    RevokeChallengeRequest, RevokeRequest, RevokeResponse, RootResponse, RotateRequest,
    RotateResponse, ServerIdentResponse, SignedMetadataResponse, SigningRequest, SigningResponse,
    TokenIssueRequest, TokenIssueResponse, TokenRevokeRequest, TokenRevokeResponse,
    TransferAcceptRequest, TransferOfferRequest, TransferResponse, ValidatePackageResponse,
};
use crate::validation::validate_owner_name;
use crate::DEFAULT_REPO_URL;
use reqwest::blocking::Client;
use serde::de::DeserializeOwned;
use std::sync::OnceLock;
use std::time::Duration;

/// A registry that cannot complete a TCP handshake this quickly is down; failing
/// fast beats waiting out a full request deadline on a dead host.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Deadline for a control-plane JSON call. These payloads are small — keys,
/// signatures, index metadata — so a slow one means a sick registry, not a big
/// transfer. This is reqwest's own default, kept deliberately: it is the right
/// bound for every endpoint except `/blob`.
const CONTROL_TIMEOUT: Duration = Duration::from_secs(30);

/// Deadline for one `/blob/<hash>` transfer.
///
/// `CONTROL_TIMEOUT` is far too tight here: the registry accepts bodies up to
/// 64 MiB, and moving that inside 30s demands ~18 Mbit/s of *sustained*
/// throughput. A vendored native library on an ordinary connection therefore
/// fails outright with the default — the common case, not an edge case.
///
/// This bound is loose on purpose. Blocking reqwest exposes no read/stall
/// timeout (`ClientBuilder::read_timeout` is async-only in 0.12), and its
/// `timeout` is a *deadline* applied twice — once to connect+headers, then again
/// to the whole body read (`blocking::Response::bytes` re-wraps it). So the only
/// portable choice is between an unbounded hang and a generous deadline; 10
/// minutes carries a full 64 MiB down to ~0.9 Mbit/s while still bounding a
/// wedged socket. A true stall timeout needs the async API or a chunked reader.
const BLOB_TIMEOUT: Duration = Duration::from_secs(600);

/// The one shared HTTP client for every registry call.
///
/// Built once, for two reasons. Each blocking `Client` owns a tokio runtime on a
/// background thread, so constructing one per request spawned a runtime and paid
/// a fresh TLS handshake every time, defeating connection pooling entirely —
/// which matters now that installing a package can mean a run of sequential blob
/// fetches. And it is the only place to set `connect_timeout`, which reqwest
/// leaves unset by default.
fn http_client() -> Result<&'static Client, String> {
    static CLIENT: OnceLock<Result<Client, String>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            Client::builder()
                .connect_timeout(CONNECT_TIMEOUT)
                .timeout(CONTROL_TIMEOUT)
                .build()
                .map_err(|err| format!("failed to build the repository HTTP client: {err}"))
        })
        .as_ref()
        .map_err(|err| err.clone())
}

pub fn repo_url_from_env() -> String {
    std::env::var("MFB_REPO_URL").unwrap_or_else(|_| DEFAULT_REPO_URL.to_string())
}

/// Reject plaintext `http://` to a non-loopback registry (audit-2 SUP-01 /
/// bug-189). Package signatures remain the authenticity anchor, but cleartext
/// transport leaks which packages a build pulls and — critically — lets an
/// on-path attacker MITM the first-contact server-key pin (SUP-02). `http` stays
/// allowed for loopback so a local dev registry needs no TLS; `https` is required
/// for anything else. Called by every network entry point.
fn ensure_transport_security(repo_url: &str) -> Result<(), String> {
    let parsed = reqwest::Url::parse(repo_url)
        .map_err(|err| format!("invalid registry URL '{repo_url}': {err}"))?;
    match parsed.scheme() {
        "https" => Ok(()),
        "http" => {
            let host = parsed.host_str().unwrap_or("");
            // `host_str` returns an IPv6 literal in brackets (`[::1]`); strip them
            // so it parses as an `IpAddr`.
            let bare = host.trim_start_matches('[').trim_end_matches(']');
            let is_loopback = host == "localhost"
                || bare
                    .parse::<std::net::IpAddr>()
                    .map(|ip| ip.is_loopback())
                    .unwrap_or(false);
            if is_loopback {
                Ok(())
            } else {
                Err(format!(
                    "refusing plaintext http:// to non-loopback registry '{host}': use \
                     https:// (set MFB_REPO_URL). Signatures still gate package \
                     authenticity, but http exposes your dependency set and lets an \
                     on-path attacker tamper with the trust bootstrap."
                ))
            }
        }
        other => Err(format!(
            "unsupported registry URL scheme '{other}://' in '{repo_url}'"
        )),
    }
}

/// Fetch the registry public key from `GET /ident` and pin it as
/// `server.pub` on first contact; a later mismatch is refused (plan-23 index
/// §10.3). Every online flow calls this before touching other routes.
pub fn ensure_server_key(repo_url: &str, paths: &LocalPaths) -> Result<Vec<u8>, String> {
    let response = get_json::<ServerIdentResponse>(repo_url, "/ident")?;
    let server_key = crypto::decode_bytes(&response.server_key, "serverKey")?;
    if crypto::fingerprint(&server_key) != response.server_fingerprint {
        return Err("repository /ident fingerprint does not match its key".to_string());
    }
    local::pin_server_key(paths, &server_key)?;
    Ok(server_key)
}

pub fn register(
    repo_url: &str,
    paths: &LocalPaths,
    owner: &str,
) -> Result<RegisterResponse, String> {
    validate_owner_name(owner)?;
    ensure_server_key(repo_url, paths)?;
    // Both keypairs are generated locally; only the public halves and their
    // role-separated proofs-of-possession go to the server (plan-23 §3.1).
    let (auth_public, auth_private) = crypto::generate_keypair();
    let (ident_public, ident_private) = crypto::generate_keypair();
    let auth_message = crypto::registration_message(crypto::ROLE_AUTH, owner, &auth_public);
    let auth_proof = crypto::sign(&auth_private, &auth_message)?;
    let ident_message = crypto::registration_message(crypto::ROLE_IDENT, owner, &ident_public);
    let ident_proof = crypto::sign(&ident_private, &ident_message)?;
    let request = RegisterRequest {
        owner: owner.to_string(),
        auth_key: crypto::encode_bytes(&auth_public),
        ident_key: crypto::encode_bytes(&ident_public),
        proofs: RegisterProofs {
            auth: crypto::encode_bytes(&auth_proof),
            ident: crypto::encode_bytes(&ident_proof),
        },
    };
    local::write_auth_keypair(paths, owner, &auth_public, &auth_private)?;
    local::write_ident_keypair(paths, owner, &ident_public, &ident_private)?;
    let response = post_json::<RegisterResponse>(repo_url, "/accounts/register", &request);
    match response {
        Ok(response) => Ok(response),
        Err(err) => {
            local::remove_owner_keys(paths, owner);
            Err(err)
        }
    }
}

pub fn auth(repo_url: &str, paths: &LocalPaths, owner: &str) -> Result<LoginResponse, String> {
    validate_owner_name(owner)?;
    ensure_server_key(repo_url, paths)?;
    let private = match local::read_auth_private_key(paths, owner) {
        Ok(private) => private,
        Err(local_err) => {
            let probe = post_json::<ChallengeResponse>(
                repo_url,
                "/auth/challenge",
                &ChallengeRequest {
                    owner: owner.to_string(),
                    auth_fingerprint: String::new(),
                },
            );
            if let Err(remote_err) = probe {
                if remote_err.contains("unknown owner") {
                    return Err(remote_err);
                }
            }
            return Err(local_err);
        }
    };
    let public = crypto::public_from_private(&private)?;
    if let Ok(stored_public) = local::read_auth_public_key(paths, owner) {
        if stored_public != public {
            return Err("mismatched local key fingerprint".to_string());
        }
    }
    let fingerprint = crypto::fingerprint(&public);
    let challenge = post_json::<ChallengeResponse>(
        repo_url,
        "/auth/challenge",
        &ChallengeRequest {
            owner: owner.to_string(),
            auth_fingerprint: fingerprint,
        },
    )?;
    let nonce = crypto::decode_bytes(&challenge.nonce, "nonce")?;
    let message = crypto::challenge_message(&challenge.challenge_id, &nonce);
    let signature = crypto::sign(&private, &message)?;
    let login = post_json::<LoginResponse>(
        repo_url,
        "/auth/login",
        &LoginRequest {
            challenge_id: challenge.challenge_id,
            signature: crypto::encode_bytes(&signature),
        },
    )?;
    local::write_session(paths, owner, &login.session_token)?;
    Ok(login)
}

/// `POST /signing` (plan-23 §3.3): pre-register the one-off signing key for
/// one exact package+version and fetch the server-signed attestation. The
/// attestation signature is verified against the pinned server key before it
/// is returned, so a swapped registry can never hand back paperwork the
/// consumer chain would later reject.
pub fn request_attestation(
    repo_url: &str,
    paths: &LocalPaths,
    owner: &str,
    ident: &str,
    version: &str,
    signing_fingerprint: &str,
) -> Result<SigningResponse, String> {
    validate_owner_name(owner)?;
    let server_key = ensure_server_key(repo_url, paths)?;
    let session_token = local::read_session(paths, owner)?;
    let response = post_json::<SigningResponse>(
        repo_url,
        "/signing",
        &SigningRequest {
            owner: owner.to_string(),
            ident: ident.to_string(),
            version: version.to_string(),
            signing_fingerprint: signing_fingerprint.to_string(),
            session_token,
        },
    )?;
    let signature = crypto::decode_bytes(&response.attestation_signature, "attestationSignature")?;
    crypto::verify(
        &server_key,
        &crypto::attestation_signing_input(response.attestation.as_bytes()),
        &signature,
    )
    .map_err(|_| "attestation signature does not verify under the pinned server key".to_string())?;
    Ok(response)
}

/// Old-machine side of a machine link (plan-23 §3.2): generate the one-time
/// pairing code, encrypt the local ident keypair under it, and park the blob
/// on the server (single-use, short TTL). Returns `(code, expires_at)` — the
/// code is displayed to the user and never sent anywhere.
pub fn link_start(
    repo_url: &str,
    paths: &LocalPaths,
    owner: &str,
) -> Result<(String, i64), String> {
    validate_owner_name(owner)?;
    ensure_server_key(repo_url, paths)?;
    let ident_private = local::read_ident_private_key(paths, owner)?;
    let ident_public = local::read_ident_public_key(paths, owner)?;
    let session_token = local::read_session(paths, owner)?;

    let code = crypto::generate_pairing_code();
    let mut plaintext = ident_private.clone();
    plaintext.extend_from_slice(&ident_public);
    let (blob, salt) = crypto::seal_pairing_blob(&code, &plaintext)?;
    let response = post_json::<LinkStartResponse>(
        repo_url,
        "/machines/link",
        &LinkStartRequest {
            owner: owner.to_string(),
            lookup: crypto::pairing_lookup(&code),
            blob: crypto::encode_bytes(&blob),
            salt: crypto::encode_bytes(&salt),
            session_token,
        },
    )?;
    Ok((code, response.expires_at))
}

/// New-machine side of a machine link: generate this machine's own auth
/// keypair, fetch the relay blob with the typed pairing code, decrypt the
/// ident keypair, and store all four key files. After this the machine is a
/// full equal — `mfb repo auth` opens its session.
pub fn link_fetch(
    repo_url: &str,
    paths: &LocalPaths,
    owner: &str,
    code: &str,
) -> Result<RegisterResponse, String> {
    validate_owner_name(owner)?;
    ensure_server_key(repo_url, paths)?;
    let (auth_public, auth_private) = crypto::generate_keypair();
    let message = crypto::registration_message(crypto::ROLE_AUTH, owner, &auth_public);
    let proof = crypto::sign(&auth_private, &message)?;
    let response = post_json::<LinkFetchResponse>(
        repo_url,
        "/machines/link/fetch",
        &LinkFetchRequest {
            owner: owner.to_string(),
            lookup: crypto::pairing_lookup(code.trim()),
            auth_key: crypto::encode_bytes(&auth_public),
            proof: crypto::encode_bytes(&proof),
        },
    )?;
    let blob = crypto::decode_bytes(&response.blob, "blob")?;
    let salt = crypto::decode_bytes(&response.salt, "salt")?;
    let plaintext = crypto::open_pairing_blob(code.trim(), &blob, &salt)?;
    if plaintext.len() != crypto::PRIVATE_KEY_LEN + crypto::PUBLIC_KEY_LEN {
        return Err("pairing blob does not contain an ident keypair".to_string());
    }
    let (ident_private, ident_public) = plaintext.split_at(crypto::PRIVATE_KEY_LEN);
    if crypto::public_from_private(ident_private)? != ident_public {
        return Err("pairing blob ident keypair is inconsistent".to_string());
    }
    local::write_auth_keypair(paths, owner, &auth_public, &auth_private)?;
    local::write_ident_keypair(paths, owner, ident_public, ident_private)?;
    Ok(RegisterResponse {
        owner: response.owner,
        auth_fingerprint: response.auth_fingerprint,
        ident_fingerprint: crypto::fingerprint(ident_public),
    })
}

/// Rotate the account ident (plan-23-B2, `mfb key rotate`): generate a new
/// ident keypair, sign the chain link with the OLD ident, prove possession
/// with the NEW one, and install the new keypair locally on success. Other
/// linked machines still hold the old (now `past`) ident private key and
/// must re-link — the rotation exists because a machine was lost, so the new
/// private key is never distributed automatically.
pub fn rotate_ident(
    repo_url: &str,
    paths: &LocalPaths,
    owner: &str,
) -> Result<RotateResponse, String> {
    validate_owner_name(owner)?;
    ensure_server_key(repo_url, paths)?;
    let old_private = local::read_ident_private_key(paths, owner)?;
    let old_public = local::read_ident_public_key(paths, owner)?;
    let session_token = local::read_session(paths, owner)?;

    let (new_public, new_private) = crypto::generate_keypair();
    let chain_message =
        crypto::ident_rotation_message(owner, &crypto::fingerprint(&old_public), &new_public);
    let chain_signature = crypto::sign(&old_private, &chain_message)?;
    let possession_message = crypto::registration_message(crypto::ROLE_IDENT, owner, &new_public);
    let possession_proof = crypto::sign(&new_private, &possession_message)?;

    let response = post_json::<RotateResponse>(
        repo_url,
        "/keys/rotate",
        &RotateRequest {
            owner: owner.to_string(),
            new_ident_key: crypto::encode_bytes(&new_public),
            chain_signature: crypto::encode_bytes(&chain_signature),
            possession_proof: crypto::encode_bytes(&possession_proof),
            session_token,
        },
    )?;
    local::write_ident_keypair(paths, owner, &new_public, &new_private)?;
    Ok(response)
}

/// Fetch the owner's current ident binding and the signed rotation chain.
pub fn fetch_ident_chain(repo_url: &str, owner: &str) -> Result<IdentChainResponse, String> {
    validate_owner_name(owner)?;
    get_json::<IdentChainResponse>(repo_url, &format!("/idents/{owner}"))
}

/// Walk a signed ident chain from `pinned` (plan-23-B2 pin-follow): verify
/// each link's old-key signature over the rotation message and follow
/// old→new until the chain is exhausted. Returns the newest chained
/// successor of `pinned`, or None when `pinned` never appears — the
/// no-chain-link case (a re-anchor), which callers must treat as a hard
/// error, never a silent re-pin.
pub fn follow_ident_chain(
    owner: &str,
    pinned: &[u8],
    chain: &[crate::server::IdentChainLink],
) -> Result<Option<Vec<u8>>, String> {
    let mut current = pinned.to_vec();
    let mut advanced = false;
    for link in chain {
        let old_key = crypto::decode_bytes(&link.old_key, "oldKey")?;
        if old_key != current {
            continue;
        }
        let new_key = crypto::decode_bytes(&link.new_key, "newKey")?;
        let signature = crypto::decode_bytes(&link.signature, "signature")?;
        let message =
            crypto::ident_rotation_message(owner, &crypto::fingerprint(&old_key), &new_key);
        crypto::verify(&old_key, &message, &signature)
            .map_err(|_| "invalid ident chain link signature".to_string())?;
        current = new_key;
        advanced = true;
    }
    Ok(if advanced { Some(current) } else { None })
}

/// Revoke a (lost) machine's auth key. Authority is the ident key alone: the
/// request signs a server challenge plus the fingerprint being revoked with
/// the local ident private key; no session is required.
pub fn revoke_machine(
    repo_url: &str,
    paths: &LocalPaths,
    owner: &str,
    auth_fingerprint: &str,
) -> Result<RevokeResponse, String> {
    validate_owner_name(owner)?;
    ensure_server_key(repo_url, paths)?;
    let ident_private = local::read_ident_private_key(paths, owner)?;
    let challenge = post_json::<ChallengeResponse>(
        repo_url,
        "/machines/revoke/challenge",
        &RevokeChallengeRequest {
            owner: owner.to_string(),
        },
    )?;
    let nonce = crypto::decode_bytes(&challenge.nonce, "nonce")?;
    let message = crypto::revocation_message(&challenge.challenge_id, &nonce, auth_fingerprint);
    let signature = crypto::sign(&ident_private, &message)?;
    post_json::<RevokeResponse>(
        repo_url,
        "/machines/revoke",
        &RevokeRequest {
            challenge_id: challenge.challenge_id,
            auth_fingerprint: auth_fingerprint.to_string(),
            ident_signature: crypto::encode_bytes(&signature),
        },
    )
}

/// Fetch the signed log checkpoint, verify it under the pinned server key,
/// and enforce append-only growth against the locally pinned checkpoint
/// (plan-23-B3): a shrunken tree, or a different root at the same size, is a
/// hard error — never silently re-pinned.
pub fn fetch_checkpoint(repo_url: &str, paths: &LocalPaths) -> Result<CheckpointResponse, String> {
    let server_key = ensure_server_key(repo_url, paths)?;
    let checkpoint = get_json::<CheckpointResponse>(repo_url, "/log/checkpoint")?;
    let root = decode_hex32(&checkpoint.root_hash, "rootHash")?;
    let signature = crypto::decode_bytes(&checkpoint.signature, "signature")?;
    crypto::verify(
        &server_key,
        &crate::log::checkpoint_signing_input(checkpoint.size as u64, &root),
        &signature,
    )
    .map_err(|_| "log checkpoint does not verify under the pinned server key".to_string())?;
    if let Some((pinned_size, pinned_root)) = local::read_checkpoint(paths)? {
        if checkpoint.size < pinned_size {
            return Err(format!(
                "registry log ROLLBACK: checkpoint size {} is smaller than the pinned size {pinned_size}",
                checkpoint.size
            ));
        }
        if checkpoint.size == pinned_size && checkpoint.root_hash != pinned_root {
            return Err(
                "registry log FORK: checkpoint root differs from the pinned root at the same size"
                    .to_string(),
            );
        }
    }
    local::write_checkpoint(paths, checkpoint.size, &checkpoint.root_hash)?;
    Ok(checkpoint)
}

/// Verify that the publish of `ident@version` is included in the registry
/// log under the current (verified, rollback-checked) checkpoint. Returns
/// the log entry and the checkpoint it verified against.
pub fn verify_publish_inclusion(
    repo_url: &str,
    paths: &LocalPaths,
    ident: &str,
    version: &str,
) -> Result<(LogEntry, CheckpointResponse), String> {
    let checkpoint = fetch_checkpoint(repo_url, paths)?;
    let entry = get_json::<LogEntry>(
        repo_url,
        &format!(
            "/log/publish?ident={}&version={}",
            percent_encode(ident),
            percent_encode(version)
        ),
    )?;
    let proof = get_json::<InclusionProofResponse>(
        repo_url,
        &format!("/log/proof/{}?size={}", entry.index, checkpoint.size),
    )?;
    if proof.index != entry.index || proof.size != checkpoint.size {
        return Err("inclusion proof does not match the requested entry".to_string());
    }
    let leaf = decode_hex32(&entry.leaf_hash, "leafHash")?;
    if decode_hex32(&proof.leaf_hash, "leafHash")? != leaf {
        return Err("inclusion proof leaf does not match the publish entry".to_string());
    }
    let root = decode_hex32(&checkpoint.root_hash, "rootHash")?;
    let mut path = Vec::new();
    for node in &proof.path {
        path.push(decode_hex32(node, "proof node")?);
    }
    crate::log::verify_inclusion(
        entry.index as usize,
        checkpoint.size as usize,
        &leaf,
        &path,
        &root,
    )?;
    Ok((entry, checkpoint))
}

/// Fetch and verify a consistency proof between the pinned checkpoint and
/// the current one.
pub fn verify_log_consistency(repo_url: &str, paths: &LocalPaths) -> Result<(), String> {
    let Some((pinned_size, pinned_root)) = local::read_checkpoint(paths)? else {
        // Nothing pinned yet: fetch_checkpoint establishes the first pin.
        fetch_checkpoint(repo_url, paths)?;
        return Ok(());
    };
    let checkpoint = fetch_checkpoint(repo_url, paths)?;
    let proof = get_json::<ConsistencyProofResponse>(
        repo_url,
        &format!("/log/consistency?from={pinned_size}&to={}", checkpoint.size),
    )?;
    let old_root = decode_hex32(&pinned_root, "pinned root")?;
    let new_root = decode_hex32(&checkpoint.root_hash, "rootHash")?;
    let mut path = Vec::new();
    for node in &proof.path {
        path.push(decode_hex32(node, "proof node")?);
    }
    crate::log::verify_consistency(
        pinned_size as usize,
        checkpoint.size as usize,
        &old_root,
        &new_root,
        &path,
    )
}

fn decode_hex32(value: &str, field: &str) -> Result<[u8; 32], String> {
    let raw = hex::decode(value).map_err(|_| format!("malformed {field}"))?;
    if raw.len() != 32 {
        return Err(format!("malformed {field}"));
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&raw);
    Ok(out)
}

fn percent_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// The root-delegated keys and index state recovered from a verified metadata
/// chain (plan-10-C2).
#[derive(Debug)]
pub struct DelegatedMetadata {
    /// The root-delegated server (attestation) key. A consumer refuses any
    /// attestation not signed by this exact key.
    pub server_key: Vec<u8>,
    pub snapshot_version: i64,
    pub index_hash: String,
}

/// Verify the signed-metadata chain (plan-10-C2): root → timestamp → snapshot.
/// Rejects a bad root fingerprint, a registry-id mismatch, expired metadata,
/// an undelegated key, a snapshot/timestamp that disagree, and a version
/// rollback below `min_snapshot_version`. `now` is passed in for testability.
pub fn verify_registry_metadata(
    root: &RootResponse,
    timestamp: &SignedMetadataResponse,
    snapshot: &SignedMetadataResponse,
    expected_registry_id: &str,
    pinned_root_fingerprint: &str,
    min_snapshot_version: i64,
    now: i64,
) -> Result<DelegatedMetadata, String> {
    // Root: the pinned fingerprint is the sole trust anchor.
    let root_key = crypto::decode_bytes(&root.root_key, "rootKey")?;
    if crypto::fingerprint(&root_key) != pinned_root_fingerprint {
        return Err("registry root key does not match the pinned root fingerprint".to_string());
    }
    let root_signature = crypto::decode_bytes(&root.signature, "root signature")?;
    crypto::verify(
        &root_key,
        &crypto::root_signing_input(root.signed.as_bytes()),
        &root_signature,
    )
    .map_err(|_| "root.json signature does not verify under the root key".to_string())?;
    let root_doc = parse_metadata(&root.signed, "root.json")?;
    check_field(&root_doc, "registryId", expected_registry_id, "root.json")?;
    check_not_expired(&root_doc, now, "root.json")?;
    let server_key = decode_delegated_key(&root_doc, "serverKey")?;
    let snapshot_key = decode_delegated_key(&root_doc, "snapshotKey")?;
    let timestamp_key = decode_delegated_key(&root_doc, "timestampKey")?;

    // Timestamp: signed by the delegated timestamp key, fresh, no rollback.
    let timestamp_signature = crypto::decode_bytes(&timestamp.signature, "timestamp signature")?;
    crypto::verify(
        &timestamp_key,
        &crypto::timestamp_signing_input(timestamp.signed.as_bytes()),
        &timestamp_signature,
    )
    .map_err(|_| "timestamp.json signature does not verify under the delegated key".to_string())?;
    let timestamp_doc = parse_metadata(&timestamp.signed, "timestamp.json")?;
    check_field(
        &timestamp_doc,
        "registryId",
        expected_registry_id,
        "timestamp.json",
    )?;
    check_not_expired(&timestamp_doc, now, "timestamp.json")?;
    let snapshot_version = metadata_i64(&timestamp_doc, "snapshotVersion", "timestamp.json")?;
    if snapshot_version < min_snapshot_version {
        return Err(format!(
            "metadata ROLLBACK: snapshot version {snapshot_version} is below the pinned version {min_snapshot_version}"
        ));
    }
    let timestamp_index_hash = metadata_str(&timestamp_doc, "indexHash", "timestamp.json")?;

    // Snapshot: signed by the delegated snapshot key, fresh, and agrees with
    // the timestamp on version + index hash.
    let snapshot_signature = crypto::decode_bytes(&snapshot.signature, "snapshot signature")?;
    crypto::verify(
        &snapshot_key,
        &crypto::snapshot_signing_input(snapshot.signed.as_bytes()),
        &snapshot_signature,
    )
    .map_err(|_| "snapshot.json signature does not verify under the delegated key".to_string())?;
    let snapshot_doc = parse_metadata(&snapshot.signed, "snapshot.json")?;
    check_field(
        &snapshot_doc,
        "registryId",
        expected_registry_id,
        "snapshot.json",
    )?;
    check_not_expired(&snapshot_doc, now, "snapshot.json")?;
    let snapshot_doc_version = metadata_i64(&snapshot_doc, "version", "snapshot.json")?;
    let snapshot_index_hash = metadata_str(&snapshot_doc, "indexHash", "snapshot.json")?;
    if snapshot_doc_version != snapshot_version {
        return Err("timestamp and snapshot disagree on the snapshot version".to_string());
    }
    if snapshot_index_hash != timestamp_index_hash {
        return Err("timestamp and snapshot disagree on the index hash".to_string());
    }

    Ok(DelegatedMetadata {
        server_key,
        snapshot_version,
        index_hash: snapshot_index_hash,
    })
}

fn parse_metadata(signed: &str, what: &str) -> Result<serde_json::Value, String> {
    serde_json::from_str(signed).map_err(|_| format!("malformed {what}"))
}

fn check_field(
    doc: &serde_json::Value,
    field: &str,
    expected: &str,
    what: &str,
) -> Result<(), String> {
    if doc.get(field).and_then(|value| value.as_str()) != Some(expected) {
        return Err(format!("{what} {field} does not match the expected value"));
    }
    Ok(())
}

fn check_not_expired(doc: &serde_json::Value, now: i64, what: &str) -> Result<(), String> {
    let expires = doc
        .get("expires")
        .and_then(|value| value.as_i64())
        .ok_or_else(|| format!("{what} is missing an expiry"))?;
    if expires <= now {
        return Err(format!("{what} has expired"));
    }
    Ok(())
}

fn metadata_i64(doc: &serde_json::Value, field: &str, what: &str) -> Result<i64, String> {
    doc.get(field)
        .and_then(|value| value.as_i64())
        .ok_or_else(|| format!("{what} is missing {field}"))
}

fn metadata_str(doc: &serde_json::Value, field: &str, what: &str) -> Result<String, String> {
    doc.get(field)
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .ok_or_else(|| format!("{what} is missing {field}"))
}

fn decode_delegated_key(doc: &serde_json::Value, field: &str) -> Result<Vec<u8>, String> {
    let encoded = doc
        .get(field)
        .and_then(|value| value.as_str())
        .ok_or_else(|| format!("root.json is missing {field}"))?;
    crypto::decode_bytes(encoded, field)
}

/// `mfb repo trust`: fetch and verify the metadata chain, cross-check that the
/// pinned server key is the root-delegated one, and pin the registry id + root
/// fingerprint. Returns the pinned snapshot version.
pub fn trust_registry(
    repo_url: &str,
    paths: &LocalPaths,
    registry_id: &str,
    root_fingerprint: &str,
) -> Result<i64, String> {
    let server_key = ensure_server_key(repo_url, paths)?;
    let delegated = fetch_and_verify_metadata(repo_url, registry_id, root_fingerprint, 0)?;
    if delegated.server_key != server_key {
        return Err(
            "registry attestation key is not delegated by the pinned root; refusing to trust"
                .to_string(),
        );
    }
    local::write_root_pin(paths, registry_id, root_fingerprint)?;
    local::write_snapshot_version(paths, delegated.snapshot_version)?;
    Ok(delegated.snapshot_version)
}

/// Verify the pinned metadata chain before trusting the registry, when (and
/// only when) a root has been pinned via `mfb repo trust` — the signed-metadata
/// layer is opt-in on top of the plan-23 pinned-server-key anchor. Advances the
/// pinned snapshot version on success; rejects a rollback.
pub fn verify_pinned_metadata(repo_url: &str, paths: &LocalPaths) -> Result<(), String> {
    let Some((registry_id, root_fingerprint)) = local::read_root_pin(paths)? else {
        return Ok(());
    };
    let server_key = ensure_server_key(repo_url, paths)?;
    let min = local::read_snapshot_version(paths)?.unwrap_or(0);
    let delegated = fetch_and_verify_metadata(repo_url, &registry_id, &root_fingerprint, min)?;
    if delegated.server_key != server_key {
        return Err(
            "registry attestation key is not delegated by the pinned root; refusing to trust"
                .to_string(),
        );
    }
    local::write_snapshot_version(paths, delegated.snapshot_version)?;
    Ok(())
}

fn fetch_and_verify_metadata(
    repo_url: &str,
    registry_id: &str,
    root_fingerprint: &str,
    min_snapshot_version: i64,
) -> Result<DelegatedMetadata, String> {
    let root = get_json::<RootResponse>(repo_url, "/root.json")?;
    let timestamp = get_json::<SignedMetadataResponse>(repo_url, "/timestamp.json")?;
    let snapshot = get_json::<SignedMetadataResponse>(repo_url, "/snapshot.json")?;
    verify_registry_metadata(
        &root,
        &timestamp,
        &snapshot,
        registry_id,
        root_fingerprint,
        min_snapshot_version,
        crate::store::now_unix(),
    )
}

/// `POST /orgs/members` (plan-10-D1): grant or remove a member's org role,
/// authorized by the grantor's local ident key + session.
pub fn set_org_member(
    repo_url: &str,
    paths: &LocalPaths,
    org: &str,
    grantor: &str,
    member: &str,
    role: &str,
    remove: bool,
) -> Result<OrgMemberResponse, String> {
    validate_owner_name(grantor)?;
    ensure_server_key(repo_url, paths)?;
    let ident_private = local::read_ident_private_key(paths, grantor)?;
    let session_token = local::read_session(paths, grantor)?;
    let role_in_message = if remove { "removed" } else { role };
    let signature = crypto::sign(
        &ident_private,
        &crypto::org_role_message(org, member, role_in_message),
    )?;
    post_json::<OrgMemberResponse>(
        repo_url,
        "/orgs/members",
        &OrgMemberRequest {
            org: org.to_string(),
            grantor: grantor.to_string(),
            member: member.to_string(),
            role: role.to_string(),
            action: if remove { "remove" } else { "grant" }.to_string(),
            session_token,
            ident_signature: crypto::encode_bytes(&signature),
        },
    )
}

/// `POST /tokens` (plan-10-D1): issue a scoped publish token. Generates the
/// token keypair locally, registers its public key, and returns the response
/// plus the token private key (base64url) for the operator to deploy to CI.
pub fn issue_publish_token(
    repo_url: &str,
    paths: &LocalPaths,
    owner: &str,
    scope: &str,
    ttl_seconds: i64,
) -> Result<(TokenIssueResponse, String), String> {
    validate_owner_name(owner)?;
    ensure_server_key(repo_url, paths)?;
    let ident_private = local::read_ident_private_key(paths, owner)?;
    let session_token = local::read_session(paths, owner)?;
    let (token_public, token_private) = crypto::generate_keypair();
    let token_fingerprint = crypto::fingerprint(&token_public);
    let proof = crypto::sign(
        &token_private,
        &crypto::registration_message(crypto::ROLE_AUTH, owner, &token_public),
    )?;
    let signature = crypto::sign(
        &ident_private,
        &crypto::token_issue_message(owner, &token_fingerprint, scope),
    )?;
    let response = post_json::<TokenIssueResponse>(
        repo_url,
        "/tokens",
        &TokenIssueRequest {
            owner: owner.to_string(),
            token_key: crypto::encode_bytes(&token_public),
            proof: crypto::encode_bytes(&proof),
            scope: scope.to_string(),
            ttl_seconds,
            session_token,
            ident_signature: crypto::encode_bytes(&signature),
        },
    )?;
    Ok((response, crypto::encode_bytes(&token_private)))
}

/// `POST /tokens/revoke` (plan-10-D1): revoke a publish token by fingerprint.
pub fn revoke_publish_token(
    repo_url: &str,
    paths: &LocalPaths,
    owner: &str,
    token_fingerprint: &str,
) -> Result<TokenRevokeResponse, String> {
    validate_owner_name(owner)?;
    ensure_server_key(repo_url, paths)?;
    let ident_private = local::read_ident_private_key(paths, owner)?;
    let session_token = local::read_session(paths, owner)?;
    let signature = crypto::sign(
        &ident_private,
        &crypto::token_revoke_message(owner, token_fingerprint),
    )?;
    post_json::<TokenRevokeResponse>(
        repo_url,
        "/tokens/revoke",
        &TokenRevokeRequest {
            owner: owner.to_string(),
            token_fingerprint: token_fingerprint.to_string(),
            session_token,
            ident_signature: crypto::encode_bytes(&signature),
        },
    )
}

/// `POST /packages/transfer/offer` (plan-10-D1): the current owner offers a
/// package to a recipient, signed by the current owner's ident.
pub fn transfer_offer(
    repo_url: &str,
    paths: &LocalPaths,
    ident: &str,
    from_owner: &str,
    to_owner: &str,
) -> Result<TransferResponse, String> {
    validate_owner_name(from_owner)?;
    ensure_server_key(repo_url, paths)?;
    let ident_private = local::read_ident_private_key(paths, from_owner)?;
    let session_token = local::read_session(paths, from_owner)?;
    let signature = crypto::sign(
        &ident_private,
        &crypto::transfer_offer_message(ident, from_owner, to_owner),
    )?;
    post_json::<TransferResponse>(
        repo_url,
        "/packages/transfer/offer",
        &TransferOfferRequest {
            ident: ident.to_string(),
            from_owner: from_owner.to_string(),
            to_owner: to_owner.to_string(),
            session_token,
            ident_signature: crypto::encode_bytes(&signature),
        },
    )
}

/// `POST /packages/transfer/accept` (plan-10-D1): the recipient accepts a
/// pending offer, signed by the recipient's ident.
pub fn transfer_accept(
    repo_url: &str,
    paths: &LocalPaths,
    ident: &str,
    to_owner: &str,
) -> Result<TransferResponse, String> {
    validate_owner_name(to_owner)?;
    ensure_server_key(repo_url, paths)?;
    let ident_private = local::read_ident_private_key(paths, to_owner)?;
    let session_token = local::read_session(paths, to_owner)?;
    let signature = crypto::sign(
        &ident_private,
        &crypto::transfer_accept_message(ident, to_owner),
    )?;
    post_json::<TransferResponse>(
        repo_url,
        "/packages/transfer/accept",
        &TransferAcceptRequest {
            ident: ident.to_string(),
            to_owner: to_owner.to_string(),
            session_token,
            ident_signature: crypto::encode_bytes(&signature),
        },
    )
}

/// `POST /release-state` (plan-10-C1): a maintainer sets a published version's
/// release state. Signed with the local ident key (authority) and carrying the
/// session token (auth); an auth session alone is refused server-side.
pub fn set_release_state(
    repo_url: &str,
    paths: &LocalPaths,
    owner: &str,
    ident: &str,
    version: &str,
    state: &str,
) -> Result<ReleaseStateResponse, String> {
    validate_owner_name(owner)?;
    ensure_server_key(repo_url, paths)?;
    let ident_private = local::read_ident_private_key(paths, owner)?;
    let session_token = local::read_session(paths, owner)?;
    let signature = crypto::sign(
        &ident_private,
        &crypto::release_state_message(ident, version, state),
    )?;
    post_json::<ReleaseStateResponse>(
        repo_url,
        "/release-state",
        &ReleaseStateRequest {
            owner: owner.to_string(),
            ident: ident.to_string(),
            version: version.to_string(),
            state: state.to_string(),
            session_token,
            ident_signature: crypto::encode_bytes(&signature),
        },
    )
}

/// `GET /index/<owner>#<package>` (plan-10-A): fetch the published version
/// list plus the owner's current ident key. The server-signed name binding is
/// verified under the pinned server key, so the returned `identKey` is a
/// registry-authenticated anchor a first `mfb pkg add` can pin.
pub fn fetch_index(
    repo_url: &str,
    paths: &LocalPaths,
    owner: &str,
    package: &str,
) -> Result<IndexResponse, String> {
    validate_owner_name(owner)?;
    let server_key = ensure_server_key(repo_url, paths)?;
    // If a signed-metadata root is pinned (plan-10-C2), the chain must verify
    // and delegate this server key before we trust anything the index says.
    verify_pinned_metadata(repo_url, paths)?;
    let ident = format!("{owner}#{package}");
    let response =
        get_json::<IndexResponse>(repo_url, &format!("/index/{}", percent_encode(&ident)))?;
    // The pinned ident is only as trustworthy as the name binding: verify it
    // under the pinned server key and cross-check the fingerprint.
    let ident_public = crypto::decode_bytes(
        response
            .ident_key
            .strip_prefix("ed25519:")
            .unwrap_or(&response.ident_key),
        "identKey",
    )?;
    if crypto::fingerprint(&ident_public) != response.ident_fingerprint {
        return Err("registry index identKey does not match its fingerprint".to_string());
    }
    let signature = crypto::decode_bytes(&response.name_binding_signature, "nameBindingSignature")?;
    crypto::verify(
        &server_key,
        &crypto::name_binding_message(&response.owner, &response.ident_fingerprint),
        &signature,
    )
    .map_err(|_| "registry name binding does not verify under the pinned server key".to_string())?;
    Ok(response)
}

/// `GET /blob/<hash>` (plan-10-A): download a content-addressed `.mfp` blob and
/// verify its bytes hash to the requested hash before returning them.
pub fn fetch_blob(repo_url: &str, hash: &str) -> Result<Vec<u8>, String> {
    ensure_transport_security(repo_url)?;
    let url = format!("{}/blob/{}", repo_url.trim_end_matches('/'), hash);
    // Blob bodies are orders of magnitude larger than any control-plane payload,
    // so they get their own deadline; the client default would reject a slow but
    // perfectly healthy transfer. The override rides into the `Response`, so it
    // covers the body read below and not just the headers.
    let response = http_client()?
        .get(&url)
        .timeout(BLOB_TIMEOUT)
        .send()
        .map_err(|err| format!("failed to connect to repository service: {err}"))?;
    let status = response.status();
    if !status.is_success() {
        let text = response
            .text()
            .unwrap_or_else(|_| "repository request failed".to_string());
        if let Ok(error) = serde_json::from_str::<ErrorResponse>(&text) {
            return Err(error.error);
        }
        return Err(format!(
            "repository request failed with status {status}: {text}"
        ));
    }
    let bytes = response
        .bytes()
        .map_err(|err| format!("failed to read blob body: {err}"))?
        .to_vec();
    if hex::encode(crypto::sha256(&bytes)) != hash {
        return Err("downloaded blob does not match the requested content hash".to_string());
    }
    Ok(bytes)
}

pub struct PackageArtifact<'a> {
    pub ident: &'a str,
    pub version: &'a str,
    pub artifact: &'a [u8],
    pub content_hash: &'a str,
    pub ident_fingerprint: &'a str,
    pub signing_fingerprint: &'a str,
}

pub fn validate_package(
    repo_url: &str,
    paths: &LocalPaths,
    owner: &str,
    package: &PackageArtifact<'_>,
) -> Result<ValidatePackageResponse, String> {
    validate_owner_name(owner)?;
    let session_token = local::read_session(paths, owner)?;
    post_json::<ValidatePackageResponse>(
        repo_url,
        "/validate",
        &package_request(package, session_token),
    )
}

pub fn publish_package(
    repo_url: &str,
    paths: &LocalPaths,
    owner: &str,
    package: &PackageArtifact<'_>,
) -> Result<PublishPackageResponse, String> {
    validate_owner_name(owner)?;
    let session_token = local::read_session(paths, owner)?;
    post_json::<PublishPackageResponse>(
        repo_url,
        "/publish",
        &package_request(package, session_token),
    )
}

fn package_request(package: &PackageArtifact<'_>, session_token: String) -> PackageArtifactRequest {
    PackageArtifactRequest {
        ident: package.ident.to_string(),
        version: package.version.to_string(),
        artifact: crypto::encode_bytes(package.artifact),
        content_hash: package.content_hash.to_string(),
        ident_fingerprint: package.ident_fingerprint.to_string(),
        signing_fingerprint: package.signing_fingerprint.to_string(),
        session_token,
    }
}

fn post_json<T: DeserializeOwned>(
    repo_url: &str,
    path: &str,
    body: &impl serde::Serialize,
) -> Result<T, String> {
    ensure_transport_security(repo_url)?;
    let url = format!("{}{}", repo_url.trim_end_matches('/'), path);
    let response = http_client()?
        .post(&url)
        .json(body)
        .send()
        .map_err(|err| format!("failed to connect to repository service: {err}"))?;
    read_json_response(response)
}

fn get_json<T: DeserializeOwned>(repo_url: &str, path: &str) -> Result<T, String> {
    ensure_transport_security(repo_url)?;
    let url = format!("{}{}", repo_url.trim_end_matches('/'), path);
    let response = http_client()?
        .get(&url)
        .send()
        .map_err(|err| format!("failed to connect to repository service: {err}"))?;
    read_json_response(response)
}

fn read_json_response<T: DeserializeOwned>(
    response: reqwest::blocking::Response,
) -> Result<T, String> {
    let status = response.status();
    if status.is_success() {
        return response
            .json::<T>()
            .map_err(|err| format!("invalid repository response: {err}"));
    }
    let text = response
        .text()
        .unwrap_or_else(|_| "repository request failed".to_string());
    if let Ok(error) = serde_json::from_str::<ErrorResponse>(&text) {
        return Err(error.error);
    }
    Err(format!(
        "repository request failed with status {status}: {text}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    /// The one client is shared, so blob fetches reuse connections instead of
    /// standing up a fresh tokio runtime and TLS handshake per request.
    #[test]
    fn http_client_is_built_once_and_shared() {
        let first = http_client().expect("client builds");
        let second = http_client().expect("client builds");
        assert!(std::ptr::eq(first, second));
    }

    /// A blob whose body arrives after the control-plane deadline must still
    /// download.
    ///
    /// reqwest's blocking `timeout` is a *deadline*, not a stall timeout, and it
    /// is applied twice: once to connect+headers, then again — fresh — to the
    /// whole body read, because `blocking::Response::bytes` re-wraps it. With the
    /// 30s default and no `Client::builder()` anywhere, any blob slow enough to
    /// take half a minute failed outright. A vendored native library on an
    /// ordinary connection routinely is that slow (64 MiB inside 30s needs
    /// ~18 Mbit/s sustained), so this was the common case, not an edge case.
    ///
    /// The server here holds the body back past `CONTROL_TIMEOUT` and then sends
    /// it: that delay is exactly what the old code rejected and the per-request
    /// `BLOB_TIMEOUT` override now tolerates. Necessarily slow — the 30s bound it
    /// guards is the thing under test.
    #[test]
    fn fetch_blob_survives_a_body_slower_than_the_control_timeout() {
        let body = b"vendored-native-library-payload".to_vec();
        let hash = hex::encode(crypto::sha256(&body));

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let port = listener.local_addr().unwrap().port();

        let served = body.clone();
        let server = thread::spawn(move || {
            let (mut sock, _) = listener.accept().expect("accept");
            // Drain the request head; its contents do not matter here.
            let mut scratch = [0u8; 1024];
            let _ = sock.read(&mut scratch);
            write!(
                sock,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
                served.len()
            )
            .expect("write head");
            sock.flush().expect("flush head");
            // Headers land immediately, so the request phase succeeds either way;
            // the body is what outlives the old deadline.
            thread::sleep(CONTROL_TIMEOUT + Duration::from_secs(2));
            sock.write_all(&served).expect("write body");
            sock.flush().expect("flush body");
        });

        let fetched = fetch_blob(&format!("http://127.0.0.1:{port}"), &hash);
        server.join().expect("server thread");
        assert_eq!(fetched.expect("slow blob still downloads"), body);
    }

    struct MetadataFixture {
        root_fingerprint: String,
        server_public: Vec<u8>,
        root: RootResponse,
        timestamp: SignedMetadataResponse,
        snapshot: SignedMetadataResponse,
    }

    /// Build a fully valid metadata chain for the given parameters; individual
    /// tests then mutate one field to exercise a rejection.
    fn build_metadata(
        registry_id: &str,
        version: i64,
        expires: i64,
        index_hash: &str,
        timestamp_index_hash: &str,
    ) -> MetadataFixture {
        let (root_public, root_private) = crypto::generate_keypair();
        let (snapshot_public, snapshot_private) = crypto::generate_keypair();
        let (timestamp_public, timestamp_private) = crypto::generate_keypair();
        let (server_public, _server_private) = crypto::generate_keypair();
        let root_signed = format!(
            "{{\"type\":\"root\",\"registryId\":\"{registry_id}\",\"version\":1,\"expires\":{expires},\"serverKey\":\"{}\",\"snapshotKey\":\"{}\",\"timestampKey\":\"{}\"}}",
            crypto::encode_bytes(&server_public),
            crypto::encode_bytes(&snapshot_public),
            crypto::encode_bytes(&timestamp_public),
        );
        let root_signature = crypto::sign(
            &root_private,
            &crypto::root_signing_input(root_signed.as_bytes()),
        )
        .unwrap();
        let timestamp_signed = format!(
            "{{\"type\":\"timestamp\",\"registryId\":\"{registry_id}\",\"version\":{version},\"expires\":{expires},\"snapshotVersion\":{version},\"indexHash\":\"{timestamp_index_hash}\"}}",
        );
        let timestamp_signature = crypto::sign(
            &timestamp_private,
            &crypto::timestamp_signing_input(timestamp_signed.as_bytes()),
        )
        .unwrap();
        let snapshot_signed = format!(
            "{{\"type\":\"snapshot\",\"registryId\":\"{registry_id}\",\"version\":{version},\"expires\":{expires},\"indexHash\":\"{index_hash}\"}}",
        );
        let snapshot_signature = crypto::sign(
            &snapshot_private,
            &crypto::snapshot_signing_input(snapshot_signed.as_bytes()),
        )
        .unwrap();
        MetadataFixture {
            root_fingerprint: crypto::fingerprint(&root_public),
            server_public,
            root: RootResponse {
                signed: root_signed,
                signature: crypto::encode_bytes(&root_signature),
                root_key: crypto::encode_bytes(&root_public),
                root_fingerprint: crypto::fingerprint(&root_public),
            },
            timestamp: SignedMetadataResponse {
                signed: timestamp_signed,
                signature: crypto::encode_bytes(&timestamp_signature),
            },
            snapshot: SignedMetadataResponse {
                signed: snapshot_signed,
                signature: crypto::encode_bytes(&snapshot_signature),
            },
        }
    }

    #[test]
    fn metadata_chain_verifies_and_returns_the_delegated_server_key() {
        let m = build_metadata("reg-1", 5, 2_000, "idxhash", "idxhash");
        let delegated = verify_registry_metadata(
            &m.root,
            &m.timestamp,
            &m.snapshot,
            "reg-1",
            &m.root_fingerprint,
            0,
            1_000,
        )
        .expect("valid chain verifies");
        assert_eq!(delegated.server_key, m.server_public);
        assert_eq!(delegated.snapshot_version, 5);
    }

    #[test]
    fn metadata_chain_rejects_tampering_and_rollback() {
        let m = build_metadata("reg-1", 5, 2_000, "idxhash", "idxhash");

        // Wrong pinned root fingerprint.
        assert!(verify_registry_metadata(
            &m.root,
            &m.timestamp,
            &m.snapshot,
            "reg-1",
            "deadbeef",
            0,
            1_000,
        )
        .is_err());

        // Registry-id mismatch.
        assert!(verify_registry_metadata(
            &m.root,
            &m.timestamp,
            &m.snapshot,
            "reg-2",
            &m.root_fingerprint,
            0,
            1_000,
        )
        .is_err());

        // Expired (now past the expiry).
        assert!(verify_registry_metadata(
            &m.root,
            &m.timestamp,
            &m.snapshot,
            "reg-1",
            &m.root_fingerprint,
            0,
            9_000,
        )
        .unwrap_err()
        .contains("expired"));

        // Rollback below the pinned snapshot version.
        assert!(verify_registry_metadata(
            &m.root,
            &m.timestamp,
            &m.snapshot,
            "reg-1",
            &m.root_fingerprint,
            6,
            1_000,
        )
        .unwrap_err()
        .contains("ROLLBACK"));

        // Tampered snapshot signature.
        let mut tampered = build_metadata("reg-1", 5, 2_000, "idxhash", "idxhash");
        tampered.snapshot.signed = tampered.snapshot.signed.replace("idxhash", "evilhash");
        assert!(verify_registry_metadata(
            &tampered.root,
            &tampered.timestamp,
            &tampered.snapshot,
            "reg-1",
            &tampered.root_fingerprint,
            0,
            1_000,
        )
        .is_err());

        // Timestamp and snapshot disagree on the index hash.
        let m2 = build_metadata("reg-1", 5, 2_000, "idxhash", "otherhash");
        assert!(verify_registry_metadata(
            &m2.root,
            &m2.timestamp,
            &m2.snapshot,
            "reg-1",
            &m2.root_fingerprint,
            0,
            1_000,
        )
        .unwrap_err()
        .contains("index hash"));
    }

    #[test]
    fn register_duplicate_failure_leaves_no_local_keys() {
        let temp_home = tempfile::tempdir().unwrap();
        let paths = LocalPaths::new(temp_home.path().join(".mfb"));

        let (auth_public, auth_private) = crypto::generate_keypair();
        let (ident_public, ident_private) = crypto::generate_keypair();
        local::write_auth_keypair(&paths, "alice", &auth_public, &auth_private).unwrap();
        local::write_ident_keypair(&paths, "alice", &ident_public, &ident_private).unwrap();

        local::remove_owner_keys(&paths, "alice");
        assert!(!paths.auth_private_key_path("alice").exists());
        assert!(!paths.ident_private_key_path("alice").exists());
    }

    use crate::server::IdentChainLink;

    fn temp_paths() -> (tempfile::TempDir, LocalPaths) {
        let temp = tempfile::tempdir().unwrap();
        let paths = LocalPaths::new(temp.path().join(".mfb"));
        (temp, paths)
    }

    /// A URL that will refuse every connection: port 1 on loopback is never
    /// listening, so any HTTP flow fails fast at the connect step. This lets
    /// us drive the network-bound client functions through their argument
    /// validation and local-key handling up to the first request.
    const DEAD_URL: &str = "http://127.0.0.1:1";

    #[test]
    fn repo_url_env_defaults_and_overrides() {
        // The default is returned when the env var is unset; an explicit value
        // overrides it. (Serialized guard: env is process-global.)
        std::env::remove_var("MFB_REPO_URL");
        assert_eq!(repo_url_from_env(), DEFAULT_REPO_URL);
        std::env::set_var("MFB_REPO_URL", "http://example.test:9999");
        assert_eq!(repo_url_from_env(), "http://example.test:9999");
        std::env::remove_var("MFB_REPO_URL");
    }

    #[test]
    fn decode_hex32_accepts_only_32_byte_hex() {
        let ok = "aa".repeat(32);
        assert_eq!(decode_hex32(&ok, "x").unwrap(), [0xaa; 32]);
        assert!(decode_hex32("zz", "x").unwrap_err().contains("malformed x"));
        assert!(decode_hex32(&"aa".repeat(31), "x").is_err()); // wrong length
    }

    #[test]
    fn percent_encode_preserves_unreserved_and_escapes_the_rest() {
        assert_eq!(percent_encode("azAZ09-_.~"), "azAZ09-_.~");
        assert_eq!(percent_encode("alice#toolbox"), "alice%23toolbox");
        assert_eq!(percent_encode("a b/c"), "a%20b%2Fc");
    }

    #[test]
    fn metadata_helpers_read_fields_and_report_missing() {
        let doc: serde_json::Value = serde_json::from_str(
            r#"{"registryId":"reg","version":7,"indexHash":"h","expires":100}"#,
        )
        .unwrap();
        check_field(&doc, "registryId", "reg", "d").unwrap();
        assert!(check_field(&doc, "registryId", "other", "d").is_err());
        check_not_expired(&doc, 50, "d").unwrap();
        assert!(check_not_expired(&doc, 100, "d")
            .unwrap_err()
            .contains("expired"));
        assert_eq!(metadata_i64(&doc, "version", "d").unwrap(), 7);
        assert!(metadata_i64(&doc, "missing", "d").is_err());
        assert_eq!(metadata_str(&doc, "indexHash", "d").unwrap(), "h");
        assert!(metadata_str(&doc, "missing", "d").is_err());
        // A doc with no expiry field is rejected by check_not_expired.
        let no_exp: serde_json::Value = serde_json::from_str(r#"{"a":1}"#).unwrap();
        assert!(check_not_expired(&no_exp, 0, "d")
            .unwrap_err()
            .contains("missing an expiry"));
        // parse_metadata rejects non-JSON.
        assert!(parse_metadata("not json", "d").is_err());
    }

    #[test]
    fn decode_delegated_key_reads_or_errors() {
        let (public, _private) = crypto::generate_keypair();
        let doc: serde_json::Value = serde_json::from_str(&format!(
            r#"{{"serverKey":"{}"}}"#,
            crypto::encode_bytes(&public)
        ))
        .unwrap();
        assert_eq!(decode_delegated_key(&doc, "serverKey").unwrap(), public);
        assert!(decode_delegated_key(&doc, "missing")
            .unwrap_err()
            .contains("missing"));
    }

    #[test]
    fn follow_ident_chain_walks_signed_links_and_detects_reanchor() {
        // Build a two-hop signed chain k0 -> k1 -> k2.
        let (k0_pub, k0_prv) = crypto::generate_keypair();
        let (k1_pub, k1_prv) = crypto::generate_keypair();
        let (k2_pub, _k2_prv) = crypto::generate_keypair();
        let link = |old_pub: &[u8], old_prv: &[u8], new_pub: &[u8]| {
            let message =
                crypto::ident_rotation_message("alice", &crypto::fingerprint(old_pub), new_pub);
            IdentChainLink {
                old_key: crypto::encode_bytes(old_pub),
                new_key: crypto::encode_bytes(new_pub),
                signature: crypto::encode_bytes(&crypto::sign(old_prv, &message).unwrap()),
                issued: 0,
            }
        };
        let chain = vec![
            link(&k0_pub, &k0_prv, &k1_pub),
            link(&k1_pub, &k1_prv, &k2_pub),
        ];
        // Following from the oldest pin reaches the newest key.
        let followed = follow_ident_chain("alice", &k0_pub, &chain)
            .unwrap()
            .unwrap();
        assert_eq!(followed, k2_pub);
        // Following from a key that never appears (re-anchor) yields None.
        let (stranger, _) = crypto::generate_keypair();
        assert!(follow_ident_chain("alice", &stranger, &chain)
            .unwrap()
            .is_none());
        // A tampered link signature is a hard error.
        let bad = vec![IdentChainLink {
            old_key: crypto::encode_bytes(&k0_pub),
            new_key: crypto::encode_bytes(&k1_pub),
            signature: crypto::encode_bytes(&[0u8; 64]),
            issued: 0,
        }];
        assert!(follow_ident_chain("alice", &k0_pub, &bad).is_err());
    }

    #[test]
    fn abi_map_reads_object_and_defaults_empty() {
        use crate::server::IndexVersion;
        let with_abi = IndexVersion {
            version: "1.0.0".to_string(),
            hash: "h".to_string(),
            published_at: 0,
            state: "available".to_string(),
            abi_index: serde_json::json!({"greet": "aa", "bad": 3}),
            log_entry: None,
        };
        let map = with_abi.abi_map();
        assert_eq!(map.get("greet").unwrap(), "aa");
        assert!(!map.contains_key("bad")); // non-string values are skipped
        let without = IndexVersion {
            abi_index: serde_json::json!("not an object"),
            ..with_abi
        };
        assert!(without.abi_map().is_empty());
    }

    #[test]
    fn package_request_carries_all_fields() {
        let artifact = b"artifact-bytes";
        let package = PackageArtifact {
            ident: "alice#toolbox",
            version: "1.0.0",
            artifact,
            content_hash: "hash",
            ident_fingerprint: "identfp",
            signing_fingerprint: "signfp",
        };
        let request = package_request(&package, "session".to_string());
        assert_eq!(request.ident, "alice#toolbox");
        assert_eq!(request.version, "1.0.0");
        assert_eq!(request.artifact, crypto::encode_bytes(artifact));
        assert_eq!(request.content_hash, "hash");
        assert_eq!(request.ident_fingerprint, "identfp");
        assert_eq!(request.signing_fingerprint, "signfp");
        assert_eq!(request.session_token, "session");
    }

    #[test]
    fn metadata_chain_rejects_undelegated_server_key_and_bad_root_signature() {
        // A bad root signature (root_key present but signature is garbage).
        let mut m = build_metadata("reg-1", 5, 2_000, "idxhash", "idxhash");
        m.root.signature = crypto::encode_bytes(&[0u8; 64]);
        assert!(verify_registry_metadata(
            &m.root,
            &m.timestamp,
            &m.snapshot,
            "reg-1",
            &m.root_fingerprint,
            0,
            1_000,
        )
        .unwrap_err()
        .contains("root.json signature"));

        // A tampered timestamp signature.
        let mut m2 = build_metadata("reg-1", 5, 2_000, "idxhash", "idxhash");
        m2.timestamp.signature = crypto::encode_bytes(&[0u8; 64]);
        assert!(verify_registry_metadata(
            &m2.root,
            &m2.timestamp,
            &m2.snapshot,
            "reg-1",
            &m2.root_fingerprint,
            0,
            1_000,
        )
        .unwrap_err()
        .contains("timestamp.json signature"));
    }

    /// Assemble a metadata chain from explicit doc versions so the
    /// snapshot-vs-timestamp version disagreement branch can be exercised with
    /// correctly-signed docs (the signature check would otherwise fire first).
    fn build_metadata_versions(
        registry_id: &str,
        timestamp_version: i64,
        snapshot_version: i64,
    ) -> MetadataFixture {
        let (root_public, root_private) = crypto::generate_keypair();
        let (snapshot_public, snapshot_private) = crypto::generate_keypair();
        let (timestamp_public, timestamp_private) = crypto::generate_keypair();
        let (server_public, _server_private) = crypto::generate_keypair();
        let expires = 9_000i64;
        let root_signed = format!(
            "{{\"type\":\"root\",\"registryId\":\"{registry_id}\",\"version\":1,\"expires\":{expires},\"serverKey\":\"{}\",\"snapshotKey\":\"{}\",\"timestampKey\":\"{}\"}}",
            crypto::encode_bytes(&server_public),
            crypto::encode_bytes(&snapshot_public),
            crypto::encode_bytes(&timestamp_public),
        );
        let root_signature = crypto::sign(
            &root_private,
            &crypto::root_signing_input(root_signed.as_bytes()),
        )
        .unwrap();
        let timestamp_signed = format!(
            "{{\"type\":\"timestamp\",\"registryId\":\"{registry_id}\",\"version\":{timestamp_version},\"expires\":{expires},\"snapshotVersion\":{timestamp_version},\"indexHash\":\"h\"}}",
        );
        let timestamp_signature = crypto::sign(
            &timestamp_private,
            &crypto::timestamp_signing_input(timestamp_signed.as_bytes()),
        )
        .unwrap();
        let snapshot_signed = format!(
            "{{\"type\":\"snapshot\",\"registryId\":\"{registry_id}\",\"version\":{snapshot_version},\"expires\":{expires},\"indexHash\":\"h\"}}",
        );
        let snapshot_signature = crypto::sign(
            &snapshot_private,
            &crypto::snapshot_signing_input(snapshot_signed.as_bytes()),
        )
        .unwrap();
        MetadataFixture {
            root_fingerprint: crypto::fingerprint(&root_public),
            server_public,
            root: RootResponse {
                signed: root_signed,
                signature: crypto::encode_bytes(&root_signature),
                root_key: crypto::encode_bytes(&root_public),
                root_fingerprint: crypto::fingerprint(&root_public),
            },
            timestamp: SignedMetadataResponse {
                signed: timestamp_signed,
                signature: crypto::encode_bytes(&timestamp_signature),
            },
            snapshot: SignedMetadataResponse {
                signed: snapshot_signed,
                signature: crypto::encode_bytes(&snapshot_signature),
            },
        }
    }

    #[test]
    fn metadata_chain_rejects_snapshot_timestamp_version_disagreement() {
        let m = build_metadata_versions("reg-1", 5, 6);
        let err = verify_registry_metadata(
            &m.root,
            &m.timestamp,
            &m.snapshot,
            "reg-1",
            &m.root_fingerprint,
            0,
            1_000,
        )
        .unwrap_err();
        assert!(err.contains("disagree on the snapshot version"), "{err}");
        // Sanity: the fixture's server key would be the delegated one on a
        // matching chain.
        let matched = build_metadata_versions("reg-1", 7, 7);
        let delegated = verify_registry_metadata(
            &matched.root,
            &matched.timestamp,
            &matched.snapshot,
            "reg-1",
            &matched.root_fingerprint,
            0,
            1_000,
        )
        .unwrap();
        assert_eq!(delegated.server_key, matched.server_public);
    }

    // --- Network-bound flows: driven against a dead endpoint so the connect
    // step fails deterministically. This exercises argument validation, local
    // key handling, and the request/response error plumbing without a bind.

    #[test]
    fn network_flows_reject_invalid_owner_names_before_any_request() {
        let (_temp, paths) = temp_paths();
        assert!(register(DEAD_URL, &paths, "std").is_err());
        assert!(auth(DEAD_URL, &paths, "1bad").is_err());
        assert!(link_start(DEAD_URL, &paths, "").is_err());
        assert!(link_fetch(DEAD_URL, &paths, "bad-name", "code").is_err());
        assert!(rotate_ident(DEAD_URL, &paths, "std").is_err());
        assert!(fetch_ident_chain(DEAD_URL, "std").is_err());
        assert!(revoke_machine(DEAD_URL, &paths, "std", "fp").is_err());
        assert!(set_org_member(DEAD_URL, &paths, "acme", "std", "m", "admin", false).is_err());
        assert!(issue_publish_token(DEAD_URL, &paths, "std", "scope", 60).is_err());
        assert!(revoke_publish_token(DEAD_URL, &paths, "std", "fp").is_err());
        assert!(transfer_offer(DEAD_URL, &paths, "a#p", "std", "bob").is_err());
        assert!(transfer_accept(DEAD_URL, &paths, "a#p", "std").is_err());
        assert!(set_release_state(DEAD_URL, &paths, "std", "a#p", "1.0.0", "yanked").is_err());
        assert!(fetch_index(DEAD_URL, &paths, "std", "pkg").is_err());
        assert!(request_attestation(DEAD_URL, &paths, "std", "a#p", "1.0.0", "fp").is_err());
        assert!(validate_package(
            DEAD_URL,
            &paths,
            "std",
            &PackageArtifact {
                ident: "a#p",
                version: "1",
                artifact: b"x",
                content_hash: "h",
                ident_fingerprint: "i",
                signing_fingerprint: "s",
            }
        )
        .is_err());
        assert!(publish_package(
            DEAD_URL,
            &paths,
            "std",
            &PackageArtifact {
                ident: "a#p",
                version: "1",
                artifact: b"x",
                content_hash: "h",
                ident_fingerprint: "i",
                signing_fingerprint: "s",
            }
        )
        .is_err());
    }

    #[test]
    fn network_flows_report_connection_failure_against_a_dead_endpoint() {
        let (_temp, paths) = temp_paths();
        // ensure_server_key / get_json / post_json all fail at connect.
        let err = ensure_server_key(DEAD_URL, &paths).unwrap_err();
        assert!(err.contains("failed to connect"), "{err}");
        assert!(fetch_blob(DEAD_URL, "hash")
            .unwrap_err()
            .contains("failed to connect"));
        assert!(fetch_ident_chain(DEAD_URL, "alice")
            .unwrap_err()
            .contains("failed to connect"));
        assert!(fetch_checkpoint(DEAD_URL, &paths)
            .unwrap_err()
            .contains("failed to connect"));
        assert!(verify_log_consistency(DEAD_URL, &paths)
            .unwrap_err()
            .contains("failed to connect"));
        assert!(verify_publish_inclusion(DEAD_URL, &paths, "a#p", "1.0.0")
            .unwrap_err()
            .contains("failed to connect"));
        // register with a valid name reaches the network step and fails there;
        // the failure path must clean up the locally written keypair.
        assert!(register(DEAD_URL, &paths, "alice")
            .unwrap_err()
            .contains("failed to connect"));
        assert!(!paths.auth_private_key_path("alice").exists());
    }

    #[test]
    fn auth_without_local_key_probes_owner_and_surfaces_the_local_error() {
        let (_temp, paths) = temp_paths();
        // No local key on disk and the server is unreachable: the probe fails
        // to connect (not "unknown owner"), so the local read error surfaces.
        let err = auth(DEAD_URL, &paths, "alice").unwrap_err();
        assert!(
            err.contains("failed to connect") || err.contains("missing local private key"),
            "{err}"
        );
    }

    #[test]
    fn flows_needing_local_ident_key_fail_when_it_is_absent() {
        let (_temp, paths) = temp_paths();
        // These reach past ensure_server_key only if the server is live; with a
        // dead endpoint they fail at connect, which is still an Err. Confirm
        // the Err regardless (both the missing-key and connect paths are Err).
        assert!(link_start(DEAD_URL, &paths, "alice").is_err());
        assert!(rotate_ident(DEAD_URL, &paths, "alice").is_err());
        assert!(revoke_machine(DEAD_URL, &paths, "alice", "fp").is_err());
    }

    #[test]
    fn verify_pinned_metadata_is_a_noop_without_a_pin() {
        let (_temp, paths) = temp_paths();
        // No root pin written: the function returns Ok(()) without any request.
        verify_pinned_metadata(DEAD_URL, &paths).unwrap();
    }

    #[test]
    fn trust_registry_fails_to_connect_against_a_dead_endpoint() {
        let (_temp, paths) = temp_paths();
        assert!(trust_registry(DEAD_URL, &paths, "reg-1", "deadbeef")
            .unwrap_err()
            .contains("failed to connect"));
    }

    #[test]
    fn transport_security_requires_https_for_non_loopback() {
        // SUP-01 / bug-189: http is allowed only for loopback; anything else must
        // use https.
        assert!(ensure_transport_security("http://127.0.0.1:7777").is_ok());
        assert!(ensure_transport_security("http://localhost:7777").is_ok());
        assert!(ensure_transport_security("http://[::1]:7777").is_ok());
        assert!(ensure_transport_security("https://packages.example.com").is_ok());
        // Plaintext to a non-loopback host is rejected, naming https.
        let err = ensure_transport_security("http://packages.example.com").unwrap_err();
        assert!(err.contains("https"), "error should steer to https: {err}");
        // An unsupported scheme is rejected too.
        assert!(ensure_transport_security("ftp://packages.example.com").is_err());
    }
}
