use crate::crypto;
use crate::local::{self, LocalPaths};
use crate::server::{
    ChallengeRequest, ChallengeResponse, CheckpointResponse, ConsistencyProofResponse,
    ErrorResponse, IdentChainResponse, IndexResponse, InclusionProofResponse, LinkFetchRequest,
    LinkFetchResponse, LinkStartRequest, LinkStartResponse, LogEntry, LoginRequest,
    LoginResponse, PackageArtifactRequest, PublishPackageResponse, RegisterProofs,
    RegisterRequest, RegisterResponse, RevokeChallengeRequest, RevokeRequest, RevokeResponse,
    RotateRequest, RotateResponse, ServerIdentResponse, SigningRequest, SigningResponse,
    ValidatePackageResponse,
};
use crate::validation::validate_owner_name;
use crate::DEFAULT_REPO_URL;
use reqwest::blocking::Client;
use serde::de::DeserializeOwned;

pub fn repo_url_from_env() -> String {
    std::env::var("MFB_REPO_URL").unwrap_or_else(|_| DEFAULT_REPO_URL.to_string())
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

pub fn register(repo_url: &str, paths: &LocalPaths, owner: &str) -> Result<RegisterResponse, String> {
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
pub fn link_start(repo_url: &str, paths: &LocalPaths, owner: &str) -> Result<(String, i64), String> {
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
pub fn rotate_ident(repo_url: &str, paths: &LocalPaths, owner: &str) -> Result<RotateResponse, String> {
    validate_owner_name(owner)?;
    ensure_server_key(repo_url, paths)?;
    let old_private = local::read_ident_private_key(paths, owner)?;
    let old_public = local::read_ident_public_key(paths, owner)?;
    let session_token = local::read_session(paths, owner)?;

    let (new_public, new_private) = crypto::generate_keypair();
    let chain_message = crypto::ident_rotation_message(
        owner,
        &crypto::fingerprint(&old_public),
        &new_public,
    );
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
    let ident = format!("{owner}#{package}");
    let response = get_json::<IndexResponse>(
        repo_url,
        &format!("/index/{}", percent_encode(&ident)),
    )?;
    // The pinned ident is only as trustworthy as the name binding: verify it
    // under the pinned server key and cross-check the fingerprint.
    let ident_public = crypto::decode_bytes(
        response.ident_key.strip_prefix("ed25519:").unwrap_or(&response.ident_key),
        "identKey",
    )?;
    if crypto::fingerprint(&ident_public) != response.ident_fingerprint {
        return Err("registry index identKey does not match its fingerprint".to_string());
    }
    let signature =
        crypto::decode_bytes(&response.name_binding_signature, "nameBindingSignature")?;
    crypto::verify(
        &server_key,
        &crypto::name_binding_message(&response.owner, &response.ident_fingerprint),
        &signature,
    )
    .map_err(|_| {
        "registry name binding does not verify under the pinned server key".to_string()
    })?;
    Ok(response)
}

/// `GET /blob/<hash>` (plan-10-A): download a content-addressed `.mfp` blob and
/// verify its bytes hash to the requested hash before returning them.
pub fn fetch_blob(repo_url: &str, hash: &str) -> Result<Vec<u8>, String> {
    let url = format!("{}/blob/{}", repo_url.trim_end_matches('/'), hash);
    let response = Client::new()
        .get(&url)
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
        return Err(format!("repository request failed with status {status}: {text}"));
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
    let url = format!("{}{}", repo_url.trim_end_matches('/'), path);
    let response = Client::new()
        .post(&url)
        .json(body)
        .send()
        .map_err(|err| format!("failed to connect to repository service: {err}"))?;
    read_json_response(response)
}

fn get_json<T: DeserializeOwned>(repo_url: &str, path: &str) -> Result<T, String> {
    let url = format!("{}{}", repo_url.trim_end_matches('/'), path);
    let response = Client::new()
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
    Err(format!("repository request failed with status {status}: {text}"))
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
