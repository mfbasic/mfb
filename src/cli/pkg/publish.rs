//! plan-176-L: `mfb pkg` — the publish commands. The dispatcher `run_pkg_command` and the
//! shared `use` header live in `cli/pkg/mod.rs`.

use super::*;

// coverage:off — builds and uploads a package to a live registry
// (validate_package/publish_package/verify_publish_inclusion); the argument
// validation and the package-project gate are unit-tested via run_pkg_command,
// and the full publish is covered by the tests/ integration harness.
pub(crate) fn publish_package_project(owner: &str, project_dir: &Path) -> Result<(), String> {
    let project_path = project_dir.join("project.json");
    let manifest = validate_project_manifest(&project_path)
        .map_err(|_| "package project validation failed".to_string())?;
    if project_kind(&manifest) != "package" {
        return Err("mfb repo publish requires a package project".to_string());
    }

    build_project(&BuildOptions {
        location: project_dir.to_path_buf(),
        outputs: Vec::new(),
        package_output_dir: None,
        target: target::BuildTarget::host(),
        sign_owner: Some(owner.to_string()),
        app_mode: false,
        app_debug: false,
        debug: false,
        // A package build emits a `.mfp`, which never folds `os::getConfigOr`;
        // `release` is still selected and validated (plan-167-C).
        config: None,
        opt: crate::optimizer::active_opt_level(),
        allow_unsigned: false,
        mode: crate::testing::CompileMode::Build,
        verbosity: Verbosity::Quiet,
    })
    .map_err(|_| "package build failed".to_string())?;

    let package_name = manifest
        .get("name")
        .and_then(|value| value.get::<String>())
        .expect("validated project name");
    let package_path = project_dir.join(format!("{package_name}.mfp"));
    let artifact = fs::read(&package_path).map_err(|err| {
        format!(
            "failed to read built package '{}': {err}",
            package_path.display()
        )
    })?;
    let package = mfb_repository::package::parse_mfp_package(&artifact).map_err(|err| {
        format!(
            "failed to verify built package '{}': {err}",
            package_path.display()
        )
    })?;
    binary_repr::read_package_info(&package_path)?;

    let repo_url = mfb_repository::client::repo_url_from_env();
    let paths = super::local_paths_for_repo(&repo_url)?;
    let content_hash = package.content_hash_hex();
    let ident_fingerprint = package.ident_fingerprint()?;
    let signing_fingerprint = package.signing_fingerprint()?;
    let artifact_request = mfb_repository::client::PackageArtifact {
        ident: &package.ident,
        version: &package.version,
        artifact: &artifact,
        content_hash: &content_hash,
        ident_fingerprint: &ident_fingerprint,
        signing_fingerprint: &signing_fingerprint,
    };

    // Refuse to publish into a suspect registry: the checkpoint must verify
    // under the pinned server key and be append-only relative to the pinned
    // one (plan-23-B3) BEFORE anything is uploaded.
    //
    // A fork that simply grows passes a signature and monotonicity check, so
    // demanding the RFC-6962 consistency proof against the pinned head is what
    // actually establishes the new head extends the history this client already
    // saw (bug-276 R2). Since bug-582 every pin advance demands that proof, so
    // this is the same primitive as `fetch_checkpoint`; the name records intent.
    mfb_repository::client::verify_log_consistency(&repo_url, &paths).inspect_err(|err| {
        if err.contains("ROLLBACK") || err.contains("FORK") {
            crate::rules::show_general_diagnostic("REGISTRY_LOG_ROLLBACK", err);
        }
    })?;

    // plan-48-B §4.2: upload every `vendor` locator's file as its own blob —
    // skipping any the registry already has — BEFORE publishing the `.mfp`. The
    // `.mfp` is the commit point; blobs first means a successful publish never
    // leaves a section-10 hash dangling (the registry enforces the converse).
    let (_name, native_libraries) = binary_repr::read_package_native_libraries(&package_path)?;
    upload_vendor_blobs(&repo_url, &paths, owner, project_dir, &native_libraries)?;

    let report =
        mfb_repository::client::validate_package(&repo_url, &paths, owner, &artifact_request)?;
    print_publish_verify_report(&report);
    if !report.valid {
        return Err("package validation failed".to_string());
    }

    let response =
        mfb_repository::client::publish_package(&repo_url, &paths, owner, &artifact_request)?;
    println!(
        "Published {}@{} as {}",
        response.ident, response.version, response.hash
    );
    for warning in &response.warnings {
        println!("warning: {warning}");
    }
    println!(
        "Publish logged at index {} (leaf {})",
        response.log_entry.index, response.log_entry.leaf_hash
    );
    // Verify our own publish landed in the log under a signed,
    // rollback-checked checkpoint (plan-23-B3).
    let (_entry, checkpoint) = mfb_repository::client::verify_publish_inclusion(
        &repo_url,
        &paths,
        &response.ident,
        &response.version,
        &response.hash,
    )?;
    println!(
        "Inclusion verified against checkpoint (size {}, root {})",
        checkpoint.size, checkpoint.root_hash
    );
    Ok(())
}

/// plan-48-B §4.2: upload every `vendor` locator's file as its own blob before
/// the `.mfp` is published, skipping any the registry already holds.
///
/// A library may list the same vendored file for several platforms, so dedup by
/// content hash — HEAD/PUT each distinct blob at most once. The section-10 hash
/// **is** the upload key; the file is not re-hashed here (a second computation is
/// a second chance to disagree — plan-48-B §4.2), and the registry re-hashes the
/// body before storing regardless.
fn upload_vendor_blobs(
    repo_url: &str,
    paths: &mfb_repository::local::LocalPaths,
    owner: &str,
    project_dir: &Path,
    native_libraries: &binary_repr::NativeLibraryTable,
) -> Result<(), String> {
    use crate::manifest::libraries::LibType;
    use std::collections::HashSet;

    let mut seen: HashSet<String> = HashSet::new();
    let mut uploaded = 0usize;
    let mut skipped = 0usize;
    let mut session_token: Option<String> = None;
    for entry in &native_libraries.entries {
        for locator in &entry.locators {
            if locator.lib_type != LibType::Vendor {
                continue;
            }
            // A vendor locator always carries a hash (encoder enforces it).
            let Some(hash) = locator.hash else { continue };
            let hash_hex = hex_bytes(&hash);
            if !seen.insert(hash_hex.clone()) {
                continue;
            }
            if mfb_repository::client::blob_exists(repo_url, &hash_hex)? {
                skipped += 1;
                continue;
            }
            let vendor_path = crate::manifest::libraries::vendor_path(project_dir, &locator.source);
            let bytes = fs::read(&vendor_path).map_err(|err| {
                format!(
                    "failed to read vendored library '{}': {err}",
                    vendor_path.display()
                )
            })?;
            let token = match &session_token {
                Some(token) => token.clone(),
                None => {
                    let token = mfb_repository::local::read_session(paths, owner)?;
                    session_token = Some(token.clone());
                    token
                }
            };
            println!(
                "Uploading vendor blob for \"{}\" ({}, {} bytes)",
                entry.logical,
                locator.source,
                bytes.len()
            );
            mfb_repository::client::put_blob(repo_url, &hash_hex, bytes, &token)?;
            uploaded += 1;
        }
    }
    if uploaded + skipped > 0 {
        println!("Vendor blobs: {uploaded} uploaded, {skipped} already present");
    }
    Ok(())
}

/// `mfb repo transfer <owner>#<package> <to-owner>` (plan-10-D1): the
/// package's current owner offers it to a recipient (signed with the local
/// ident key). The offering account is the current owner the registry's
/// `/index` names (plan-168-A), which after a transfer is no longer the ident
/// prefix, so a recipient can pass the package on.
pub(crate) fn transfer_offer(ident: &str, to_owner: &str) -> Result<(), String> {
    if ident.split_once('#').is_none() {
        return Err("ident must use <owner>#<package>".to_string());
    }
    let repo_url = mfb_repository::client::repo_url_from_env();
    let paths = super::local_paths_for_repo(&repo_url)?;
    let response = mfb_repository::client::transfer_offer(&repo_url, &paths, ident, to_owner)?;
    println!(
        "Offered {} to {}; they must run `mfb repo transfer-accept {}` to accept.",
        response.ident, response.to_owner, response.ident
    );
    Ok(())
}

/// `mfb repo transfer-accept <owner>#<package>@<to-owner>` (plan-10-D1): the
/// recipient accepts a pending transfer offer. The accepting account is named
/// explicitly after `@` in the argument (never prompted or inferred from a
/// session): the text before the first `@` is the `<owner>#<package>` ident and
/// the text after it is the `<to-owner>` recipient. A missing `@` is an error.
pub(crate) fn transfer_accept(ident: &str) -> Result<(), String> {
    // The recipient is whoever holds a local session able to accept; require
    // it explicitly via `<ident>@<to-owner>` to avoid ambiguity.
    let (ident, to_owner) = ident.split_once('@').ok_or_else(|| {
        "use <owner>#<package>@<to-owner> to name the accepting account".to_string()
    })?;
    let repo_url = mfb_repository::client::repo_url_from_env();
    let paths = super::local_paths_for_repo(&repo_url)?;
    let response = mfb_repository::client::transfer_accept(&repo_url, &paths, ident, to_owner)?;
    println!(
        "Accepted transfer of {} to {}.",
        response.ident, response.to_owner
    );
    Ok(())
}

/// `mfb repo release-state <state> [version]` (plan-10-C1): set a published
/// version's maintainer release state (`available`/`deprecated`/`yanked`). Run
/// in the package project; the ident and default version come from the
/// manifest, and the change is ident-signed and logged by the registry.
pub(crate) fn set_release_state(
    project_dir: &Path,
    state: &str,
    version_override: Option<&str>,
) -> Result<(), String> {
    if !matches!(state, "available" | "deprecated" | "yanked") {
        return Err(format!(
            "state must be one of available, deprecated, or yanked (got `{state}`)"
        ));
    }
    let project_path = project_dir.join("project.json");
    let manifest = validate_project_manifest(&project_path)
        .map_err(|_| "package project validation failed".to_string())?;
    if project_kind(&manifest) != "package" {
        return Err("mfb repo release-state requires a package project".to_string());
    }
    let ident = manifest
        .get("ident")
        .and_then(|value| value.get::<String>())
        .cloned()
        .ok_or_else(|| "project.json must declare an `ident` of <owner>#<package>".to_string())?;
    let Some((owner, _package)) = ident.split_once('#') else {
        return Err(format!(
            "project ident `{ident}` must use <owner>#<package>"
        ));
    };
    let version = version_override.map(str::to_string).unwrap_or_else(|| {
        manifest
            .get("version")
            .and_then(|value| value.get::<String>())
            .cloned()
            .unwrap_or_default()
    });
    if version.is_empty() {
        return Err(
            "no version to set state on (pass one or declare it in project.json)".to_string(),
        );
    }

    let repo_url = mfb_repository::client::repo_url_from_env();
    let paths = super::local_paths_for_repo(&repo_url)?;
    let response = mfb_repository::client::set_release_state(
        &repo_url, &paths, owner, &ident, &version, state,
    )?;
    println!(
        "Set {}@{} to {} (logged at index {})",
        response.ident, response.version, response.state, response.log_entry.index
    );
    Ok(())
}

/// `mfb repo check-abi` (plan-10-B1): build the working tree's package, compute
/// its per-symbol ABI index, and diff it against the latest published version's
/// index served by the registry. Names every changed or dropped symbol (both
/// break the superset relation the resolver relies on) and exits non-zero when
/// any breaking change is present; a pure superset (only additions) is OK.
pub(crate) fn check_abi(project_dir: &Path) -> Result<(), String> {
    let project_path = project_dir.join("project.json");
    let manifest = validate_project_manifest(&project_path)
        .map_err(|_| "package project validation failed".to_string())?;
    if project_kind(&manifest) != "package" {
        return Err("mfb repo check-abi requires a package project".to_string());
    }
    let ident = manifest
        .get("ident")
        .and_then(|value| value.get::<String>())
        .cloned()
        .ok_or_else(|| {
            "project.json must declare an `ident` of <owner>#<package> to compare against the registry"
                .to_string()
        })?;
    let Some((owner, package)) = ident.split_once('#') else {
        return Err(format!(
            "project ident `{ident}` must use <owner>#<package>"
        ));
    };

    // Build the working tree unsigned to emit its ABI index section.
    build_project(&BuildOptions {
        location: project_dir.to_path_buf(),
        outputs: Vec::new(),
        package_output_dir: None,
        target: target::BuildTarget::host(),
        sign_owner: None,
        app_mode: false,
        app_debug: false,
        debug: false,
        // A package build emits a `.mfp`, which never folds `os::getConfigOr`;
        // `release` is still selected and validated (plan-167-C).
        config: None,
        opt: crate::optimizer::active_opt_level(),
        allow_unsigned: false,
        mode: crate::testing::CompileMode::Build,
        verbosity: Verbosity::Quiet,
    })
    .map_err(|_| "package build failed".to_string())?;

    let package_name = manifest
        .get("name")
        .and_then(|value| value.get::<String>())
        .expect("validated project name");
    let package_path = project_dir.join(format!("{package_name}.mfp"));
    let info = binary_repr::read_package_info(&package_path)?;
    let mut working: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    for export in &info.exports {
        working.insert(export.name.clone(), export.sig_hash.clone());
    }

    let repo_url = mfb_repository::client::repo_url_from_env();
    let paths = super::local_paths_for_repo(&repo_url)?;
    let index = mfb_repository::client::fetch_index(&repo_url, &paths, owner, package)?;
    let Some(latest) = index.versions.iter().max_by_key(|entry| entry.published_at) else {
        println!("No published versions of {ident}; nothing to compare against.");
        return Ok(());
    };
    let published: std::collections::BTreeMap<String, String> = latest
        .abi_index
        .as_object()
        .map(|object| {
            object
                .iter()
                .filter_map(|(name, value)| {
                    value.as_str().map(|hash| (name.clone(), hash.to_string()))
                })
                .collect()
        })
        .unwrap_or_default();

    println!(
        "ABI comparison for {ident} against published version {}:",
        latest.version
    );
    let mut changed = Vec::new();
    let mut dropped = Vec::new();
    for (name, hash) in &published {
        match working.get(name) {
            Some(current) if current != hash => changed.push(name.clone()),
            Some(_) => {}
            None => dropped.push(name.clone()),
        }
    }
    let added: Vec<&String> = working
        .keys()
        .filter(|name| !published.contains_key(*name))
        .collect();
    for name in &changed {
        println!("  changed: {name}");
    }
    for name in &dropped {
        println!("  dropped: {name}");
    }
    for name in &added {
        println!("  added:   {name}");
    }
    if changed.is_empty() && dropped.is_empty() {
        if added.is_empty() {
            println!("  ABI is identical to the published version.");
        } else {
            println!("  ABI is a superset of the published version (backward-compatible).");
        }
        Ok(())
    } else {
        Err(format!(
            "ABI is not backward-compatible with {ident}@{}: {} changed, {} dropped",
            latest.version,
            changed.len(),
            dropped.len()
        ))
    }
}

pub(super) fn print_publish_verify_report(
    report: &mfb_repository::server::ValidatePackageResponse,
) {
    println!("Package validation report:");
    println!("  valid: {}", report.valid);
    println!("  content hash: {}", empty_marker(&report.content_hash));
    println!("  diagnostics:");
    if report.diagnostics.is_empty() {
        println!("    <none>");
    } else {
        for diagnostic in &report.diagnostics {
            println!("    {diagnostic}");
        }
    }
}
