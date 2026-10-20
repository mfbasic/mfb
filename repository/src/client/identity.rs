//! plan-176-L: the registry client's identity code. The module map lives in
//! `repository/src/client/mod.rs`.

use super::*;

/// Fetch the registry public key from `GET /ident` and pin it as
/// `server.pub` on first contact; a later mismatch is refused (plan-23 index
/// §10.3). Every online flow calls this before touching other routes.
pub fn ensure_server_key(repo_url: &str, paths: &LocalPaths) -> Result<Vec<u8>, String> {
    let server_key = fetch_server_key(repo_url)?;
    local::pin_server_key(paths, &server_key)?;
    Ok(server_key)
}

/// Fetch the registry public key from `GET /ident` and check it against the
/// fingerprint the response names — with no local state: nothing is pinned or
/// compared against a pin. Authenticated by transport only; a caller that holds
/// a signed claim about the key (an attestation's `repoFingerprint`) must compare
/// against that.
pub fn fetch_server_key(repo_url: &str) -> Result<Vec<u8>, String> {
    let response = get_json::<ServerIdentResponse>(repo_url, "/ident")?;
    let server_key = crypto::decode_bytes(&response.server_key, "serverKey")?;
    if crypto::fingerprint(&server_key) != response.server_fingerprint {
        return Err("repository /ident fingerprint does not match its key".to_string());
    }
    Ok(server_key)
}

pub fn register(
    repo_url: &str,
    paths: &LocalPaths,
    owner: &str,
) -> Result<RegisterResponse, String> {
    validate_owner_name(owner)?;
    // Refuse before touching anything if this machine already holds keys for the
    // owner (bug-272). The keypair writes below truncate unconditionally and the
    // error path calls `remove_owner_keys`, so without this guard a `register`
    // against an owner that already exists — the *guaranteed* server error — first
    // overwrote the existing ident private key and then deleted the replacement.
    // Per plan-23 the ident key is the account authority, so on a machine that is
    // the sole holder that sequence permanently locks the account out.
    //
    // Cleaning up after a failed registration for a genuinely *new* owner is still
    // correct and is left alone; the rule is only that `register` must never delete
    // keys it did not create in this call.
    for path in [
        paths.auth_private_key_path(owner),
        paths.ident_private_key_path(owner),
    ] {
        if path.exists() {
            return Err(format!(
                "keys for '{owner}' already exist locally; \
                 use `mfb repo auth` or `mfb repo link`, or remove them first"
            ));
        }
    }
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
    // A rotation staged by a run whose response was lost is settled here too
    // (bug-793). An unresolved one is kept for the next rotate or auth; logging in
    // does not depend on it.
    recover_pending_ident(repo_url, paths, owner)?;
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
    // bug-583: publish the PUBLIC half of the code-derived approval keypair as
    // the verifier for the fetch half. The code itself never leaves this
    // machine, so the relay cannot derive the private half and cannot approve
    // an auth key of its own.
    let (approval_public, _approval_private) = crypto::pairing_approval_keypair(&code)?;
    let response = post_json::<LinkStartResponse>(
        repo_url,
        "/machines/link",
        &LinkStartRequest {
            owner: owner.to_string(),
            lookup: crypto::pairing_lookup(&code),
            blob: crypto::encode_bytes(&blob),
            salt: crypto::encode_bytes(&salt),
            approval_key: crypto::encode_bytes(&approval_public),
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
    // bug-583: prove possession of the typed code by signing THIS auth key
    // with the code-derived approval private key. The lookup only names the
    // pending pairing; it is not authorization.
    let lookup = crypto::pairing_lookup(code.trim());
    let (_approval_public, approval_private) = crypto::pairing_approval_keypair(code.trim())?;
    let approval = crypto::sign(
        &approval_private,
        &crypto::pairing_approval_message(&lookup, &auth_public),
    )?;
    let response = post_json::<LinkFetchResponse>(
        repo_url,
        "/machines/link/fetch",
        &LinkFetchRequest {
            owner: owner.to_string(),
            lookup,
            auth_key: crypto::encode_bytes(&auth_public),
            proof: crypto::encode_bytes(&proof),
            approval: crypto::encode_bytes(&approval),
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
    // A rotation staged by an earlier run may already be the account authority:
    // its request can have committed with the response lost (bug-793). Settle it
    // before anything here signs with the live key or writes a new staging, since
    // overwriting it would destroy the only copy of the key the registry trusts.
    match recover_pending_ident(repo_url, paths, owner)? {
        PendingIdent::None | PendingIdent::Discarded => {}
        PendingIdent::Promoted(fingerprint) => {
            return Ok(RotateResponse {
                owner: owner.to_string(),
                ident_fingerprint: fingerprint,
            })
        }
        PendingIdent::Unresolved(reason) => {
            return Err(format!(
                "{reason}; a staged ident key was left at '{}' — re-run `mfb key \
                 rotate {owner}` once the registry can confirm whether it committed",
                paths.ident_pending_private_key_path(owner).display()
            ))
        }
    }
    let old_private = local::read_ident_private_key(paths, owner)?;
    let old_public = local::read_ident_public_key(paths, owner)?;
    let session_token = local::read_session(paths, owner)?;

    let (new_public, new_private) = crypto::generate_keypair();
    let chain_message =
        crypto::ident_rotation_message(owner, &crypto::fingerprint(&old_public), &new_public);
    let chain_signature = crypto::sign(&old_private, &chain_message)?;
    let possession_message = crypto::registration_message(crypto::ROLE_IDENT, owner, &new_public);
    let possession_proof = crypto::sign(&new_private, &possession_message)?;

    // Stage the new keypair *before* asking the server to commit the rotation
    // (bug-276 R1). The server marks the old ident `past` inside its transaction,
    // so if the new private key only existed in memory until after a successful
    // response, then a lost response or a failing disk write would leave this
    // machine holding a key that is no longer the account authority and no copy
    // of the one that is. Rotation exists precisely because other machines are
    // lost or untrusted, so that can brick the account.
    local::write_pending_ident_keypair(paths, owner, &new_public, &new_private)?;
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
    );
    match response {
        Ok(response) => {
            local::promote_pending_ident_keypair(paths, owner)?;
            Ok(response)
        }
        Err(err) => {
            // The request failed, but "failed" includes a response lost after the
            // server already committed — the case that actually strands the
            // account. Ask the registry which ident it now holds.
            match recover_pending_ident(repo_url, paths, owner)? {
                PendingIdent::Promoted(fingerprint) => Ok(RotateResponse {
                    owner: owner.to_string(),
                    ident_fingerprint: fingerprint,
                }),
                // Server still on the old ident: the rotation did not happen.
                PendingIdent::None | PendingIdent::Discarded => Err(err),
                // Leave the staged key in place — it is the only copy of a
                // possibly-authoritative key; a later rotate or auth settles it.
                PendingIdent::Unresolved(reason) => Err(format!(
                    "{err}; {reason}; a staged ident key was left at '{}' — re-run \
                     `mfb key rotate {owner}` once the registry can confirm whether it \
                     committed",
                    paths.ident_pending_private_key_path(owner).display()
                )),
            }
        }
    }
}

/// What [`recover_pending_ident`] decided about a staged ident rotation.
#[derive(Debug, PartialEq, Eq)]
enum PendingIdent {
    /// Nothing was staged.
    None,
    /// The staged key is the registry's current ident: the rotation committed,
    /// and the staging is now the live keypair. Carries its fingerprint.
    Promoted(String),
    /// The registry still holds this machine's live ident: the rotation never
    /// committed, so the staging was discarded.
    Discarded,
    /// The registry could not be consulted, or holds a key that is neither the
    /// staged nor the live one. The staging is kept untouched.
    Unresolved(String),
}

/// Settle a staged ident rotation by asking the registry which ident key it
/// currently holds (bug-276 R1, bug-793).
///
/// Only a registry that demonstrably holds the staged key promotes it, and only
/// one that demonstrably still holds the live key discards it. Every other state
/// keeps the staging: deleting a key that might be the account authority is the
/// one outcome worth avoiding. `Err` is reserved for a local file failure.
fn recover_pending_ident(
    repo_url: &str,
    paths: &LocalPaths,
    owner: &str,
) -> Result<PendingIdent, String> {
    let Some(staged_private) = local::read_pending_ident_private_key(paths, owner)? else {
        return Ok(PendingIdent::None);
    };
    let staged_public = crypto::public_from_private(&staged_private)?;
    let current = match fetch_ident_chain(repo_url, owner)
        .and_then(|chain| crypto::decode_bytes(&chain.ident_key, "identKey"))
    {
        Ok(current) => current,
        Err(err) => {
            return Ok(PendingIdent::Unresolved(format!(
                "the registry could not be asked whether a staged ident rotation \
                 committed: {err}"
            )))
        }
    };
    if current == staged_public {
        local::promote_pending_ident_keypair(paths, owner)?;
        return Ok(PendingIdent::Promoted(crypto::fingerprint(&staged_public)));
    }
    if local::read_ident_public_key(paths, owner).ok().as_deref() == Some(current.as_slice()) {
        local::remove_pending_ident_keypair(paths, owner);
        return Ok(PendingIdent::Discarded);
    }
    Ok(PendingIdent::Unresolved(format!(
        "the registry's ident for '{owner}' ({}) is neither the staged key nor this \
         machine's live key",
        crypto::fingerprint(&current)
    )))
}

/// Fetch the owner's current ident binding and the signed rotation chain.
pub fn fetch_ident_chain(repo_url: &str, owner: &str) -> Result<IdentChainResponse, String> {
    validate_owner_name(owner)?;
    get_json::<IdentChainResponse>(repo_url, &format!("/idents/{owner}"))
}

/// The owners' rotation chains as plan-168-C's verifier (`crate::trust`) takes
/// them, read from `GET /idents/<owner>` on `repo_url`. The chain needs no
/// server signature: each link is signed by the key it retires.
pub fn registry_chains(
    repo_url: &str,
) -> impl Fn(&str) -> Result<Vec<crate::server::IdentChainLink>, String> + '_ {
    move |owner: &str| fetch_ident_chain(repo_url, owner).map(|response| response.chain)
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
    let mut path = chain_path(owner, pinned, chain)?;
    Ok(if path.len() > 1 { path.pop() } else { None })
}

/// Every key on `owner`'s signed rotation chain from `start` forward, `start`
/// first (plan-168-C): each successor is accepted only through a link signed by
/// the key it retires, over the owner-bound rotation message. The chain is read
/// once, oldest link first, so the walk is bounded by its length. A key `K` is
/// "forward-reachable" from `start` exactly when it is on this path. A `start`
/// the chain never names yields just `[start]`.
pub fn chain_path(
    owner: &str,
    start: &[u8],
    chain: &[crate::server::IdentChainLink],
) -> Result<Vec<Vec<u8>>, String> {
    let mut path = vec![start.to_vec()];
    for link in chain {
        let old_key = crypto::decode_bytes(&link.old_key, "oldKey")?;
        if Some(&old_key) != path.last() {
            continue;
        }
        let new_key = crypto::decode_bytes(&link.new_key, "newKey")?;
        let signature = crypto::decode_bytes(&link.signature, "signature")?;
        let message =
            crypto::ident_rotation_message(owner, &crypto::fingerprint(&old_key), &new_key);
        crypto::verify(&old_key, &message, &signature)
            .map_err(|_| "invalid ident chain link signature".to_string())?;
        path.push(new_key);
    }
    Ok(path)
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
