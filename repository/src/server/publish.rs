//! plan-176-L: the registry server's publish code. `AppState`, `serve`, the router and the
//! error responses live in `repository/src/server/mod.rs`.

use super::*;

/// The `/validate` diagnostic for a release by a later owner of a package that
/// moved through an unsigned (legacy) transfer (plan-168-B, B-D3).
const LEGACY_TRANSFER_PUBLISH_REFUSED: &str = "package ownership moved before signed transfers, \
     so clients cannot verify a release by a later owner";

pub(super) async fn validate_package(
    State(state): State<AppState>,
    Json(request): Json<PackageArtifactRequest>,
) -> Result<Json<ValidatePackageResponse>, (StatusCode, Json<ErrorResponse>)> {
    let report =
        validate_package_request(&state, &request, "validate", VALIDATE_PER_OWNER_MAX).await?;
    Ok(Json(report))
}

pub(super) async fn publish_package(
    State(state): State<AppState>,
    Json(request): Json<PackageArtifactRequest>,
) -> Result<Json<PublishPackageResponse>, (StatusCode, Json<ErrorResponse>)> {
    let report =
        validate_package_request(&state, &request, "publish", PUBLISH_PER_OWNER_MAX).await?;
    if !report.valid {
        return Err(bad_request(format!(
            "package validation failed: {}",
            report.diagnostics.join("; ")
        )));
    }
    // Per-owner version quota (bug-188 / REPO-13): bound the permanent blob/DB
    // growth an authenticated flood can inflict. Checked before staging the blob
    // so a rejected publish leaves nothing behind. A re-publish of an existing
    // (ident, version) row updates in place and is not newly counted below.
    let quota_owner_id = verify_session_token(&state.store, &request.session_token)
        .map_err(bad_request)?
        .owner_id;
    let version_count = state
        .store
        .owner_version_count(quota_owner_id)
        .map_err(internal)?;
    if version_count >= MAX_VERSIONS_PER_OWNER
        && !state
            .store
            .package_version_exists(&request.ident, &request.version)
            .map_err(internal)?
    {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            Json(ErrorResponse {
                error: format!("per-owner version quota of {MAX_VERSIONS_PER_OWNER} reached"),
            }),
        ));
    }
    let artifact = crypto::decode_bytes(&request.artifact, "artifact").map_err(bad_request)?;
    let hash = report.content_hash;
    // The vendor locators section 10 names — recorded as version→blob edges so
    // a future GC (plan-49) can compute reachability (plan-48-A §4.5), and as
    // the native target matrix (plan-61-A §3). Their existence was already
    // enforced by the validation above; re-parse the already-verified payload
    // rather than threading the list through `report`.
    //
    // Keep the whole `VendorBlobRef`, not just `.hash`: the platform axis is
    // the metadata the target matrix is made of, and collapsing to hashes here
    // is what used to throw it away.
    let parsed = package::parse_mfp_package(&artifact).ok();
    let vendor_blobs: Vec<crate::abi::VendorBlobRef> = parsed
        .as_ref()
        .and_then(|package| crate::abi::parse_vendor_blobs(&package.payload).ok())
        .unwrap_or_default();
    // plan-61-A §4: render what the publisher *signed*. `author`/`url` exist
    // twice — in the plaintext header, and interned in MANIFEST section 1 inside
    // the signed payload — and only the second is covered by the signature. Take
    // the signed copy, and refuse a package whose two copies disagree rather
    // than silently preferring either: a mismatch is a malformed or tampered
    // artifact, and quietly picking one is how a registry ends up displaying
    // something nobody signed.
    let manifest_metadata = match parsed.as_ref() {
        Some(package) => {
            let signed = crate::abi::parse_manifest_metadata(&package.payload)
                .map_err(|err| bad_request(format!("failed to read package manifest: {err}")))?;
            if let Some(signed) = signed.as_ref() {
                if signed.author != package.author || signed.url != package.url {
                    return Err(bad_request(format!(
                        "package header and signed manifest disagree: header author {:?} url {:?}, \
                         manifest author {:?} url {:?}",
                        package.author, package.url, signed.author, signed.url,
                    )));
                }
            }
            signed
        }
        None => None,
    };
    // An empty string means the publisher set nothing; store NULL rather than
    // '' so "not provided" and "provided as empty" stay one fact, not two.
    // plan-61-E: the description rides in MFPC section 18, added by plan-61-D.
    // A package built before that carries no section 18 at all, which is a
    // normal outcome and stays NULL — not an error and not a warning.
    let description = parsed
        .as_ref()
        .and_then(|package| crate::abi::parse_package_description(&package.payload).ok())
        .flatten()
        .filter(|value| !value.is_empty());
    // plan-126-E: capture MFPC section 17 (the `DOC` table) from the payload this
    // handler already holds, best-effort. The RAW section bytes are what gets
    // stored -- `mfb_wire::docs` stays the only decoder -- but only once they
    // decode, so a malformed doc table is never recorded. It is never a reason to
    // refuse the publish either: documentation "does not affect execution or the
    // ABI" (src/binary_repr/writer.rs), and turning a doc-table defect into a
    // rejected signed package would be a new failure mode on a trust path. Same
    // posture as `parse_package_description` directly above.
    let docs = parsed.as_ref().and_then(|package| {
        let sections = mfb_wire::mfpc::read_section_table(&package.payload).ok()?;
        let section = sections.get(&mfb_wire::mfpc::SECTION_DOC_TABLE)?;
        mfb_wire::docs::read_doc_table(section).ok()?;
        Some(section.to_vec())
    });
    // plan-168-A §4.3: the publisher's ident key is the one the header names,
    // which validation has already required to equal `identFingerprint` and the
    // publisher's current ident.
    let publisher_ident_fingerprint = Some(request.ident_fingerprint.clone());
    let publish_metadata = match manifest_metadata {
        Some(meta) => crate::store::PublishMetadata {
            author: Some(meta.author).filter(|value| !value.is_empty()),
            url: Some(meta.url).filter(|value| !value.is_empty()),
            description,
            docs,
            publisher_ident_fingerprint,
        },
        // `docs` is set explicitly here too. Left to `..Default::default()` it
        // would silently become `None` for every package without a MANIFEST
        // section, dropping documentation the publisher did sign.
        None => crate::store::PublishMetadata {
            description,
            docs,
            publisher_ident_fingerprint,
            ..Default::default()
        },
    };
    let already_present = state
        .blob_store
        .exists(&hash, BlobKind::Package)
        .await
        .map_err(internal)?;
    // Blob-ordering fix (plan-10-A §2.6): stage the blob, commit the DB row, and
    // only then promote it to servable — a failed transaction leaves no orphan
    // blob, and a served blob always has a committed version row. The blob
    // backend (local file or S3 object) implements the stage/promote/abort
    // protocol.
    let staged = if already_present {
        None
    } else {
        Some(
            state
                .blob_store
                .stage(&hash, BlobKind::Package, artifact)
                .await
                .map_err(internal)?,
        )
    };
    let owner_id = verify_session_token(&state.store, &request.session_token)
        .map_err(bad_request)?
        .owner_id;
    // Persist the ABI index alongside the version row (plan-10-B1); it is
    // parsed from the same validated artifact `report` covered.
    let abi_index = serde_json::to_string(&report.abi_index).unwrap_or_else(|_| "{}".to_string());
    let published = match state.store.publish_package_version(
        owner_id,
        &request.ident,
        &request.version,
        &hash,
        &state.blob_store.blob_ref(&hash, BlobKind::Package),
        &abi_index,
        &vendor_blobs,
        &publish_metadata,
    ) {
        Ok(published) => published,
        Err(err) => {
            if let Some(staged) = staged {
                state.blob_store.abort(staged).await;
            }
            return Err(conflict_or_bad_request(err));
        }
    };
    let blob_stored = if let Some(staged) = staged {
        state.blob_store.promote(staged).await.map_err(internal)?;
        true
    } else {
        false
    };
    // Warn-only typosquat check (plan-10-D2): surface near-duplicate idents so
    // the publisher can notice an impersonation attempt; never blocks.
    let warnings = state
        .store
        .typosquat_candidates(&published.ident)
        .unwrap_or_default()
        .into_iter()
        .map(|existing| {
            format!(
                "published `{}` is one edit away from existing `{existing}`",
                published.ident
            )
        })
        .collect();
    Ok(Json(PublishPackageResponse {
        ident: published.ident,
        version: published.version,
        hash: published.hash,
        published_at: published.published_at,
        state: published.state,
        blob_stored,
        log_entry: LogEntry {
            index: published.log_entry.index,
            leaf_hash: hex::encode(published.log_entry.leaf_hash),
        },
        warnings,
    }))
}

pub(super) async fn validate_package_request(
    state: &AppState,
    request: &PackageArtifactRequest,
    route: &str,
    per_owner_max: usize,
) -> Result<ValidatePackageResponse, (StatusCode, Json<ErrorResponse>)> {
    let claims = verify_session_token(&state.store, &request.session_token).map_err(bad_request)?;
    // Per-owner sliding-window throttle on the expensive authenticated routes
    // (bug-188 / REPO-13). Registration is open, so "authenticated" is near
    // anonymous; without this a single owner could hammer /validate's Ed25519
    // verifies or /publish's permanent blob writes. Keyed per route so a publish
    // flood does not exhaust the validate budget and vice-versa.
    if !state
        .rate_limiter
        .allow(&format!("{route}:{}", claims.sub), per_owner_max, 60)
    {
        return Err(too_many_requests());
    }
    // bug-492: a publish token's scope and expiry bound `/validate` and
    // `/publish` exactly as they bound `/signing`. A hard refusal rather than a
    // diagnostic — the artifact may be perfectly valid; the credential is not —
    // and checked before any of the expensive work below. `request.ident` is
    // enough here: a package whose own ident differs fails the equality
    // diagnostic further down and can never publish.
    if let Some((_owner, key)) = state
        .store
        .owner_auth_key_by_fingerprint(&claims.sub, &claims.auth_fingerprint)
        .map_err(internal)?
    {
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
    }
    let artifact = crypto::decode_bytes(&request.artifact, "artifact").map_err(bad_request)?;
    let package = match package::parse_mfp_package(&artifact) {
        Ok(package) => package,
        Err(err) => {
            return Ok(invalid_report(String::new(), vec![err]));
        }
    };
    let hash = package.content_hash_hex();
    let mut diagnostics = Vec::new();
    if request.content_hash != hash {
        diagnostics.push("request contentHash does not match package content hash".to_string());
    }
    if request.ident != package.ident {
        diagnostics.push("request ident does not match package ident".to_string());
    }
    if request.version != package.version {
        diagnostics.push("request version does not match package version".to_string());
    }
    let header_ident_fingerprint = package.ident_fingerprint().unwrap_or_default();
    let header_signing_fingerprint = package.signing_fingerprint().unwrap_or_default();
    if request.ident_fingerprint != header_ident_fingerprint {
        diagnostics.push("request identFingerprint does not match package header".to_string());
    }
    if request.signing_fingerprint != header_signing_fingerprint {
        diagnostics.push("request signingFingerprint does not match package header".to_string());
    }

    // §3.4 step 1 — the session owner publishes, and must be entitled to the
    // ident. bug-792 / plan-168-B: when the package exists its STORED owner
    // decides (the same rule `/signing` and `/release-state` apply through
    // `require_package_owner`); the `owner#` prefix decides only a first
    // publish. The prefix never moves when ownership does, so binding the
    // publisher to it stranded every transferred package.
    let Some((owner_part, _package_part)) = package.ident.split_once('#') else {
        diagnostics.push("package ident must use <owner>#<package>".to_string());
        return Ok(invalid_report(hash, diagnostics));
    };
    match state
        .store
        .package_owner(&package.ident)
        .map_err(internal)?
    {
        Some(current) => {
            if current.id != claims.owner_id {
                diagnostics.push("session owner is not the package's current owner".to_string());
            } else if crate::validation::fold_owner(owner_part)
                != crate::validation::fold_owner(&claims.sub)
                && state
                    .store
                    .has_legacy_transfer(&package.ident)
                    .map_err(internal)?
            {
                // Clients stop trusting owners at the first unsigned (legacy)
                // transfer link and fall back to the origin owner alone, so a
                // later owner's release would publish and then never install.
                diagnostics.push(LEGACY_TRANSFER_PUBLISH_REFUSED.to_string());
            }
        }
        None => {
            if crate::validation::fold_owner(owner_part)
                != crate::validation::fold_owner(&claims.sub)
            {
                diagnostics.push("session owner does not match package ident owner".to_string());
            }
            if state
                .store
                .owner_with_ident_key(owner_part)
                .map_err(internal)?
                .is_none()
            {
                diagnostics.push("package ident owner is not registered".to_string());
                return Ok(invalid_report(hash, diagnostics));
            }
        }
    }
    let Some((owner, _key)) = state
        .store
        .owner_auth_key_by_fingerprint(&claims.sub, &claims.auth_fingerprint)
        .map_err(internal)?
    else {
        diagnostics.push("session key does not match current owner key".to_string());
        return Ok(invalid_report(hash, diagnostics));
    };
    if owner.id != claims.owner_id {
        diagnostics.push("session key does not match current owner key".to_string());
    }
    if package.author != owner.owner_display {
        diagnostics.push("package author does not match owner name".to_string());
    }
    if package.signature_type != 1 {
        diagnostics.push("registry publishes require an Ed25519-signed package".to_string());
        return Ok(invalid_report(hash, diagnostics));
    }

    // §3.4 steps 2–4 — the attestation verifies under OUR key, names OUR
    // fingerprint, and pins this exact ident/version/identKey/signingKey.
    let (server_public, _server_private) = state.store.server_keypair().map_err(internal)?;
    if let Err(err) = package::verify_attestation(
        &package,
        &server_public,
        &crypto::fingerprint(&server_public),
    ) {
        diagnostics.push(format!("attestation verification failed: {err}"));
    }

    // §3.4 step 5 — the attestation must match the server's CURRENT
    // name↔ident binding of the PUBLISHER (the session owner, plan-168-B); a
    // stale attestation (pre-rotation) is refused.
    match state
        .store
        .owner_with_ident_key(&owner.owner_display)
        .map_err(internal)?
    {
        Some((_ident_owner, ident_key)) => {
            if header_ident_fingerprint != ident_key.fingerprint {
                diagnostics.push(
                    "package identKey does not match the owner's current ident key".to_string(),
                );
            }
        }
        None => diagnostics.push("owner has no current ident key".to_string()),
    }

    // §3.4 step 6 — the proof verifies under the header identKey and pins
    // this exact package.
    match package::decode_metadata_key(&package.ident_key, "identKey") {
        Ok(ident_public) => {
            if let Err(err) = package::verify_proof(&package, &ident_public) {
                diagnostics.push(format!("proof verification failed: {err}"));
            }
        }
        Err(err) => diagnostics.push(err),
    }

    // §3.4 step 7 — the payload hash welds header to payload and the package
    // signature verifies under the one-off signing key.
    if let Err(err) = package::verify_payload_hash(&package) {
        diagnostics.push(err);
    }
    if let Err(err) = package::verify_package_signature(&package) {
        diagnostics.push(format!("package signature verification failed: {err}"));
    }

    if state
        .store
        .package_version_exists(&package.ident, &package.version)
        .map_err(internal)?
    {
        diagnostics.push(format!(
            "package version {}@{} is already published",
            package.ident, package.version
        ));
    }

    // plan-48-A §4.4: every `vendor` locator named by section 10 must already
    // have its blob uploaded, so a successful publish can never leave a
    // section-10 hash dangling. Applied on both /validate (a dry run reports
    // missing blobs before the publisher uploads anything) and /publish (which
    // funnels through this function). The section rides inside the signed+welded
    // payload, so these hashes are authenticated by re-hash alone (§3.2).
    match crate::abi::parse_vendor_blobs(&package.payload) {
        Ok(vendor_blobs) => {
            // Probe each distinct hash once (bug-275). A library legitimately
            // lists the same vendored file for several platforms, so without
            // dedup every repeat costs another backend round trip; the parser's
            // locator cap bounds the count, and this bounds the duplicates.
            let mut probed: std::collections::HashMap<&str, bool> =
                std::collections::HashMap::new();
            for vref in &vendor_blobs {
                if !probed.contains_key(vref.hash.as_str()) {
                    let exists = state
                        .blob_store
                        .exists(&vref.hash, BlobKind::Native)
                        .await
                        .map_err(internal)?;
                    probed.insert(vref.hash.as_str(), exists);
                }
            }
            // Diagnostics stay per-locator so a missing blob still names every
            // logical library and source filename that referenced it.
            for vref in &vendor_blobs {
                if !probed.get(vref.hash.as_str()).copied().unwrap_or(false) {
                    diagnostics.push(format!(
                        "native library '{}' references vendor blob {} ({}) that is not uploaded",
                        vref.logical, vref.hash, vref.source
                    ));
                }
            }
        }
        Err(err) => diagnostics.push(format!("native library table is malformed: {err}")),
    }

    // The per-symbol ABI index (plan-10-B1) is parsed best-effort from the
    // payload; it is covered by packageBinaryHash + the signature, so the
    // registry serves it for resolution without having to trust it.
    let abi_index = crate::abi::abi_index_json(&package.payload);

    Ok(ValidatePackageResponse {
        valid: diagnostics.is_empty(),
        content_hash: hash,
        abi_index,
        diagnostics,
    })
}

fn invalid_report(content_hash: String, diagnostics: Vec<String>) -> ValidatePackageResponse {
    ValidatePackageResponse {
        valid: false,
        content_hash,
        abi_index: serde_json::json!({}),
        diagnostics,
    }
}
