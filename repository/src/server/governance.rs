//! plan-176-L: the registry server's governance code. `AppState`, `serve`, the router and the
//! error responses live in `repository/src/server/mod.rs`.

use super::*;

/// `POST /orgs/members` (plan-10-D1): grant or remove a member's org role. The
/// grantor must be the org itself (bootstrap) or an owner/admin member, and the
/// change is authorized by the grantor's ident signature and logged.
pub(super) async fn org_members(
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
pub(super) async fn issue_token(
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
pub(super) async fn revoke_token(
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
pub(super) async fn transfer_offer(
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
pub(super) async fn transfer_accept(
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

/// `POST /release-state` (plan-10-C1): a maintainer moves a published version
/// between `available`/`deprecated`/`yanked`. Requires a live session AND an
/// ident signature — an auth session alone is refused.
pub(super) async fn release_state(
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
