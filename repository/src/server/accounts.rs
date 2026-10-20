//! plan-176-L: the registry server's accounts code. `AppState`, `serve`, the router and the
//! error responses live in `repository/src/server/mod.rs`.

use super::*;

pub(super) async fn server_ident(
    State(state): State<AppState>,
) -> Result<Json<ServerIdentResponse>, (StatusCode, Json<ErrorResponse>)> {
    let public = state.store.server_public_key().map_err(internal)?;
    Ok(Json(ServerIdentResponse {
        server_key: crypto::encode_bytes(&public),
        server_fingerprint: crypto::fingerprint(&public),
    }))
}

pub(super) async fn register(
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

pub(super) async fn challenge(
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

pub(super) async fn rotate_ident(
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

pub(super) async fn ident_chain(
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
pub(super) async fn link_start(
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
pub(super) async fn link_fetch(
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
pub(super) async fn revoke_challenge(
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

pub(super) async fn revoke_machine(
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

pub(super) async fn login(
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

pub(super) fn json_str(value: &str) -> String {
    serde_json::to_string(value).expect("JSON string encoding cannot fail")
}

pub(super) async fn signing(
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
