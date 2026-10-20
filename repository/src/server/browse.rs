//! plan-176-L: the registry server's browse code. `AppState`, `serve`, the router and the
//! error responses live in `repository/src/server/mod.rs`.

use super::*;

/// Clamp a description to the search-result preview length, appending an
/// ellipsis when it was actually shortened.
fn description_preview(description: Option<String>) -> Option<String> {
    description.map(|text| {
        if text.chars().count() <= SEARCH_DESCRIPTION_PREVIEW_CHARS {
            return text;
        }
        let mut preview: String = text
            .chars()
            .take(SEARCH_DESCRIPTION_PREVIEW_CHARS)
            .collect();
        preview.push('…');
        preview
    })
}

#[derive(Debug, Deserialize)]
pub(super) struct SearchQuery {
    q: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
}

/// `GET /search?q=&limit=&offset=` — anonymous, read-only (plan-61-B Phase 2).
///
/// Reads no credential. `limit` is clamped to `SEARCH_LIMIT_MAX` server-side,
/// and an empty or whitespace-only `q` returns an empty result set rather than
/// the whole table.
pub(super) async fn search(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    axum::extract::Query(query): axum::extract::Query<SearchQuery>,
) -> Result<Json<SearchResponse>, (StatusCode, Json<ErrorResponse>)> {
    if !state
        .rate_limiter
        .allow(&format!("search:{}", peer.ip()), SEARCH_PER_IP_MAX, 60)
    {
        return Err(too_many_requests());
    }
    let text = query.q.unwrap_or_default();
    // Clamp rather than reject: an over-cap `limit` is a client that wants more
    // than it may have, not an error, and rejecting it would make paging
    // brittle for no security gain.
    let limit = query
        .limit
        .unwrap_or(SEARCH_LIMIT_DEFAULT)
        .clamp(0, SEARCH_LIMIT_MAX);
    let offset = query.offset.unwrap_or(0).max(0);

    let results: Vec<SearchResult> = state
        .store
        .search_packages(&text, limit, offset)
        .map_err(internal)?
        .into_iter()
        .map(|row| SearchResult {
            ident: row.ident,
            owner: row.owner,
            latest_version: row.latest_version,
            latest_state: row.latest_state,
            description: description_preview(row.description),
            published_at: row.published_at,
        })
        .collect();

    Ok(Json(SearchResponse {
        query: text,
        total: results.len(),
        results,
    }))
}

/// The registry id and root fingerprint the HTML pages display.
///
/// Both are `None` before `mfb-repo init-root` runs, and the landing page
/// simply omits the fingerprint block rather than showing a placeholder — a
/// blank or fabricated fingerprint in the one section whose entire purpose is
/// "compare this exactly" is worse than no section at all.
fn registry_identity(state: &AppState) -> (String, Option<String>) {
    match state.store.registry_config() {
        Ok(Some(config)) => (
            config.registry_id,
            Some(crypto::fingerprint(&config.root_public)),
        ),
        _ => ("(uninitialized)".to_string(), None),
    }
}

/// `GET /style.css` — the stylesheet, compiled into the binary.
///
/// A real route rather than an inline `<style>` block because the CSP is
/// `style-src 'self'` with no `'unsafe-inline'`; and `include_str!` rather than
/// a `ServeDir` so the server stays a single self-contained binary.
pub(super) async fn stylesheet() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=3600"),
        ],
        crate::web::STYLESHEET,
    )
}

/// `GET /` — the landing page (plan-61-C Phase 2).
pub(super) async fn landing_page(State(state): State<AppState>) -> Response {
    let (registry_id, root_fingerprint) = registry_identity(&state);
    crate::web::html_response(
        StatusCode::OK,
        crate::web::landing(&registry_id, root_fingerprint.as_deref()),
    )
}

/// `GET /search.html?q=` — the rendered search page (plan-61-C Phase 2).
///
/// Shares `search_packages` with the JSON route, including its server-side
/// `limit` cap, so the HTML surface cannot be used to enumerate more than the
/// API allows.
pub(super) async fn search_html(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    axum::extract::Query(query): axum::extract::Query<SearchQuery>,
) -> Response {
    let (registry_id, _fingerprint) = registry_identity(&state);
    if !state
        .rate_limiter
        .allow(&format!("search:{}", peer.ip()), SEARCH_PER_IP_MAX, 60)
    {
        return crate::web::html_response(
            StatusCode::TOO_MANY_REQUESTS,
            crate::web::message_page(
                &registry_id,
                "Too many searches",
                "This client has made too many searches in a short window. Wait a \
                 minute and try again.",
            ),
        );
    }
    let text = query.q.unwrap_or_default();
    let limit = query
        .limit
        .unwrap_or(SEARCH_LIMIT_DEFAULT)
        .clamp(0, SEARCH_LIMIT_MAX);
    let offset = query.offset.unwrap_or(0).max(0);

    let rows = match state.store.search_packages(&text, limit, offset) {
        Ok(rows) => rows,
        Err(_err) => {
            return crate::web::html_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                crate::web::message_page(
                    &registry_id,
                    "Search failed",
                    "The registry could not complete this search.",
                ),
            )
        }
    };
    let rows: Vec<crate::web::SearchRow> = rows
        .into_iter()
        .map(|row| crate::web::SearchRow {
            ident: row.ident,
            owner: row.owner,
            latest_version: row.latest_version,
            latest_state: row.latest_state,
            description: description_preview(row.description),
            published_at: row.published_at,
        })
        .collect();

    // A query that matches nothing is 200 with a "no results" page: the request
    // succeeded and the answer is "none", which is not the same as "that page
    // does not exist".
    crate::web::html_response(
        StatusCode::OK,
        crate::web::search_page(&registry_id, &text, &rows),
    )
}

/// `GET /p/:ident/docs` — the Docs tab (plan-126-F).
///
/// Resolves the package through `package_detail` first, so an unknown ident gets
/// the identical not-found page and status the Overview and Audit tabs give. It
/// then reads the documentation of the **latest active** release
/// (`Store::latest_active_version_docs`, built on plan-126-A's selection) — the
/// same release `detail.latest_version` names, so the version shown is the
/// version documented.
///
/// The stored bytes were decode-validated when they were recorded, so a decode
/// failure here means the database no longer matches what was written. That is
/// reported as an explicit error page, never a panic, and never rendered as
/// "no documentation", which would hide it.
pub(super) async fn package_docs_html(
    State(state): State<AppState>,
    axum::extract::Path(ident): axum::extract::Path<String>,
) -> Response {
    let (registry_id, _fingerprint) = registry_identity(&state);
    let view = match lookup_package_docs(&state, ident).await {
        Ok(view) => view,
        Err(DocsLookupError::Package((status, Json(error)))) => {
            return crate::web::html_response(
                status,
                crate::web::message_page(&registry_id, "Package not found", &error.error),
            )
        }
        Err(DocsLookupError::Unavailable(message)) => {
            return crate::web::html_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                crate::web::message_page(&registry_id, "Documentation unavailable", message),
            )
        }
    };
    crate::web::html_response(StatusCode::OK, crate::web::docs_page(&registry_id, &view))
}

/// Why the Docs tab's lookup produced no view.
enum DocsLookupError {
    /// `package_detail` refused the ident (malformed, or no such package) —
    /// passed through unchanged so every surface reports it identically.
    Package((StatusCode, Json<ErrorResponse>)),
    /// The package exists but its stored documentation could not be read.
    Unavailable(&'static str),
}

/// The one lookup behind both `GET /p/:ident/docs` and `GET /packages/:ident/docs`,
/// so the HTML page and its JSON mirror cannot disagree about which release is
/// documented or what its documentation says.
///
/// Resolves the package through `package_detail` (same 400/404 as the other
/// package routes), then reads the **latest active** release's stored section
/// 17 (`Store::latest_active_version_docs`, plan-126-A's selection) — the release
/// `detail.latest_version` names, so the version reported is the one documented.
async fn lookup_package_docs(
    state: &AppState,
    ident: String,
) -> Result<crate::web::DocsView, DocsLookupError> {
    let Json(detail) = package_detail(State(state.clone()), axum::extract::Path(ident))
        .await
        .map_err(DocsLookupError::Package)?;

    let fallback_name = detail
        .ident
        .split_once('#')
        .map(|(_, package)| package.to_string())
        .unwrap_or_else(|| detail.ident.clone());
    let page = match state.store.latest_active_version_docs(&detail.ident) {
        Ok(Some((_version, section))) => match mfb_wire::docs::read_doc_table(&section) {
            Ok(docs) => Some(mfb_wire::docpage::from_package(docs, &fallback_name)),
            Err(_) => {
                return Err(DocsLookupError::Unavailable(
                    "The stored documentation for this release could not be decoded.",
                ))
            }
        },
        Ok(None) => None,
        Err(_) => {
            return Err(DocsLookupError::Unavailable(
                "The registry could not load this package's documentation.",
            ))
        }
    };

    Ok(crate::web::DocsView {
        ident: detail.ident,
        version: detail.latest_version,
        page,
    })
}

/// `GET /packages/:ident/docs` — the Docs tab as JSON (plan-126-F Phase 3).
///
/// Anonymous and read-only, like `GET /packages/:ident`. Serializes the same
/// `DocPage` model the HTML tab renders — not the raw wire structures — so the
/// two surfaces agree on grouping, order and anchors by construction.
/// `documentation` is `null` exactly when the HTML tab shows the
/// no-documentation statement.
pub(super) async fn package_docs(
    State(state): State<AppState>,
    axum::extract::Path(ident): axum::extract::Path<String>,
) -> Result<Json<PackageDocsResponse>, (StatusCode, Json<ErrorResponse>)> {
    match lookup_package_docs(&state, ident).await {
        Ok(view) => Ok(Json(PackageDocsResponse::from_view(view))),
        Err(DocsLookupError::Package(error)) => Err(error),
        Err(DocsLookupError::Unavailable(message)) => Err(internal(message.to_string())),
    }
}

/// `GET /p/:ident` — the rendered package page (plan-61-C Phase 3).
///
/// Renders the *same* `package_detail` handler output the JSON route serves, so
/// the two surfaces cannot disagree about what a package's history is — which
/// is the whole point of a transparency view.
pub(super) async fn package_page_html(
    State(state): State<AppState>,
    axum::extract::Path(ident): axum::extract::Path<String>,
) -> Response {
    let (registry_id, _fingerprint) = registry_identity(&state);
    let detail =
        match package_detail(State(state.clone()), axum::extract::Path(ident.clone())).await {
            Ok(Json(detail)) => detail,
            Err((status, Json(error))) => {
                return crate::web::html_response(
                    status,
                    crate::web::message_page(&registry_id, "Package not found", &error.error),
                )
            }
        };

    let view = crate::web::PackageView {
        ident: detail.ident,
        owner: detail.owner,
        ident_key: detail.ident_key,
        ident_fingerprint: detail.ident_fingerprint,
        server_fingerprint: detail.server_fingerprint,
        author: detail.author,
        url: detail.url,
        description: detail.description,
        latest_version: detail.latest_version,
        latest_state: detail.latest_state,
        versions: detail
            .versions
            .into_iter()
            .map(|version| crate::web::VersionRow {
                version: version.version,
                hash: version.hash,
                published_at: version.published_at,
                state: version.state,
                abi_symbols: version
                    .abi_index
                    .as_object()
                    .map(|map| map.len())
                    .unwrap_or(0),
                log_index: version.log_entry.map(|entry| entry.index),
                targets: version
                    .targets
                    .into_iter()
                    .map(|target| crate::web::TargetRow {
                        os: target.os,
                        arch: target.arch,
                        libc: target.libc,
                        lib_type: target.lib_type,
                        logical: target.logical,
                        source: target.source,
                        blob_hash: target.blob_hash,
                    })
                    .collect(),
            })
            .collect(),
    };
    crate::web::html_response(
        StatusCode::OK,
        crate::web::package_page(&registry_id, &view),
    )
}

/// `GET /p/:ident/audit` — the rendered transparency tab (plan-61-C Phase 3).
pub(super) async fn package_audit_html(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    axum::extract::Path(ident): axum::extract::Path<String>,
) -> Response {
    let (registry_id, _fingerprint) = registry_identity(&state);
    // Shares the JSON route's budget on purpose: the HTML page does the same
    // full-log work, so exempting it would leave the bypass open.
    let audit = match package_audit(
        State(state.clone()),
        ConnectInfo(peer),
        axum::extract::Path(ident.clone()),
    )
    .await
    {
        Ok(Json(audit)) => audit,
        Err((status, Json(error))) => {
            return crate::web::html_response(
                status,
                crate::web::message_page(&registry_id, "Package not found", &error.error),
            )
        }
    };

    let view = crate::web::AuditView {
        ident: audit.ident,
        checkpoint_size: audit.log_checkpoint.size,
        checkpoint_root: audit.log_checkpoint.root_hash,
        checkpoint_signature: audit.log_checkpoint.signature,
        publishes: audit
            .publishes
            .into_iter()
            .map(|entry| crate::web::AuditPublish {
                version: entry.version,
                index: entry.index,
                leaf_hash: entry.leaf_hash,
                proof: entry.proof,
            })
            .collect(),
        state_changes: audit
            .state_changes
            .into_iter()
            .map(|change| (change.version, change.state, change.at))
            .collect(),
        ident_chain: audit
            .ident_chain
            .into_iter()
            .map(|rotation| {
                (
                    rotation.old_key,
                    rotation.new_key,
                    rotation.signature,
                    rotation.issued,
                )
            })
            .collect(),
    };
    crate::web::html_response(StatusCode::OK, crate::web::audit_page(&registry_id, &view))
}

/// Split `<owner>#<package>`, or a 400 naming the expected shape.
///
/// axum percent-decodes a `Path` segment before the handler sees it, so a
/// browser-safe `%23` arrives here as a literal `#`. `package_index` has always
/// relied on that; these routes mirror it rather than re-deriving it.
fn split_ident(ident: &str) -> Result<(&str, &str), (StatusCode, Json<ErrorResponse>)> {
    match ident.split_once('#') {
        Some((owner, package)) if !owner.is_empty() && !package.is_empty() => Ok((owner, package)),
        _ => Err(bad_request("ident must use <owner>#<package>".to_string())),
    }
}

/// `GET /packages/:ident` — anonymous, read-only (plan-61-B Phase 1).
///
/// Reads **no** credential: no `sessionToken`, no `Authorization` header, no
/// cookie. A request carrying one behaves identically to one that does not,
/// which is asserted by test rather than left as a property of the signature.
pub(super) async fn package_detail(
    State(state): State<AppState>,
    axum::extract::Path(ident): axum::extract::Path<String>,
) -> Result<Json<PackageDetailResponse>, (StatusCode, Json<ErrorResponse>)> {
    split_ident(&ident)?;
    let Some(detail) = state.store.package_detail(&ident).map_err(internal)? else {
        // One 404 for "no such package", whatever the reason. Distinguishing
        // "unknown owner" from "unknown package under a known owner" would
        // turn this route into an owner-enumeration oracle.
        return Err(not_found("unknown package".to_string()));
    };
    // The key shown is the STORED owner's (plan-168-A), the account
    // `detail.owner` names. After a transfer that is no longer the ident
    // prefix, and pairing it with the prefix owner's key described an account
    // that does not exist.
    let Some((owner_record, ident_key)) = state
        .store
        .owner_with_ident_key(&detail.owner)
        .map_err(internal)?
    else {
        return Err(not_found("unknown package".to_string()));
    };

    // The package-level metadata shown is the **newest** version's. Versions
    // are ordered newest-first, and an older version's author/url describes
    // that older artifact, not the package as it stands today. Read before the
    // loop below consumes the rows.
    let (author, url, description) = match detail.versions.first() {
        Some(newest) => (
            newest.author.clone(),
            newest.url.clone(),
            newest.description.clone(),
        ),
        None => (None, None, None),
    };

    let mut versions = Vec::new();
    for row in detail.versions {
        let log_entry = state
            .store
            .publish_log_entry(&ident, &row.version)
            .map_err(internal)?
            .map(|entry| LogEntry {
                index: entry.index,
                leaf_hash: hex::encode(entry.leaf_hash),
            });
        versions.push(PackageDetailVersionResponse {
            version: row.version,
            hash: row.hash,
            published_at: row.published_at,
            state: row.state,
            abi_index: serde_json::from_str(&row.abi_index)
                .unwrap_or_else(|_| serde_json::json!({})),
            log_entry,
            targets: row
                .targets
                .into_iter()
                .map(|target| PackageTargetResponse {
                    os: target.os,
                    arch: target.arch,
                    libc: target.libc,
                    lib_type: target.lib_type,
                    logical: target.logical,
                    source: target.source,
                    blob_hash: target.blob_hash,
                })
                .collect(),
        });
    }

    let (server_public, _server_private) = state.store.server_keypair().map_err(internal)?;
    // plan-126-A: the headline version is the newest **active** release, not
    // simply the newest row. `versions.first()` had no state predicate, so a
    // package whose newest release was yanked advertised that release here — and
    // API consumers read this field as "the version to use". `versions` itself
    // stays complete and unfiltered above; only this selection narrows.
    let latest = state
        .store
        .latest_active_version(&ident)
        .map_err(internal)?;
    let (latest_version, latest_state) = match latest {
        Some((version, release_state)) => (Some(version), Some(release_state)),
        None => (None, None),
    };
    Ok(Json(PackageDetailResponse {
        latest_version,
        latest_state,
        ident,
        owner: owner_record.owner_display,
        ident_key: format!("ed25519:{}", crypto::encode_bytes(&ident_key.public_key)),
        ident_fingerprint: ident_key.fingerprint,
        server_fingerprint: crypto::fingerprint(&server_public),
        author,
        url,
        description,
        versions,
    }))
}

/// `GET /packages/:ident/audit` — anonymous, read-only (plan-61-B §4).
///
/// Every publish entry carries its inclusion proof against the checkpoint
/// returned in the same response, so a third-party monitor can verify the
/// history rather than take the registry's word for it.
pub(super) async fn package_audit(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    axum::extract::Path(ident): axum::extract::Path<String>,
) -> Result<Json<PackageAuditResponse>, (StatusCode, Json<ErrorResponse>)> {
    if !state
        .rate_limiter
        .allow(&format!("log:{}", peer.ip()), LOG_PER_IP_MAX, 60)
    {
        return Err(too_many_requests());
    }
    let (owner_part, _package_part) = split_ident(&ident)?;
    let Some(audit) = state.store.package_audit(&ident).map_err(internal)? else {
        return Err(not_found("unknown package".to_string()));
    };

    // Deliberately NOT `signed_checkpoint()`: this route needs the leaf vector
    // itself to build an inclusion path per publish, so the memo would save
    // nothing and would introduce a race — a head memoised at one size beside
    // paths built from a differently-sized leaf set. Computing both from ONE
    // read keeps the response internally coherent, which is the property the
    // bug's non-goals protect. It is the per-IP budget above that bounds this
    // route, not the cache.
    let leaves = state.store.log_leaf_hashes(None).map_err(internal)?;
    let root = crate::log::root(&leaves);
    let (_public, private) = state.store.server_keypair().map_err(internal)?;
    let signature = crypto::sign(
        &private,
        &crate::log::checkpoint_signing_input(leaves.len() as u64, &root),
    )
    .map_err(internal)?;

    let mut publishes = Vec::new();
    for version in audit.versions {
        let Some(entry) = state
            .store
            .publish_log_entry(&ident, &version)
            .map_err(internal)?
        else {
            continue;
        };
        // A proof is only meaningful against the checkpoint in this same
        // response, so both are computed from the one `leaves` snapshot.
        let proof = if entry.index >= 0 && (entry.index as usize) < leaves.len() {
            crate::log::inclusion_path(entry.index as usize, &leaves)
                .into_iter()
                .map(hex::encode)
                .collect()
        } else {
            Vec::new()
        };
        publishes.push(AuditPublishEntry {
            version,
            index: entry.index,
            leaf_hash: hex::encode(entry.leaf_hash),
            proof,
        });
    }

    let ident_chain = state
        .store
        .ident_chain(owner_part)
        .map_err(internal)?
        .into_iter()
        .map(|(old_key, new_key, signature, issued)| AuditIdentRotation {
            old_key: format!("ed25519:{}", crypto::encode_bytes(&old_key)),
            new_key: format!("ed25519:{}", crypto::encode_bytes(&new_key)),
            signature: crypto::encode_bytes(&signature),
            issued,
        })
        .collect();

    Ok(Json(PackageAuditResponse {
        ident,
        owner: audit.owner,
        log_checkpoint: CheckpointResponse {
            size: leaves.len() as i64,
            root_hash: hex::encode(root),
            signature: crypto::encode_bytes(&signature),
        },
        publishes,
        state_changes: audit
            .state_changes
            .into_iter()
            .map(|change| AuditStateChange {
                version: change.version,
                state: change.state,
                at: change.at,
            })
            .collect(),
        ident_chain,
    }))
}

/// `GET /index/<owner>#<package>` (plan-10-A): serve the published version
/// list, the owner's current ident key, and a server-signed name binding.
pub(super) async fn package_index(
    State(state): State<AppState>,
    axum::extract::Path(ident): axum::extract::Path<String>,
) -> Result<Json<IndexResponse>, (StatusCode, Json<ErrorResponse>)> {
    let Some((owner_part, package_part)) = ident.split_once('#') else {
        return Err(bad_request("ident must use <owner>#<package>".to_string()));
    };
    if owner_part.is_empty() || package_part.is_empty() {
        return Err(bad_request("ident must use <owner>#<package>".to_string()));
    }
    let Some((owner_record, ident_key)) = state
        .store
        .owner_with_ident_key(owner_part)
        .map_err(internal)?
    else {
        return Err(bad_request("unknown owner".to_string()));
    };
    let mut versions = Vec::new();
    for row in state
        .store
        .list_package_versions(&ident)
        .map_err(internal)?
    {
        let log_entry = state
            .store
            .publish_log_entry(&ident, &row.version)
            .map_err(internal)?
            .map(|entry| LogEntry {
                index: entry.index,
                leaf_hash: hex::encode(entry.leaf_hash),
            });
        let abi_index =
            serde_json::from_str(&row.abi_index).unwrap_or_else(|_| serde_json::json!({}));
        let epoch = u32::try_from(row.transfer_epoch)
            .map_err(|_| internal(format!("release {} has a negative epoch", row.version)))?;
        versions.push(IndexVersion {
            version: row.version,
            hash: row.hash,
            published_at: row.published_at,
            state: row.state,
            abi_index,
            log_entry,
            publisher: row.publisher_display.map(|owner| Publisher {
                owner,
                ident_fingerprint: row.publisher_ident_fingerprint,
            }),
            epoch,
        });
    }
    let (server_public, server_private) = state.store.server_keypair().map_err(internal)?;
    let name_binding = crypto::sign(
        &server_private,
        &crypto::name_binding_message(&owner_record.owner_display, &ident_key.fingerprint),
    )
    .map_err(internal)?;
    let mut transfers = Vec::new();
    for record in state.store.transfer_history(&ident).map_err(internal)? {
        let Some((to_record, to_key)) = state
            .store
            .owner_with_ident_key(&record.to_owner)
            .map_err(internal)?
        else {
            return Err(internal(format!(
                "transfer recipient {} has no current ident key",
                record.to_owner
            )));
        };
        let to_binding = crypto::sign(
            &server_private,
            &crypto::name_binding_message(&to_record.owner_display, &to_key.fingerprint),
        )
        .map_err(internal)?;
        transfers.push(TransferLink {
            sequence: record.sequence,
            from_owner: record.from_owner,
            from_ident_key: record.from_ident_key.as_deref().map(metadata_key),
            to_owner: record.to_owner,
            to_ident_key: record.to_ident_key.as_deref().map(metadata_key),
            release_digest: record.release_digest,
            offer_signature: record.offer_signature.as_deref().map(crypto::encode_bytes),
            accept_signature: record.accept_signature.as_deref().map(crypto::encode_bytes),
            offered_at: record.offered_at,
            accepted_at: record.accepted_at,
            to_binding: OwnerBinding {
                owner: to_record.owner_display,
                ident_key: metadata_key(&to_key.public_key),
                ident_fingerprint: to_key.fingerprint,
                name_binding_signature: crypto::encode_bytes(&to_binding),
            },
        });
    }
    Ok(Json(IndexResponse {
        ident: ident.clone(),
        owner: owner_record.owner_display,
        ident_key: metadata_key(&ident_key.public_key),
        ident_fingerprint: ident_key.fingerprint,
        name_binding_signature: crypto::encode_bytes(&name_binding),
        server_fingerprint: crypto::fingerprint(&server_public),
        versions,
        transfers,
    }))
}
