//! plan-176-L: the registry server's auth code. `AppState`, `serve`, the router and the
//! error responses live in `repository/src/server/mod.rs`.

use super::*;

/// JWT issuer/audience binding (REPO-04). A session token is minted by this
/// issuer for this registry's session audience, and verification requires the
/// exact pair — so a token minted for a different service or audience (even one
/// signed with the same HS256 secret) cannot be replayed against the session
/// endpoints here.
pub(super) const SESSION_TOKEN_ISSUER: &str = "mfb-repo";

pub(super) const SESSION_TOKEN_AUDIENCE: &str = "mfb-repo/session";

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
pub(super) fn publish_token_scope(
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
pub(super) fn require_package_owner(
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
pub(super) fn session_and_ident(
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

/// Whether a token `scope` (`<owner>#<package>` or `<owner>#*`) belongs to
/// `owner`.
pub(super) fn scope_owner_matches(scope: &str, owner: &str) -> bool {
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
pub(super) fn scope_permits(scope: &str, ident: &str, governing_owner: &str) -> bool {
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
pub(super) fn governing_owner(
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

/// Extract the bearer token from an `Authorization: Bearer <token>` header —
/// the one header-borne credential on this server (plan-48-A §4.3), used by
/// `PUT /blob/<hash>` because a raw-body request cannot carry the body-field
/// `sessionToken` every other authenticated route uses.
pub(super) fn bearer_token(
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
