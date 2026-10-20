//! plan-176-L: `mfb pkg` — the add commands. The dispatcher `run_pkg_command` and the
//! shared `use` header live in `cli/pkg/mod.rs`.

use super::*;

/// The parsed form of `mfb pkg add <target> [--pin|--no-pin]` (plan-60-C §4.3).
pub(super) struct AddOptions {
    pub(super) target: String,
    pub(super) pin: Option<bool>,
}

/// Parse `add`'s arguments, following `run_pkg_doc`'s shape (`:1177`): one
/// positional, explicit flags, unknown-flag rejection, second-positional
/// rejection.
///
/// `--pin` and `--no-pin` together are a **usage error**, not last-flag-wins:
/// the two orderings would otherwise mean different things with no way to tell
/// from the command line which was intended (§4.1).
pub(super) fn parse_add_options(args: &[String]) -> Result<AddOptions, PkgCommandError> {
    let mut target: Option<String> = None;
    let mut pin = false;
    let mut no_pin = false;

    for arg in args {
        match arg.as_str() {
            "--pin" => pin = true,
            "--no-pin" => no_pin = true,
            flag if flag.starts_with("--") => {
                return Err(PkgCommandError::Usage(format!("unknown flag `{flag}`")));
            }
            value => {
                if target.is_some() {
                    return Err(PkgCommandError::Usage(
                        "mfb pkg add accepts exactly one <target>".to_string(),
                    ));
                }
                target = Some(value.to_string());
            }
        }
    }

    if pin && no_pin {
        return Err(PkgCommandError::Usage(
            "mfb pkg add cannot take both --pin and --no-pin".to_string(),
        ));
    }
    let target = target.ok_or_else(|| {
        PkgCommandError::Usage(format!(
            "mfb pkg add requires exactly one <url>\n\n{PKG_HELP}"
        ))
    })?;

    Ok(AddOptions {
        target,
        pin: if pin {
            Some(true)
        } else if no_pin {
            Some(false)
        } else {
            None
        },
    })
}

/// §4.1's inference matrix, as a pure function.
///
/// The rule: **an explicit `@version` implies `--pin`; an explicit flag always
/// wins.** Under `pin: false` the `version` field is not the version you get —
/// it is the ABI *floor* the resolver anchors on, which is why
/// `@version --no-pin` is a meaningful combination rather than a contradiction.
pub(super) fn infer_pin(has_explicit_version: bool, pin_flag: Option<bool>) -> bool {
    match pin_flag {
        Some(explicit) => explicit,
        None => has_explicit_version,
    }
}

pub(super) fn add_package(project_dir: &Path, options: &AddOptions) -> Result<(), String> {
    let target = options.target.as_str();
    if target.starts_with("file://") {
        // §4.2: pin inference does not apply to a local copy — there is no
        // registry version stream to float along. `--no-pin` is therefore a
        // contradiction rather than a preference, and saying so is better than
        // silently writing `pin: true` and leaving the user to discover it.
        if options.pin == Some(false) {
            return Err(
                "mfb pkg add --no-pin cannot apply to a file:// package: there is no registry \
                 version stream to float along"
                    .to_string(),
            );
        }
        add_package_from_file(project_dir, target)
    } else if target.contains('#') {
        add_package_from_registry(project_dir, target, options.pin)
    } else {
        Err(format!(
            "mfb pkg add expects a file:// URL or an <owner>#<package>[@version] ident, got `{target}`"
        ))
    }
}

/// plan-48-B (Open Decisions): a local `.mfp` copy has no registry to fetch
/// vendor blobs from. A package that vendors native libraries would install but
/// never build, so refuse it explicitly rather than leaving a silently unusable
/// install.
fn refuse_vendored_local_package(path: &Path, name: &str, ident: &str) -> Result<(), String> {
    let (_name, native_libraries) = binary_repr::read_package_native_libraries(path)?;
    if native_libraries
        .entries
        .iter()
        .flat_map(|entry| &entry.locators)
        .any(|locator| locator.lib_type == crate::manifest::libraries::LibType::Vendor)
    {
        return Err(format!(
            "`{name}` vendors native libraries, which a `file://` add cannot fetch (there is no \
             registry). Publish it and `mfb pkg add {ident}#…` instead."
        ));
    }
    Ok(())
}

/// Copy a compiled dependency another declared package needs into
/// `packages/<name>.mfp` (bug-628), as a `file://` add would. A file already at
/// the destination is left alone.
pub(crate) fn install_local_package(
    project_dir: &Path,
    name: &str,
    source_path: &Path,
) -> Result<(), String> {
    let packages_dir = project_dir.join("packages");
    let destination = packages_dir.join(format!("{name}.mfp"));
    if let (Ok(from), Ok(to)) = (
        fs::canonicalize(source_path),
        fs::canonicalize(&destination),
    ) {
        if from == to {
            return Ok(());
        }
    }
    let package = read_mfp_header(source_path)?;
    if package.name != name {
        return Err(format!(
            "'{}' is package `{}`, not `{name}`",
            source_path.display(),
            package.name
        ));
    }
    refuse_vendored_local_package(source_path, &package.name, &package.ident)?;
    fs::create_dir_all(&packages_dir)
        .map_err(|err| format!("failed to create '{}': {err}", packages_dir.display()))?;
    let blob = fs::read(source_path)
        .map_err(|err| format!("failed to read '{}': {err}", source_path.display()))?;
    let staged = super::stage_package_blob(&packages_dir, name, &blob)?;
    super::commit_staged_package(&staged, &destination)
}

/// `contents` with the entry named `name` removed when it is an indirect one
/// (bug-628): adding a package the closure already declared makes it direct, at
/// the version and source the user now names.
fn without_indirect_entry(
    contents: &str,
    manifest: &std::collections::HashMap<String, JsonValue>,
    name: &str,
) -> Result<String, String> {
    let indirect = crate::manifest::closure::declared_dependencies(manifest)
        .into_iter()
        .find(|dependency| dependency.name == name && dependency.direct == Some(false));
    match indirect {
        Some(dependency) => project_json_without_packages(contents, &[dependency.ident.as_str()]),
        None => Ok(contents.to_string()),
    }
}

fn add_package_from_file(project_dir: &Path, url: &str) -> Result<(), String> {
    let source_path = package_file_url_path(url)?;
    let package = read_mfp_header(&source_path)?;
    refuse_vendored_local_package(&source_path, &package.name, &package.ident)?;

    let project_path = project_dir.join("project.json");
    let contents = fs::read_to_string(&project_path)
        .map_err(|err| format!("failed to read '{}': {err}", project_path.display()))?;
    let manifest = parse_project_json(&contents, &project_path)?;
    validate_packages_array(&manifest)?;
    let contents = without_indirect_entry(&contents, &manifest, &package.name)?;
    let manifest = parse_project_json(&contents, &project_path)?;

    let package_filename = format!("{}.mfp", package.name);
    // Trust-on-first-use (plan-23 §3.5): adding a SIGNED package pins its
    // identKey in the dependency entry. From then on the pin — never the
    // file-embedded key — is the trust anchor every build verifies against.
    let dependency = ProjectPackageDependency {
        name: package.name.clone(),
        ident: package.ident.clone(),
        version: package.version.clone(),
        pin: true,
        source: url.to_string(),
        ident_key: package.ident_key.clone(),
        direct: Some(true),
        required_by: Some(Vec::new()),
    };
    let updated = project_json_with_package(&contents, &manifest, &dependency)?;

    let packages_dir = project_dir.join("packages");
    fs::create_dir_all(&packages_dir)
        .map_err(|err| format!("failed to create '{}': {err}", packages_dir.display()))?;

    // `read_mfp_header` has already rejected a `name` that is not a single path
    // component. Copy through a staged file so a symlink planted at the
    // destination is replaced by the rename rather than written through.
    let blob = fs::read(&source_path)
        .map_err(|err| format!("failed to read '{}': {err}", source_path.display()))?;
    let staged = super::stage_package_blob(&packages_dir, &package.name, &blob)?;
    let destination = packages_dir.join(&package_filename);
    super::commit_staged_package(&staged, &destination)?;

    // plan-60-C Phase 3 (§5's chosen branch): route the manifest write through
    // the pipeline so the lock stays current. A `file://` dependency is not a
    // resolver node — plan-60-C §5 fixed `resolve()` to exclude it by `source`
    // — but it still contributes to `projectHash`, so the lock must be rewritten
    // or `mfb pkg install` would hard-error on a stale lock. That rewrite is
    // this letter's whole point.
    //
    // The local copy is committed above rather than through the pipeline: it
    // comes from a path the user named, not from the registry, so there is no
    // blob for `install()` to fetch.
    super::resolve::apply_manifest_change(project_dir, &updated)?;

    println!(
        "Added package {} {} to {} (pinned)",
        package.name,
        package.version,
        project_path.display()
    );
    Ok(())
}

/// Install a package from the registry (plan-10-A): resolve `/index`, pin the
/// registry-vouched identKey, download `/blob/<hash>`, run the full §3.5
/// verification chain, and install into `packages/`.
// coverage:off — the ident-validation guards are unit-tested; everything past
// fetch_index reaches a live registry and is covered by the tests/ package-add
// integration harness.
pub(super) fn add_package_from_registry(
    project_dir: &Path,
    target: &str,
    pin_flag: Option<bool>,
) -> Result<(), String> {
    let (ident, requested_version) = match target.split_once('@') {
        Some((ident, version)) if !version.is_empty() => (ident, Some(version)),
        Some((_, _)) => return Err("version after `@` must not be empty".to_string()),
        None => (target, None),
    };
    let Some((owner, package)) = ident.split_once('#') else {
        return Err("registry ident must use <owner>#<package>".to_string());
    };
    if owner.is_empty() || package.is_empty() {
        return Err("registry ident must use <owner>#<package>".to_string());
    }

    let project_path = project_dir.join("project.json");
    let contents = fs::read_to_string(&project_path)
        .map_err(|err| format!("failed to read '{}': {err}", project_path.display()))?;
    let manifest = parse_project_json(&contents, &project_path)?;
    validate_packages_array(&manifest)?;

    let repo_url = mfb_repository::client::repo_url_from_env();
    let paths = super::local_paths_for_repo(&repo_url)?;
    let index = mfb_repository::client::fetch_index(&repo_url, &paths, owner, package)?;

    // Pick the requested version, or the newest install-eligible one
    // (yanked/blocked/legal-tombstoned are excluded from a floating add).
    // Determines the `version` VALUE written into project.json — which under
    // `pin: false` is the resolver's ABI *floor*, not the version that gets
    // installed. `apply_manifest_change`'s `resolve()` performs the actual
    // selection from that anchor. Kept because the floor still has to be a real
    // published version.
    let chosen = select_index_version(&index, requested_version)?;
    let blob = mfb_repository::client::fetch_blob(&repo_url, &chosen.hash)?;
    let header = mfb_repository::package::parse_mfp_package(&blob)
        .map_err(|err| format!("registry returned a malformed package: {err}"))?;
    let full_ident = format!("{owner}#{package}");
    if header.ident != full_ident {
        return Err(format!(
            "downloaded package ident `{}` does not match `{full_ident}`",
            header.ident
        ));
    }
    // plan-168-C: the pin is the identKey of THIS release, verified for its
    // publisher through the signed transfer chain from the registry's name
    // binding (trust on first use, as before). Pinning the index's current key
    // instead refused every release signed before a rotation or a transfer.
    let ident_key = match verify_registry_release(
        &repo_url,
        &paths,
        &index,
        chosen,
        &blob,
        mfb_repository::trust::Anchor::FirstUse,
    ) {
        Ok(Some(trusted)) => {
            if trusted.crossed_transfer {
                print_transfer_notice(&full_ident, &trusted.publisher);
            }
            metadata_key(&trusted.key)
        }
        // An unsigned package carries no key to verify; its pin is never read.
        Ok(None) => index.ident_key.clone(),
        Err(err) => {
            return Err(format!(
                "{}: refusing to add `{full_ident}`@{}: {err}",
                err.rule(),
                chosen.version
            ))
        }
    };

    // plan-60-C §4.1: an explicit `@version` implies a pin; an explicit flag
    // always wins. Under `pin: false` the `version` written here is the ABI
    // FLOOR the resolver anchors on, not the version you are stuck with.
    let pin = infer_pin(requested_version.is_some(), pin_flag);
    let dependency = ProjectPackageDependency {
        name: header.name.clone(),
        ident: full_ident.clone(),
        version: header.version.clone(),
        pin,
        source: full_ident,
        ident_key,
        direct: Some(true),
        required_by: Some(Vec::new()),
    };
    let contents = without_indirect_entry(&contents, &manifest, &dependency.name)?;
    let manifest = parse_project_json(&contents, &project_path)?;
    let updated = project_json_with_package(&contents, &manifest, &dependency)?;

    // plan-60-C Phase 3: resolve-first. The blob fetched above is deliberately
    // NOT installed here — `apply_manifest_change` resolves the proposed
    // manifest and its `install()` step fetches and verifies every package from
    // the resulting lock, including this one. Installing here as well would
    // write `packages/` before resolution had a chance to reject the change,
    // which is exactly the atomicity hole this letter closes.
    super::resolve::apply_manifest_change(project_dir, &updated)?;

    let suffix = if pin {
        "(pinned)".to_string()
    } else if requested_version.is_some() {
        format!("(floating, floor {})", header.version)
    } else {
        "(floating)".to_string()
    };
    println!(
        "Added package {} {} from {} to {} {suffix}",
        header.name,
        header.version,
        index.ident,
        project_path.display()
    );
    Ok(())
}

/// plan-168-C: whether a downloaded registry release is trusted for its
/// publisher, through the signed transfer chain of the verified `index`, from
/// `anchor`. `None` for an unsigned package, which carries no identKey.
///
/// `blob` is the release's bytes as fetched (`fetch_blob` has already checked
/// them against the content address); the header is read from them, never
/// from the index.
// coverage:off — reads the owners' rotation chains from the registry; the
// verifier itself is `mfb_repository::trust`, unit-tested without a network.
pub(crate) fn verify_registry_release(
    repo_url: &str,
    paths: &mfb_repository::local::LocalPaths,
    index: &mfb_repository::server::IndexResponse,
    version: &mfb_repository::server::IndexVersion,
    blob: &[u8],
    anchor: mfb_repository::trust::Anchor,
) -> Result<Option<mfb_repository::trust::Trusted>, mfb_repository::trust::TrustError> {
    use mfb_repository::trust::{Release, TrustError};
    let header = mfb_repository::package::parse_mfp_package(blob)
        .map_err(|err| TrustError::Untrusted(format!("malformed package: {err}")))?;
    if header.signature_type == 0 || header.ident_key.is_empty() {
        return Ok(None);
    }
    let header_key = mfb_repository::package::decode_metadata_key(&header.ident_key, "identKey")
        .map_err(TrustError::Untrusted)?;
    let server_key = mfb_repository::local::read_pinned_server_key(paths)
        .map_err(|err| TrustError::Untrusted(format!("no pinned registry key: {err}")))?;
    mfb_repository::trust::verify_release(
        &server_key,
        index,
        &mfb_repository::client::registry_chains(repo_url),
        anchor,
        &Release {
            version,
            content_hash: &header.content_hash_hex(),
            header_ident_key: &header_key,
            header_author: &header.author,
        },
    )
    .map(Some)
}

// coverage:on

/// A public key in project-metadata form, `ed25519:<base64url>`.
pub(crate) fn metadata_key(public_key: &[u8]) -> String {
    format!(
        "ed25519:{}",
        mfb_repository::crypto::encode_bytes(public_key)
    )
}

/// The notice printed when a pin moves to a release published by another owner
/// of the package, through a verified signed transfer (plan-168-C).
pub(crate) fn print_transfer_notice(ident: &str, publisher: &str) {
    println!(
        "notice: ownership of {} moved to {} through a signed transfer",
        crate::terminal_safe::safe(ident),
        crate::terminal_safe::safe(publisher)
    );
}

/// plan-48-B §4.4: download and place every `vendor` blob a just-verified
/// package's section-10 table names.
///
/// Called only after the `.mfp` fully verified, so section 10 — and every hash
/// in it — is trusted. Each blob is fetched by content hash (`fetch_blob`
/// re-hashes, so a substituted blob fails there), then placed under
/// `packages/<name>.vendor/<source>` with stage-verify-rename. A missing or
/// tampered blob is fatal and leaves nothing usable on disk.
///
/// Every vendor blob in the table is downloaded — not just the host target's —
/// so a later cross-compile and an offline build both work.
// coverage:off — reaches a live registry; covered by the package-add integration
// harness and the end-to-end acceptance run.
pub(crate) fn install_vendor_blobs(
    repo_url: &str,
    project_dir: &Path,
    package_name: &str,
) -> Result<(), String> {
    use crate::manifest::libraries::LibType;
    use std::collections::HashMap;

    let installed = project_dir
        .join("packages")
        .join(format!("{package_name}.mfp"));
    let (_name, table) = binary_repr::read_package_native_libraries(&installed)?;
    let vendor_dir = crate::manifest::libraries::imported_vendor_dir(project_dir, package_name);

    // A content hash names bytes, not a file: two locators may ship identical
    // bytes under different source names, and every name is a requirement the
    // build looks up (bug-794). Each distinct hash is fetched once; each distinct
    // source name is placed, copied from the first file that hash produced.
    let mut fetched: HashMap<String, PathBuf> = HashMap::new();
    let mut placed_sources: HashMap<&str, String> = HashMap::new();
    let mut downloaded = 0usize;
    for entry in &table.entries {
        for locator in &entry.locators {
            if locator.lib_type != LibType::Vendor {
                continue;
            }
            let Some(hash) = locator.hash else { continue };
            let hash_hex = hex_bytes(&hash);
            // `source` came from the `.mfp` — untrusted input. plan-46-B §4.1's
            // decoder already re-checked the bare-filename rule, but do not assume
            // it: re-validate before `source` becomes a path.
            if let Err(reason) = crate::manifest::libraries::source_is_bare(&locator.source) {
                return Err(format!(
                    "package `{package_name}` names an unsafe vendored file \"{}\": {reason}",
                    locator.source
                ));
            }
            match placed_sources.get(locator.source.as_str()) {
                Some(placed_hash) if *placed_hash == hash_hex => continue,
                Some(placed_hash) => {
                    return Err(format!(
                        "package `{package_name}` names vendored file \"{}\" with two \
                         different contents (sha256 `{placed_hash}` and `{hash_hex}`)",
                        locator.source
                    ));
                }
                None => {}
            }
            if let Some(first) = fetched.get(&hash_hex) {
                let bytes = std::fs::read(first)
                    .map_err(|err| format!("failed to read '{}': {err}", first.display()))?;
                super::install_vendor_file(&vendor_dir, &locator.source, &bytes)?;
                placed_sources.insert(&locator.source, hash_hex);
                continue;
            }
            let bytes = match mfb_repository::client::fetch_blob(repo_url, &hash_hex) {
                Ok(bytes) => bytes,
                Err(err) => {
                    // `fetch_blob` re-hashes, so a substituted blob fails there with
                    // a distinct message; anything else is a missing blob.
                    if err.contains("does not match the requested content hash") {
                        crate::rules::show_general_diagnostic(
                            "PACKAGE_VENDOR_BLOB_HASH_MISMATCH",
                            &format!(
                                "the registry served a blob for \"{}\" (native library \"{}\") \
                                 whose contents do not match the sha256 `{hash_hex}` recorded in \
                                 `{package_name}`'s signed section-10 table.",
                                locator.source, entry.logical
                            ),
                        );
                        return Err(format!(
                            "vendor blob for `{package_name}` failed hash verification"
                        ));
                    }
                    crate::rules::show_general_diagnostic(
                        "PACKAGE_VENDOR_BLOB_MISSING",
                        &format!(
                            "the registry has no blob {hash_hex} for vendored native library \
                             \"{}\" (\"{}\") that `{package_name}` requires: {err}",
                            locator.source, entry.logical
                        ),
                    );
                    return Err(format!(
                        "vendor blob for `{package_name}` is missing from the registry"
                    ));
                }
            };
            let destination = super::install_vendor_file(&vendor_dir, &locator.source, &bytes)?;
            fetched.insert(hash_hex.clone(), destination);
            placed_sources.insert(&locator.source, hash_hex);
            downloaded += 1;
        }
    }
    if downloaded > 0 {
        println!(
            "Downloaded {downloaded} vendor {} for {package_name}",
            if downloaded == 1 {
                "library"
            } else {
                "libraries"
            }
        );
    }
    Ok(())
}

/// Whether a release state is eligible for a floating (non-pinned) install
/// (plan-10-C). `available`/`deprecated` are eligible; `yanked` is pin-only;
/// `blocked`/`legal-tombstoned` are never installed.
pub(crate) fn state_is_floating_eligible(state: &str) -> bool {
    matches!(state, "available" | "deprecated")
}

/// Choose the version to install from a registry index: an exact requested
/// version (any non-blocked state), else the newest floating-eligible one.
pub(super) fn select_index_version<'a>(
    index: &'a mfb_repository::server::IndexResponse,
    requested: Option<&str>,
) -> Result<&'a mfb_repository::server::IndexVersion, String> {
    if index.versions.is_empty() {
        return Err(format!(
            "registry has no published versions of `{}`",
            index.ident
        ));
    }
    if let Some(version) = requested {
        return index
            .versions
            .iter()
            .find(|entry| entry.version == version)
            .ok_or_else(|| format!("registry has no version `{version}` of `{}`", index.ident))
            .and_then(|entry| {
                if entry.state == "blocked" || entry.state == "legal-tombstoned" {
                    Err(format!(
                        "version `{version}` of `{}` is {} and cannot be installed",
                        index.ident, entry.state
                    ))
                } else {
                    Ok(entry)
                }
            });
    }
    index
        .versions
        .iter()
        .filter(|entry| state_is_floating_eligible(&entry.state))
        .max_by_key(|entry| entry.published_at)
        .ok_or_else(|| {
            format!(
                "registry has no install-eligible version of `{}` (all yanked or blocked)",
                index.ident
            )
        })
}
