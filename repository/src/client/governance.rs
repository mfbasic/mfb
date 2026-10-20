//! plan-176-L: the registry client's governance code. The module map lives in
//! `repository/src/client/mod.rs`.

use super::*;

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

/// Verify a server-signed name binding (plan-168-A) under `server_key` and
/// return the bound ident public key: the key must hash to the bound
/// fingerprint and the signature must cover `name_binding_message(owner,
/// fingerprint)`.
pub fn verify_owner_binding(server_key: &[u8], binding: &OwnerBinding) -> Result<Vec<u8>, String> {
    let public = crypto::decode_bytes(
        binding
            .ident_key
            .strip_prefix("ed25519:")
            .unwrap_or(&binding.ident_key),
        "identKey",
    )?;
    if crypto::fingerprint(&public) != binding.ident_fingerprint {
        return Err(format!(
            "registry binding for {} names a key that does not match its fingerprint",
            sanitize_server_text(&binding.owner)
        ));
    }
    let signature = crypto::decode_bytes(&binding.name_binding_signature, "nameBindingSignature")?;
    crypto::verify(
        server_key,
        &crypto::name_binding_message(&binding.owner, &binding.ident_fingerprint),
        &signature,
    )
    .map_err(|_| {
        format!(
            "registry binding for {} does not verify under the registry server key",
            sanitize_server_text(&binding.owner)
        )
    })?;
    Ok(public)
}

/// The package's current owner as a verified `/index` states it (plan-168-A
/// §4.5): the last transfer's `toBinding`, or the index's own binding for a
/// package that never moved. Returns `(owner display, ident fingerprint)`.
fn current_owner_binding(
    server_key: &[u8],
    index: &IndexResponse,
) -> Result<(String, String), String> {
    match index.transfers.last() {
        Some(link) => {
            verify_owner_binding(server_key, &link.to_binding)?;
            Ok((
                link.to_binding.owner.clone(),
                link.to_binding.ident_fingerprint.clone(),
            ))
        }
        None => Ok((index.owner.clone(), index.ident_fingerprint.clone())),
    }
}

/// The v2 transfer terms the next link of an ident signs (plan-168-A §4.5),
/// computed from a verified `/index` exactly as the registry computes them
/// from its tables: the sequence is the number of served links and the digest
/// covers every served version.
struct ClientTransferTerms {
    current_owner: String,
    current_fingerprint: String,
    sequence: i64,
    release_digest: String,
    to_owner: String,
}

fn client_transfer_terms(
    repo_url: &str,
    paths: &LocalPaths,
    ident: &str,
    to_owner: &str,
) -> Result<ClientTransferTerms, String> {
    let Some((owner, package)) = ident.split_once('#') else {
        return Err("ident must use <owner>#<package>".to_string());
    };
    validate_owner_name(to_owner)?;
    let server_key = ensure_server_key(repo_url, paths)?;
    verify_pinned_metadata(repo_url, paths)?;
    let index = fetch_index_with_key(repo_url, &server_key, owner, package)?;
    let (current_owner, current_fingerprint) = current_owner_binding(&server_key, &index)?;
    // The signed bytes carry the recipient's REGISTERED display name, which is
    // what the registry records and `/index` later serves.
    let recipient = fetch_ident_chain(repo_url, to_owner)?.owner;
    Ok(ClientTransferTerms {
        current_owner,
        current_fingerprint,
        sequence: i64::try_from(index.transfers.len())
            .map_err(|_| "registry index serves too many transfers".to_string())?,
        release_digest: crypto::release_digest(
            index
                .versions
                .iter()
                .map(|version| (version.version.as_str(), version.hash.as_str())),
        ),
        to_owner: recipient,
    })
}

/// `POST /packages/transfer/offer` (plan-10-D1, v2 since plan-168-A): the
/// package's CURRENT owner — the last transfer's recipient, or the ident
/// prefix owner for a package that never moved — offers it to a recipient,
/// signing the v2 terms read from `/index` with the local ident key.
pub fn transfer_offer(
    repo_url: &str,
    paths: &LocalPaths,
    ident: &str,
    to_owner: &str,
) -> Result<TransferResponse, String> {
    let terms = client_transfer_terms(repo_url, paths, ident, to_owner)?;
    validate_owner_name(&terms.current_owner)?;
    let ident_private = local::read_ident_private_key(paths, &terms.current_owner)?;
    if crypto::fingerprint(&crypto::public_from_private(&ident_private)?)
        != terms.current_fingerprint
    {
        return Err(format!(
            "this machine does not hold {}'s current ident",
            terms.current_owner
        ));
    }
    let session_token = local::read_session(paths, &terms.current_owner)?;
    let signature = crypto::sign(
        &ident_private,
        &crypto::transfer_offer_message_v2(crypto::TransferTermsRef {
            ident,
            sequence: terms.sequence,
            from_owner: &terms.current_owner,
            from_ident_fingerprint: &terms.current_fingerprint,
            to_owner: &terms.to_owner,
            release_digest: &terms.release_digest,
        }),
    )?;
    post_json::<TransferResponse>(
        repo_url,
        "/packages/transfer/offer",
        &TransferOfferRequest {
            ident: ident.to_string(),
            from_owner: terms.current_owner,
            to_owner: terms.to_owner,
            sequence: terms.sequence,
            release_digest: terms.release_digest,
            session_token,
            ident_signature: crypto::encode_bytes(&signature),
        },
    )
}

/// `POST /packages/transfer/accept` (plan-10-D1, v2 since plan-168-A): the
/// recipient accepts a pending offer by signing the same v2 terms, read from
/// `/index`, with the local ident key.
pub fn transfer_accept(
    repo_url: &str,
    paths: &LocalPaths,
    ident: &str,
    to_owner: &str,
) -> Result<TransferResponse, String> {
    let terms = client_transfer_terms(repo_url, paths, ident, to_owner)?;
    let ident_private = local::read_ident_private_key(paths, to_owner)?;
    let session_token = local::read_session(paths, to_owner)?;
    let signature = crypto::sign(
        &ident_private,
        &crypto::transfer_accept_message_v2(crypto::TransferTermsRef {
            ident,
            sequence: terms.sequence,
            from_owner: &terms.current_owner,
            from_ident_fingerprint: &terms.current_fingerprint,
            to_owner: &terms.to_owner,
            release_digest: &terms.release_digest,
        }),
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
