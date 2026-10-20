//! plan-176-L: the registry client's packages code. The module map lives in
//! `repository/src/client/mod.rs`.

use super::*;

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
    fetch_index_with_key(repo_url, &server_key, owner, package)
}

/// [`fetch_index`] against a server key the caller already trusts, with no local
/// state: the route binding, the identKey/fingerprint cross-check and the
/// name-binding signature are all checked under `server_key`.
pub fn fetch_index_with_key(
    repo_url: &str,
    server_key: &[u8],
    owner: &str,
    package: &str,
) -> Result<IndexResponse, String> {
    validate_owner_name(owner)?;
    let ident = format!("{owner}#{package}");
    let response =
        get_json::<IndexResponse>(repo_url, &format!("/index/{}", percent_encode(&ident)))?;
    // bug-581: bind the response to the route BEFORE anything in it is trusted.
    //
    // Every signature check below is self-referential: the name binding is
    // verified over `name_binding_message(response.owner, response.ident_fingerprint)`,
    // i.e. over values the response itself supplies. It therefore proves the
    // registry signed *something*, never that it answered the question that was
    // asked. A registry holding its online server key could return a validly
    // signed binding and version list for a different package entirely and the
    // client would return it as the requested index.
    //
    // The ident is compared exactly because the registry echoes the requested
    // ident verbatim (`server.rs:package_index` returns the raw path parameter),
    // so this is a true route binding and not a guess about normalization.
    if response.ident != ident {
        return Err(format!(
            "registry index is for a different package: requested {ident}, served {}",
            sanitize_server_text(&response.ident)
        ));
    }
    // The owner, by contrast, is compared CASE-FOLDED. The registry resolves
    // owners by `owner_folded` and answers with `owner_display`, so `Alice#pkg`
    // is a legitimate request whose honest answer says `alice`. An exact
    // comparison here would refuse valid input — the failure mode this repo has
    // shipped four times. `fold_owner` is the registry's own rule, not a
    // second, drifting copy of it.
    if fold_owner(&response.owner) != fold_owner(owner) {
        return Err(format!(
            "registry index names a different owner: requested {owner}, served {}",
            sanitize_server_text(&response.owner)
        ));
    }
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
        server_key,
        &crypto::name_binding_message(&response.owner, &response.ident_fingerprint),
        &signature,
    )
    .map_err(|_| {
        "registry name binding does not verify under the registry server key".to_string()
    })?;
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
        let body = read_body_capped(response, MAX_JSON_BYTES, "repository error body")
            .unwrap_or_else(|_| b"repository request failed".to_vec());
        let text = String::from_utf8_lossy(&body);
        if let Ok(error) = serde_json::from_str::<ErrorResponse>(&text) {
            // bug-489: server-authored; sanitize at the trust boundary.
            return Err(sanitize_server_text(&error.error));
        }
        return Err(format!(
            "repository request failed with status {status}: {}",
            sanitize_server_text(&text)
        ));
    }
    let bytes = read_body_capped(response, MAX_BLOB_BYTES, "blob body")?;
    if hex::encode(crypto::sha256(&bytes)) != hash {
        return Err("downloaded blob does not match the requested content hash".to_string());
    }
    Ok(bytes)
}

/// `HEAD /blob/<hash>` (plan-48-B §4.2): probe whether the registry already
/// holds a blob for `hash`. The dedup win on publish — an unchanged vendored
/// library uploads once, ever, across every version that ships it.
pub fn blob_exists(repo_url: &str, hash: &str) -> Result<bool, String> {
    ensure_transport_security(repo_url)?;
    let url = format!("{}/blob/{}", repo_url.trim_end_matches('/'), hash);
    let response = http_client()?
        .head(&url)
        .send()
        .map_err(|err| format!("failed to connect to repository service: {err}"))?;
    let status = response.status();
    if status.is_success() {
        Ok(true)
    } else if status == reqwest::StatusCode::NOT_FOUND {
        Ok(false)
    } else {
        Err(format!(
            "repository HEAD /blob request failed with status {status}"
        ))
    }
}

/// `PUT /blob/<hash>` (plan-48-B §4.2): upload raw native-library bytes,
/// authenticated with the session token in an `Authorization: Bearer` header —
/// the one header-borne credential on this server (plan-48-A §4.3), because a
/// raw-body request cannot carry the body-field `sessionToken`. The server
/// re-hashes the body before storing, so a corrupt upload is rejected there and
/// the client need not verify a response.
pub fn put_blob(
    repo_url: &str,
    hash: &str,
    bytes: Vec<u8>,
    session_token: &str,
) -> Result<(), String> {
    ensure_transport_security(repo_url)?;
    let url = format!("{}/blob/{}", repo_url.trim_end_matches('/'), hash);
    // Blob uploads share the download's generous deadline — a large library
    // needs far more than the control-plane 30s to move (plan-48-B §4.1).
    // bug-490: no documented redirect on this route. The bearer header WOULD be
    // stripped by reqwest on a cross-host hop, but the body — the raw native
    // library bytes — would be replayed to whatever origin the registry names,
    // so this does not follow one either.
    let response = no_redirect_client()?
        .put(&url)
        .bearer_auth(session_token)
        .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
        .timeout(BLOB_TIMEOUT)
        .body(bytes)
        .send()
        .map_err(|err| format!("failed to connect to repository service: {err}"))?;
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }
    let text = response
        .text()
        .unwrap_or_else(|_| "repository request failed".to_string());
    if let Ok(error) = serde_json::from_str::<ErrorResponse>(&text) {
        // bug-489: the server authored this string. Sanitize it HERE, at the
        // trust boundary, not at the terminal — see `sanitize_server_text`.
        return Err(sanitize_server_text(&error.error));
    }
    Err(format!(
        "repository PUT /blob request failed with status {status}: {}",
        sanitize_server_text(&text)
    ))
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

pub(super) fn package_request(
    package: &PackageArtifact<'_>,
    session_token: String,
) -> PackageArtifactRequest {
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
