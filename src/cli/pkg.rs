use std::fs;
use std::path::{Path, PathBuf};
use tinyjson::JsonValue;

use crate::binary_repr;
use crate::doc;
use crate::manifest::package::{
    package_file_url_path, project_json_with_package, project_package_dependency, read_mfp_header,
    ProjectPackageDependency,
};
use crate::manifest::{parse_project_json, project_kind, validate_packages_array, validate_project_manifest};
use crate::target;
use crate::USAGE;

use super::build::{build_project, BuildOptions};

pub(crate) enum PkgCommandError {
    Usage(String),
    Failed(String),
}

pub(crate) fn run_pkg_command(args: &[String]) -> Result<(), PkgCommandError> {
    match args {
        [command, url] if command == "add" => {
            add_package(Path::new("."), url).map_err(PkgCommandError::Failed)
        }
        [command, package] if command == "info" => {
            print_package_info(Path::new(package)).map_err(PkgCommandError::Failed)
        }
        [command, rest @ ..] if command == "doc" => run_pkg_doc(rest),
        [command] if command == "verify" => {
            verify_packages(Path::new("."), false).map_err(PkgCommandError::Failed)
        }
        [command, flag] if command == "verify" && flag == "--proof" => {
            verify_packages(Path::new("."), true).map_err(PkgCommandError::Failed)
        }
        [command, package] if command == "validate" => {
            validate_package_file(Path::new("."), package).map_err(PkgCommandError::Failed)
        }
        [command, ..] if command == "validate" => Err(PkgCommandError::Usage(format!(
            "mfb pkg validate requires exactly one <package>\n\n{USAGE}"
        ))),
        [command, owner, package] if command == "publish" => {
            publish_package_project(owner, Path::new(package)).map_err(PkgCommandError::Failed)
        }
        [command, ..] if command == "add" => Err(PkgCommandError::Usage(format!(
            "mfb pkg add requires exactly one <url>\n\n{USAGE}"
        ))),
        [command, ..] if command == "info" => Err(PkgCommandError::Usage(format!(
            "mfb pkg info requires exactly one <package>\n\n{USAGE}"
        ))),
        [command, ..] if command == "verify" => Err(PkgCommandError::Usage(format!(
            "mfb pkg verify accepts only the optional --proof flag\n\n{USAGE}"
        ))),
        [command, ..] if command == "publish" => Err(PkgCommandError::Usage(format!(
            "mfb pkg publish requires <owner_name> <package>\n\n{USAGE}"
        ))),
        [] => Err(PkgCommandError::Usage(format!(
            "mfb pkg requires a subcommand\n\n{USAGE}"
        ))),
        [command, ..] => Err(PkgCommandError::Usage(format!(
            "unknown pkg command `{command}`\n\n{USAGE}"
        ))),
    }
}

fn publish_package_project(owner: &str, project_dir: &Path) -> Result<(), String> {
    let project_path = project_dir.join("project.json");
    let manifest = validate_project_manifest(&project_path)
        .map_err(|_| "package project validation failed".to_string())?;
    if project_kind(&manifest) != "package" {
        return Err("mfb pkg publish requires a package project".to_string());
    }

    build_project(&BuildOptions {
        location: project_dir.to_path_buf(),
        outputs: Vec::new(),
        target: target::BuildTarget::host(),
        sign_owner: Some(owner.to_string()),
        app_mode: false,
        regalloc: target::shared::code::regalloc::active_kind(),
        allow_unsigned: false,
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
    mfb_repository::client::fetch_checkpoint(&repo_url, &paths).map_err(|err| {
        if err.contains("ROLLBACK") || err.contains("FORK") {
            crate::rules::show_general_diagnostic("REGISTRY_LOG_ROLLBACK", &err);
        }
        err
    })?;

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
    )?;
    println!(
        "Inclusion verified against checkpoint (size {}, root {})",
        checkpoint.size, checkpoint.root_hash
    );
    Ok(())
}

fn print_publish_verify_report(report: &mfb_repository::server::ValidatePackageResponse) {
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

/// `mfb pkg add <target>`: a `file://…​.mfp` URL copies a local package (the
/// original path, kept as a `source: "file:"` special case), while a
/// `<owner>#<package>[@version]` ident installs from the configured registry
/// (plan-10-A) — pinning the registry-vouched identKey, downloading the blob,
/// and verifying the full plan-23 §3.5 chain before it is installed.
fn add_package(project_dir: &Path, target: &str) -> Result<(), String> {
    if target.starts_with("file://") {
        add_package_from_file(project_dir, target)
    } else if target.contains('#') {
        add_package_from_registry(project_dir, target)
    } else {
        Err(format!(
            "mfb pkg add expects a file:// URL or an <owner>#<package>[@version] ident, got `{target}`"
        ))
    }
}

fn add_package_from_file(project_dir: &Path, url: &str) -> Result<(), String> {
    let source_path = package_file_url_path(url)?;
    let package = read_mfp_header(&source_path)?;

    let project_path = project_dir.join("project.json");
    let contents = fs::read_to_string(&project_path)
        .map_err(|err| format!("failed to read '{}': {err}", project_path.display()))?;
    let manifest = parse_project_json(&contents, &project_path)?;
    validate_packages_array(&manifest)?;

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
    };
    let updated = project_json_with_package(&contents, &manifest, &dependency)?;

    let packages_dir = project_dir.join("packages");
    fs::create_dir_all(&packages_dir)
        .map_err(|err| format!("failed to create '{}': {err}", packages_dir.display()))?;

    let destination = packages_dir.join(&package_filename);
    fs::copy(&source_path, &destination).map_err(|err| {
        format!(
            "failed to copy '{}' to '{}': {err}",
            source_path.display(),
            destination.display()
        )
    })?;

    fs::write(&project_path, updated)
        .map_err(|err| format!("failed to write '{}': {err}", project_path.display()))?;

    println!(
        "Added package {} {} to {}",
        package.name,
        package.version,
        project_path.display()
    );
    Ok(())
}

/// Install a package from the registry (plan-10-A): resolve `/index`, pin the
/// registry-vouched identKey, download `/blob/<hash>`, run the full §3.5
/// verification chain, and install into `packages/`.
fn add_package_from_registry(project_dir: &Path, target: &str) -> Result<(), String> {
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

    let packages_dir = project_dir.join("packages");
    fs::create_dir_all(&packages_dir)
        .map_err(|err| format!("failed to create '{}': {err}", packages_dir.display()))?;
    let destination = packages_dir.join(format!("{}.mfp", header.name));
    fs::write(&destination, &blob)
        .map_err(|err| format!("failed to write '{}': {err}", destination.display()))?;

    // Verify the full plan-23 §3.5 chain against the registry-vouched pin
    // (pinned server key → attestation → pinned ident → proof → package
    // signature → packageBinaryHash). Anything less than Verified is fatal.
    let classification =
        super::build::classify_installed_package(&destination, Some(&index.ident_key));
    if classification.state != super::build::PackageVerification::Verified {
        let _ = fs::remove_file(&destination);
        let detail = classification
            .refusal
            .map(|(_, detail)| detail)
            .unwrap_or_else(|| "downloaded package did not verify".to_string());
        return Err(format!(
            "refusing to add `{}`: {detail}",
            header.name
        ));
    }

    let dependency = ProjectPackageDependency {
        name: header.name.clone(),
        ident: full_ident.clone(),
        version: header.version.clone(),
        pin: true,
        source: full_ident,
        ident_key: index.ident_key.clone(),
    };
    let updated = project_json_with_package(&contents, &manifest, &dependency)?;
    fs::write(&project_path, updated)
        .map_err(|err| format!("failed to write '{}': {err}", project_path.display()))?;

    println!(
        "Added package {} {} from {} to {}",
        header.name,
        header.version,
        index.ident,
        project_path.display()
    );
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
fn select_index_version<'a>(
    index: &'a mfb_repository::server::IndexResponse,
    requested: Option<&str>,
) -> Result<&'a mfb_repository::server::IndexVersion, String> {
    if index.versions.is_empty() {
        return Err(format!("registry has no published versions of `{}`", index.ident));
    }
    if let Some(version) = requested {
        return index
            .versions
            .iter()
            .find(|entry| entry.version == version)
            .ok_or_else(|| {
                format!("registry has no version `{version}` of `{}`", index.ident)
            })
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

/// `mfb pkg validate <pkg>` (plan-23 index §10.4): validate an EXISTING
/// package file — "is this package correct?". Checks the container structure
/// and, for a signed package, every internally-checkable link of the §3.5
/// chain: the payload hash weld, the prefix signature under the embedded
/// signingKey, the proof under the embedded identKey, and the attestation
/// under the pinned registry key. When the working project declares the
/// package with a pinned identKey, the pin is checked too. This is not a
/// pre-signing step; nothing is uploaded.
fn validate_package_file(project_dir: &Path, target: &str) -> Result<(), String> {
    let direct = Path::new(target);
    let package_path = if target.ends_with(".mfp") || direct.is_file() {
        direct.to_path_buf()
    } else {
        let candidate = project_dir.join("packages").join(format!("{target}.mfp"));
        if candidate.is_file() {
            candidate
        } else {
            return Err(format!(
                "no package named `{target}` found (looked for '{}')",
                candidate.display()
            ));
        }
    };

    let bytes = fs::read(&package_path)
        .map_err(|err| format!("failed to read '{}': {err}", package_path.display()))?;
    println!("Package validation report for {}:", package_path.display());

    let mut failures = 0usize;
    let mut check = |name: &str, result: Result<String, String>| match result {
        Ok(note) if note.is_empty() => println!("  {name}: OK"),
        Ok(note) => println!("  {name}: OK ({note})"),
        Err(err) => {
            println!("  {name}: FAILED ({err})");
            failures += 1;
        }
    };

    let package = match mfb_repository::package::parse_mfp_package(&bytes) {
        Ok(package) => {
            check("container", Ok("v1.0".to_string()));
            package
        }
        Err(err) => {
            check("container", Err(err));
            return Err("package validation failed".to_string());
        }
    };
    println!("  ident: {}", package.ident);
    println!("  version: {}", package.version);
    println!(
        "  signature type: {}",
        signature_type_name(package.signature_type)
    );

    check(
        "payload hash",
        mfb_repository::package::verify_payload_hash(&package).map(|()| String::new()),
    );

    if package.signature_type == 0 {
        println!("  trust chain: <none> (unsigned package)");
    } else {
        check(
            "package signature",
            mfb_repository::package::verify_package_signature(&package).and_then(|()| {
                Ok(format!(
                    "signingKey {}",
                    package.signing_fingerprint()?
                ))
            }),
        );
        check(
            "proof",
            mfb_repository::package::decode_metadata_key(&package.ident_key, "identKey")
                .and_then(|ident_public| {
                    mfb_repository::package::verify_proof(&package, &ident_public)
                })
                .and_then(|()| Ok(format!("identKey {}", package.ident_fingerprint()?))),
        );

        // The attestation needs the pinned registry key (plan-23 §3.5 step 2).
        let repo_url = mfb_repository::client::repo_url_from_env();
        let attestation = super::local_paths_for_repo(&repo_url)
            .and_then(|paths| mfb_repository::local::read_pinned_server_key(&paths))
            .map_err(|_| {
                "no pinned registry key; run `mfb repo auth <owner>` against the registry to pin server.pub"
                    .to_string()
            })
            .and_then(|server_key| {
                let repo_fingerprint = mfb_repository::crypto::fingerprint(&server_key);
                mfb_repository::package::verify_attestation(
                    &package,
                    &server_key,
                    &repo_fingerprint,
                )
                .map(|()| format!("repoFingerprint {repo_fingerprint}"))
            });
        check("attestation", attestation);

        // The pin check runs when the working project declares this package.
        let pin = project_pinned_ident_key(project_dir, &package.name);
        match pin {
            Some(anchor) => {
                let result = mfb_repository::package::decode_metadata_key(&anchor, "identKey")
                    .and_then(|pinned| {
                        let header = mfb_repository::package::decode_metadata_key(
                            &package.ident_key,
                            "identKey",
                        )?;
                        if header == pinned {
                            Ok(String::new())
                        } else {
                            Err(
                                "package identKey does not match the identKey pinned in project.json"
                                    .to_string(),
                            )
                        }
                    });
                check("ident pin", result);
            }
            None => println!("  ident pin: <not declared in project.json>"),
        }
    }

    if failures == 0 {
        println!("  result: valid");
        Ok(())
    } else {
        println!("  result: INVALID ({failures} failed check(s))");
        Err("package validation failed".to_string())
    }
}

/// The `identKey` pinned for `name` in the working project's manifest, if the
/// project declares that dependency.
fn project_pinned_ident_key(project_dir: &Path, name: &str) -> Option<String> {
    let contents = fs::read_to_string(project_dir.join("project.json")).ok()?;
    let manifest = parse_project_json(&contents, &project_dir.join("project.json")).ok()?;
    let packages = manifest
        .get("packages")
        .and_then(|value| value.get::<Vec<JsonValue>>())?;
    packages.iter().find_map(|entry| {
        let object = entry.get::<std::collections::HashMap<String, JsonValue>>()?;
        if object.get("name")?.get::<String>()? != name {
            return None;
        }
        object
            .get("identKey")
            .or_else(|| object.get("ident_key"))
            .and_then(|value| value.get::<String>())
            .cloned()
    })
}

fn verify_packages(project_dir: &Path, demand_proof: bool) -> Result<(), String> {
    let project_path = project_dir.join("project.json");
    let contents = fs::read_to_string(&project_path)
        .map_err(|err| format!("failed to read '{}': {err}", project_path.display()))?;
    let manifest = parse_project_json(&contents, &project_path)?;
    validate_packages_array(&manifest)?;

    let Some(packages) = manifest
        .get("packages")
        .and_then(|value| value.get::<Vec<JsonValue>>())
    else {
        return Ok(());
    };

    let mut rotation_errors = Vec::new();
    for package in packages {
        let Some(dependency) = project_package_dependency(package) else {
            println!("<invalid> @ <invalid> : Invalid Package");
            continue;
        };
        let result = verify_package_dependency(project_dir, &dependency);
        // Compiled dependencies also get their plan-23 §3.5 trust state,
        // verified against the pinned identKey (never the embedded key).
        let package_file = project_dir
            .join("packages")
            .join(format!("{}.mfp", dependency.name));
        let state = if package_file.is_file() {
            let mut anchor = if dependency.ident_key.is_empty() {
                None
            } else {
                Some(dependency.ident_key.clone())
            };
            // Pin-follow (plan-23-B2): an installed package naming a NEWER
            // CHAINED ident updates the pin automatically with a notice; an
            // ident change with no chain link is a hard error (re-anchor).
            if let Some(pinned) = anchor.clone() {
                match follow_rotated_pin(project_dir, &dependency, &pinned, &package_file) {
                    Ok(Some(new_pin)) => anchor = Some(new_pin),
                    Ok(None) => {}
                    Err((rule, err)) => rotation_errors.push((rule, err)),
                }
            }
            let classification =
                super::build::classify_installed_package(&package_file, anchor.as_deref());
            // `--proof` (plan-23-B3): additionally demand a transparency-log
            // inclusion proof for the package's publish entry, verified
            // against the signed, rollback-checked checkpoint.
            let mut suffix = format!(" [{}]", classification.state.label());
            if demand_proof
                && classification.state == super::build::PackageVerification::Verified
                && dependency.ident.contains('#')
            {
                let repo_url = mfb_repository::client::repo_url_from_env();
                let version = read_mfp_header(&package_file)
                    .map(|header| header.version)
                    .unwrap_or_default();
                match super::local_paths_for_repo(&repo_url).and_then(|paths| {
                    mfb_repository::client::verify_publish_inclusion(
                        &repo_url,
                        &paths,
                        &dependency.ident,
                        &version,
                    )
                }) {
                    Ok((entry, checkpoint)) => {
                        suffix.push_str(&format!(
                            " (log index {} ⊂ checkpoint size {})",
                            entry.index, checkpoint.size
                        ));
                    }
                    Err(err) => {
                        rotation_errors.push((
                            "PACKAGE_ATTESTATION_INVALID",
                            format!(
                                "package `{}` has no verifiable publish log entry: {err}",
                                dependency.name
                            ),
                        ));
                        suffix.push_str(" (no publish proof)");
                    }
                }
            }
            suffix
        } else {
            String::new()
        };
        println!("{}{state}", package_verify_line(&dependency, &result));
    }

    if rotation_errors.is_empty() {
        Ok(())
    } else {
        for (rule, detail) in &rotation_errors {
            crate::rules::show_general_diagnostic(rule, detail);
        }
        Err("package identity verification failed".to_string())
    }
}

/// When the installed package's identKey differs from the pin, consult the
/// registry's signed rotation chain. A verifiable chain from the pin to the
/// package's key updates `project.json` (with a notice) and returns the new
/// pin; a missing chain is the re-anchor case and errors loudly.
fn follow_rotated_pin(
    project_dir: &Path,
    dependency: &ProjectPackageDependency,
    pinned: &str,
    package_file: &Path,
) -> Result<Option<String>, (&'static str, String)> {
    let untrusted = |detail: String| ("PACKAGE_IDENT_KEY_UNTRUSTED", detail);
    let header = read_mfp_header(package_file).map_err(|err| untrusted(err))?;
    if header.signature_type == 0 || header.ident_key.is_empty() {
        return Ok(None);
    }
    let pinned_raw = mfb_repository::package::decode_metadata_key(pinned, "identKey")
        .map_err(untrusted)?;
    let header_raw = mfb_repository::package::decode_metadata_key(&header.ident_key, "identKey")
        .map_err(untrusted)?;
    if pinned_raw == header_raw {
        return Ok(None);
    }
    let Some((owner, _)) = dependency.ident.split_once('#') else {
        return Ok(None);
    };
    let repo_url = mfb_repository::client::repo_url_from_env();
    let chain = mfb_repository::client::fetch_ident_chain(&repo_url, owner).map_err(|err| {
        untrusted(format!(
            "package `{}` is signed by a different ident than the pinned key and the registry \
             chain could not be fetched: {err}",
            dependency.name
        ))
    })?;
    match mfb_repository::client::follow_ident_chain(owner, &pinned_raw, &chain.chain)
        .map_err(untrusted)?
    {
        Some(newest) if newest == header_raw => {
            let new_pin = format!(
                "ed25519:{}",
                mfb_repository::crypto::encode_bytes(&newest)
            );
            let project_path = project_dir.join("project.json");
            let contents = fs::read_to_string(&project_path).map_err(|err| {
                untrusted(format!("failed to read '{}': {err}", project_path.display()))
            })?;
            let updated = crate::manifest::package::project_json_with_updated_ident_key(
                &contents,
                &dependency.name,
                &new_pin,
            )
            .map_err(untrusted)?;
            fs::write(&project_path, updated).map_err(|err| {
                untrusted(format!("failed to write '{}': {err}", project_path.display()))
            })?;
            println!(
                "notice: owner `{owner}` rotated their ident; updated the pinned identKey for `{}` to fingerprint {}",
                dependency.name,
                mfb_repository::crypto::fingerprint(&newest),
            );
            Ok(Some(new_pin))
        }
        Some(_other) => Err(untrusted(format!(
            "package `{}` is signed by an ident that is neither the pinned key nor its chained successor",
            dependency.name
        ))),
        None => Err((
            "PACKAGE_IDENT_REANCHORED",
            format!(
                "owner `{owner}`'s ident changed with NO chain link from your pinned key \
                 (a re-anchor or an impersonation). Verify the owner's new identity out-of-band \
                 before re-adding `{}`; the pin was NOT updated.",
                dependency.name
            ),
        )),
    }
}

/// `mfb pkg doc <name-or-path> [--out <file>]` — render HTML from a compiled
/// package's doc section (plan-09-doc.md §6.2).
fn run_pkg_doc(args: &[String]) -> Result<(), PkgCommandError> {
    let mut target: Option<String> = None;
    let mut out: Option<String> = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--out" => {
                index += 1;
                let file = args.get(index).ok_or_else(|| {
                    PkgCommandError::Usage("mfb pkg doc --out requires a file".to_string())
                })?;
                out = Some(file.clone());
            }
            flag if flag.starts_with("--") => {
                return Err(PkgCommandError::Usage(format!("unknown flag `{flag}`")));
            }
            value => {
                if target.is_some() {
                    return Err(PkgCommandError::Usage(
                        "mfb pkg doc accepts exactly one <name-or-path>".to_string(),
                    ));
                }
                target = Some(value.to_string());
            }
        }
        index += 1;
    }
    let target = target.ok_or_else(|| {
        PkgCommandError::Usage(format!("mfb pkg doc requires <name-or-path>\n\n{USAGE}"))
    })?;
    let out_path = out
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("doc.html"));
    write_package_doc(&target, &out_path).map_err(PkgCommandError::Failed)
}

/// Resolve a package name or path to its `.mfp`, decode the doc section, and
/// render it. A package with no doc section yields a minimal page (exit 0).
fn write_package_doc(target: &str, out_path: &Path) -> Result<(), String> {
    let direct = Path::new(target);
    let package_path = if target.ends_with(".mfp") || direct.is_file() {
        direct.to_path_buf()
    } else {
        let candidate = Path::new("packages").join(format!("{target}.mfp"));
        if candidate.is_file() {
            candidate
        } else {
            return Err(format!(
                "no package named `{target}` found (looked for '{}')",
                candidate.display()
            ));
        }
    };

    let header = read_mfp_header(&package_path)?;
    let docs = binary_repr::read_package_docs(&package_path)?;
    let html = if docs.is_empty() {
        doc::render_empty_html(&header.name)
    } else {
        let page = doc::from_package(docs, &header.name);
        doc::render_html(&page)
    };
    fs::write(out_path, html)
        .map_err(|err| format!("failed to write '{}': {err}", out_path.display()))?;
    println!("Wrote documentation to {}", out_path.display());
    Ok(())
}

fn print_package_info(path: &Path) -> Result<(), String> {
    let package_bytes =
        fs::read(path).map_err(|err| format!("failed to read '{}': {err}", path.display()))?;
    let content_hash = target::package_mfp::package_content_hash(&package_bytes)?;
    let header = read_mfp_header(path)?;
    let info = binary_repr::read_package_info(path)?;

    println!("Package: {}", header.name);
    println!("Ident: {}", empty_marker(&header.ident));
    println!("Version: {}", header.version);
    println!("Ident Key: {}", empty_marker(&header.ident_key));
    println!("Signing Key: {}", empty_marker(&header.signing_key));
    println!(
        "Proof: {}",
        if header.proof.is_empty() {
            "<none>"
        } else {
            &header.proof
        }
    );
    println!(
        "Attestation: {}",
        if header.attestation.is_empty() {
            "<none>"
        } else {
            &header.attestation
        }
    );
    println!("Author: {}", empty_marker(&header.author));
    println!("URL: {}", empty_marker(&header.url));
    println!("Path: {}", path.display());
    println!();
    println!("Container:");
    println!("  format: MFP");
    println!(
        "  version: {}.{}",
        header.container_major, header.container_minor
    );
    println!(
        "  binary representation version: {}.{}",
        header.binary_repr_major, header.binary_repr_minor
    );
    println!("  flags: 0x{:08x}", header.flags);
    println!(
        "  signature type: {}",
        signature_type_name(header.signature_type)
    );
    println!("  signature length: {}", header.signature_length);
    println!(
        "  package binary hash: {}",
        hex_bytes(&header.package_binary_hash)
    );
    println!("  content hash: {}", hex_bytes(&content_hash));
    println!(
        "  binary representation length: {}",
        header.binary_repr_length
    );
    println!();
    println!("Manifest:");
    println!("  name: {}", info.manifest_name);
    println!("  ident: {}", empty_marker(&info.manifest_ident));
    println!("  version: {}", info.manifest_version);
    println!("  ident key: {}", empty_marker(&info.manifest_ident_key));
    println!(
        "  ident fingerprint: {}",
        empty_marker(&info.manifest_ident_fingerprint)
    );
    println!(
        "  signing fingerprint: {}",
        empty_marker(&info.manifest_signing_fingerprint)
    );
    println!("  author: {}", empty_marker(&info.author));
    println!("  url: {}", empty_marker(&info.url));
    println!();
    println!("Binary Representation:");
    println!("  ABI format version: {}", info.abi_format_version);
    println!("  types: {}", info.type_count);
    println!("  constants: {}", info.const_count);
    println!("  resources: {}", info.resource_count);
    println!("  functions: {}", info.function_count);
    println!("  globals: {}", info.global_count);
    println!("  cleanups: {}", info.cleanup_count);
    println!("  imports: {}", info.import_count);
    println!("  exports: {}", info.export_count);
    println!();
    println!("Exports:");
    if info.exports.is_empty() {
        println!("  <none>");
    } else {
        for export in &info.exports {
            println!(
                "  {} {}",
                package_export_kind_name(export.kind),
                export.name
            );
            println!("    sigHash: {}", export.sig_hash);
        }
    }
    println!();
    println!("Package State:");
    if info.globals.is_empty() {
        println!("  <none>");
    } else {
        for global in &info.globals {
            println!(
                "  {} {} AS {}",
                if global.mutable { "MUT" } else { "LET" },
                global.name,
                global.type_
            );
            println!("    visibility: {}", global.visibility);
            if global.mutable && global.visibility == "export" {
                println!("    audit: exported mutable package state");
            }
        }
    }
    println!();
    println!("Resource Cleanups:");
    if info.cleanups.is_empty() {
        println!("  <none>");
    } else {
        for cleanup in &info.cleanups {
            println!("  {} cleanup {}", cleanup.function, cleanup.cleanup_id);
            println!("    pc: {}..{}", cleanup.start_pc, cleanup.end_pc);
            println!("    resource register: {}", cleanup.resource_register);
            println!("    close function id: {}", cleanup.close_function_id);
            if cleanup.records_secondary_close_failure {
                println!("    audit: records secondary close failure");
            }
        }
    }
    println!();
    println!("Imports:");
    if info.imports.is_empty() {
        println!("  <none>");
    } else {
        for import in &info.imports {
            println!("  {}", import.package_name);
            println!("    ident: {}", empty_marker(&import.package_ident));
            println!("    version: {}", empty_marker(&import.version));
            println!("    pin: {}", import.pin);
            println!("    flags: 0x{:08x}", import.flags);
            if import.used_symbols.is_empty() {
                println!("    used symbols: <none>");
            } else {
                println!("    used symbols:");
                for symbol in &import.used_symbols {
                    println!("      {}", symbol.name);
                    println!("        sigHash: {}", symbol.sig_hash);
                }
            }
        }
    }

    Ok(())
}

pub(crate) fn signature_type_name(signature_type: u16) -> String {
    match signature_type {
        0 => "unsigned".to_string(),
        1 => "Ed25519".to_string(),
        other => format!("unknown ({other})"),
    }
}

pub(crate) fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn package_export_kind_name(kind: binary_repr::BinaryReprExportKind) -> &'static str {
    match kind {
        binary_repr::BinaryReprExportKind::Func => "FUNC",
        binary_repr::BinaryReprExportKind::Sub => "SUB",
        binary_repr::BinaryReprExportKind::Type => "TYPE",
        binary_repr::BinaryReprExportKind::Union => "UNION",
        binary_repr::BinaryReprExportKind::Enum => "ENUM",
    }
}

fn empty_marker(value: &str) -> &str {
    if value.is_empty() {
        "<empty>"
    } else {
        value
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum PackageVerifyStatus {
    Ok,
    NeedsUpdate,
    InvalidPackage,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PackageVerifyResult {
    pub(crate) version: String,
    pub(crate) status: PackageVerifyStatus,
}

impl PackageVerifyStatus {
    fn message(&self) -> &'static str {
        match self {
            PackageVerifyStatus::Ok => "OK",
            PackageVerifyStatus::NeedsUpdate => "Needs Update",
            PackageVerifyStatus::InvalidPackage => "Invalid Package",
        }
    }
}

pub(crate) fn package_verify_line(
    dependency: &ProjectPackageDependency,
    result: &PackageVerifyResult,
) -> String {
    if result.version.is_empty() {
        format!(
            "{} @ {} : {}",
            dependency.name,
            dependency.version,
            result.status.message()
        )
    } else {
        format!(
            "{} @ {} : {} ({})",
            dependency.name,
            dependency.version,
            result.status.message(),
            result.version
        )
    }
}

pub(crate) fn verify_package_dependency(
    project_dir: &Path,
    dependency: &ProjectPackageDependency,
) -> PackageVerifyResult {
    let package_file = project_dir
        .join("packages")
        .join(format!("{}.mfp", dependency.name));
    if package_file.is_file() {
        return match read_mfp_header(&package_file) {
            Ok(header) => PackageVerifyResult {
                version: header.version.clone(),
                status: package_dependency_status(
                    dependency,
                    &header.name,
                    &header.ident,
                    &header.version,
                ),
            },
            Err(_) => PackageVerifyResult {
                version: String::new(),
                status: PackageVerifyStatus::InvalidPackage,
            },
        };
    }

    let package_manifest = project_dir
        .join("packages")
        .join(&dependency.name)
        .join("project.json");
    if package_manifest.is_file() {
        return verify_source_package_manifest(&package_manifest, dependency);
    }

    PackageVerifyResult {
        version: String::new(),
        status: PackageVerifyStatus::InvalidPackage,
    }
}

fn verify_source_package_manifest(
    package_manifest: &Path,
    dependency: &ProjectPackageDependency,
) -> PackageVerifyResult {
    let Ok(contents) = fs::read_to_string(package_manifest) else {
        return PackageVerifyResult {
            version: String::new(),
            status: PackageVerifyStatus::InvalidPackage,
        };
    };
    let Ok(manifest) = parse_project_json(&contents, package_manifest) else {
        return PackageVerifyResult {
            version: String::new(),
            status: PackageVerifyStatus::InvalidPackage,
        };
    };
    let Some(actual_name) = manifest.get("name").and_then(|value| value.get::<String>()) else {
        return PackageVerifyResult {
            version: String::new(),
            status: PackageVerifyStatus::InvalidPackage,
        };
    };
    let Some(actual_version) = manifest
        .get("version")
        .and_then(|value| value.get::<String>())
    else {
        return PackageVerifyResult {
            version: String::new(),
            status: PackageVerifyStatus::InvalidPackage,
        };
    };
    let actual_ident = manifest
        .get("ident")
        .and_then(|value| value.get::<String>())
        .map(String::as_str)
        .unwrap_or(actual_name);

    PackageVerifyResult {
        version: actual_version.clone(),
        status: package_dependency_status(dependency, actual_name, actual_ident, actual_version),
    }
}

pub(crate) fn package_dependency_status(
    dependency: &ProjectPackageDependency,
    actual_name: &str,
    actual_ident: &str,
    actual_version: &str,
) -> PackageVerifyStatus {
    if dependency.name != actual_name {
        return PackageVerifyStatus::InvalidPackage;
    }
    if !dependency.ident.is_empty() && !actual_ident.is_empty() && dependency.ident != actual_ident
    {
        return PackageVerifyStatus::InvalidPackage;
    }
    if dependency.pin {
        package_version_status(&dependency.version, actual_version)
    } else {
        package_version_status(&dependency.version, actual_version)
    }
}

fn package_version_status(expected_version: &str, actual_version: &str) -> PackageVerifyStatus {
    if package_version_matches(expected_version, actual_version) {
        PackageVerifyStatus::Ok
    } else {
        PackageVerifyStatus::NeedsUpdate
    }
}

pub(crate) fn package_version_matches(expected: &str, actual: &str) -> bool {
    expected.is_empty() || expected == actual
}
