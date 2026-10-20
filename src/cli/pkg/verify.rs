//! plan-176-L: `mfb pkg` — the verify commands. The dispatcher `run_pkg_command` and the
//! shared `use` header live in `cli/pkg/mod.rs`.

use super::*;

/// `mfb pkg validate <pkg>` (plan-23 index §10.4): validate an EXISTING
/// package file — "is this package correct?". Checks the container structure
/// and, for a signed package, every internally-checkable link of the §3.5
/// chain: the payload hash weld, the prefix signature under the embedded
/// signingKey, the proof under the embedded identKey, and the attestation
/// under the pinned registry key. When the working project declares the
/// package with a pinned identKey, the pin is checked too. This is not a
/// pre-signing step; nothing is uploaded.
pub(super) fn validate_package_file(project_dir: &Path, target: &str) -> Result<(), String> {
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
    // bug-489: these come from the decoded `.mfp`, the same untrusted source
    // bug-210 covers at the `pkg info` print sites — this pair was missed.
    println!("  ident: {}", crate::terminal_safe::safe(&package.ident));
    println!(
        "  version: {}",
        crate::terminal_safe::safe(&package.version)
    );
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
            mfb_repository::package::verify_package_signature(&package)
                .and_then(|()| Ok(format!("signingKey {}", package.signing_fingerprint()?))),
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
pub(super) fn project_pinned_ident_key(project_dir: &Path, name: &str) -> Option<String> {
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

pub(super) fn verify_packages(project_dir: &Path, demand_proof: bool) -> Result<(), String> {
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
            // Pin-follow (plan-23-B2, plan-168-C): an installed package signed
            // by a key the pin reaches — through the owner's rotation chain or a
            // signed ownership transfer — updates the pin automatically with a
            // notice; an ident change with no chain link is a hard error
            // (re-anchor).
            if let Some(pinned) = anchor.clone() {
                match follow_identity_pin(project_dir, &dependency, &pinned, &package_file) {
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
                // bug-273: the log leaf is bound to `(ident, version, contentHash)`,
                // so the hash has to come from the installed file, not from the
                // registry — otherwise the server still picks what the leaf
                // describes. `install_verified_package` stages the downloaded blob
                // verbatim and renames it, and `fetch_blob` re-hashes against the
                // content address, so this digest is exactly the published one.
                // Streamed from disk because a `packages/*.mfp` is untrusted input
                // of arbitrary size.
                match target::package_mfp::package_content_hash_file(&package_file)
                    .map(|hash| hex_bytes(&hash))
                    .and_then(|content_hash| {
                        super::local_paths_for_repo(&repo_url).and_then(|paths| {
                            mfb_repository::client::verify_publish_inclusion(
                                &repo_url,
                                &paths,
                                &dependency.ident,
                                &version,
                                &content_hash,
                            )
                        })
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

    for (rule, detail) in &rotation_errors {
        crate::rules::show_general_diagnostic(rule, detail);
    }
    let closure_ok = report_dependency_closure(project_dir, &manifest)?;
    if !rotation_errors.is_empty() {
        Err("package identity verification failed".to_string())
    } else if !closure_ok {
        Err("package dependency verification failed".to_string())
    } else {
        Ok(())
    }
}

/// bug-628: report, in full, every way `packages[]` disagrees with the packages'
/// import tables and every declared package that does not provide what an
/// importer was compiled against — the detail `mfb build` points here for.
/// Returns whether there was nothing to report.
fn report_dependency_closure(
    project_dir: &Path,
    manifest: &std::collections::HashMap<String, JsonValue>,
) -> Result<bool, String> {
    let issues = crate::manifest::closure::check(project_dir, manifest)?;
    for issue in &issues {
        crate::rules::show_general_diagnostic(
            "PACKAGE_DEPENDENCIES_INCONSISTENT",
            &format!(
                "{}; run `mfb pkg update` to rewrite project.json",
                issue.message()
            ),
        );
    }
    let conflicts = crate::manifest::closure::conflicts(project_dir, manifest);
    for conflict in &conflicts {
        crate::rules::show_general_diagnostic("PACKAGE_VERSION_CONFLICT", &conflict.message());
        for detail in conflict.details() {
            eprintln!("                 {detail}");
        }
    }
    Ok(issues.is_empty() && conflicts.is_empty())
}

/// When the installed package's identKey differs from the pin, find out
/// whether the pin reaches it (plan-168-C). A release the registry lists is
/// verified through its signed transfer chain from the pin — the owner's
/// rotations and any ownership transfer — and updates `project.json` with a
/// notice. A release the registry does not list (a locally built package)
/// consults the pinned owner's signed rotation chain alone, as before
/// transfers existed. A missing chain is the re-anchor case and errors loudly.
// coverage:off — the interesting branches require a live registry (the index
// and the ident chain) reached only after a real key rotation or transfer;
// covered by the tests/ ident-rotation and transfer integration harnesses.
fn follow_identity_pin(
    project_dir: &Path,
    dependency: &ProjectPackageDependency,
    pinned: &str,
    package_file: &Path,
) -> Result<Option<String>, (&'static str, String)> {
    let untrusted = |detail: String| ("PACKAGE_IDENT_KEY_UNTRUSTED", detail);
    let header = read_mfp_header(package_file).map_err(&untrusted)?;
    if header.signature_type == 0 || header.ident_key.is_empty() {
        return Ok(None);
    }
    let pinned_raw =
        mfb_repository::package::decode_metadata_key(pinned, "identKey").map_err(untrusted)?;
    let header_raw = mfb_repository::package::decode_metadata_key(&header.ident_key, "identKey")
        .map_err(untrusted)?;
    if pinned_raw == header_raw {
        return Ok(None);
    }
    if let Some(followed) = follow_listed_release(dependency, &header, &pinned_raw, package_file)? {
        let new_pin = metadata_key(&followed.key);
        rewrite_pinned_ident_key(project_dir, &dependency.name, &new_pin).map_err(untrusted)?;
        if followed.crossed_transfer {
            print_transfer_notice(&dependency.ident, &followed.publisher);
        } else {
            println!(
                "notice: owner `{}` rotated their ident; updated the pinned identKey for `{}` to fingerprint {}",
                crate::terminal_safe::safe(&followed.publisher),
                dependency.name,
                mfb_repository::crypto::fingerprint(&followed.key),
            );
        }
        return Ok(Some(new_pin));
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
            let new_pin = metadata_key(&newest);
            rewrite_pinned_ident_key(project_dir, &dependency.name, &new_pin)
                .map_err(untrusted)?;
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

/// The registry's verdict on an installed release it lists, from `pinned`
/// through the signed transfer chain (plan-168-C). `Ok(None)` when the registry
/// does not list this exact release (a local build, or an unreachable or
/// unverifiable index), which leaves the rotation-only follow to decide.
fn follow_listed_release(
    dependency: &ProjectPackageDependency,
    header: &MfpHeader,
    pinned: &[u8],
    package_file: &Path,
) -> Result<Option<mfb_repository::trust::Trusted>, (&'static str, String)> {
    let Some((owner, package)) = dependency.ident.split_once('#') else {
        return Ok(None);
    };
    let repo_url = mfb_repository::client::repo_url_from_env();
    let Ok(paths) = super::local_paths_for_repo(&repo_url) else {
        return Ok(None);
    };
    let Ok(index) = mfb_repository::client::fetch_index(&repo_url, &paths, owner, package) else {
        return Ok(None);
    };
    let Ok(content_hash) = target::package_mfp::package_content_hash_file(package_file) else {
        return Ok(None);
    };
    let content_hash = hex_bytes(&content_hash);
    let Some(version) = index
        .versions
        .iter()
        .find(|version| version.version == header.version && version.hash == content_hash)
    else {
        return Ok(None);
    };
    let blob = fs::read(package_file).map_err(|err| {
        (
            "PACKAGE_IDENT_KEY_UNTRUSTED",
            format!("failed to read '{}': {err}", package_file.display()),
        )
    })?;
    verify_registry_release(
        &repo_url,
        &paths,
        &index,
        version,
        &blob,
        mfb_repository::trust::Anchor::Pinned(pinned.to_vec()),
    )
    .map_err(|err| {
        (
            err.rule(),
            format!(
                "package `{}` is signed by an ident key the pinned key does not reach: {err}",
                dependency.name
            ),
        )
    })
}

/// Point `name`'s pinned `identKey` in `project.json` at `new_pin`.
fn rewrite_pinned_ident_key(project_dir: &Path, name: &str, new_pin: &str) -> Result<(), String> {
    let project_path = project_dir.join("project.json");
    let contents = fs::read_to_string(&project_path)
        .map_err(|err| format!("failed to read '{}': {err}", project_path.display()))?;
    let updated =
        crate::manifest::package::project_json_with_updated_ident_key(&contents, name, new_pin)?;
    fs::write(&project_path, updated)
        .map_err(|err| format!("failed to write '{}': {err}", project_path.display()))
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
    pub(super) fn message(&self) -> &'static str {
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
        // A pin demands exactly the version the manifest names — including
        // refusing an empty pinned version, which can never equal an installed
        // one. This mirrors the gate builds enforce in `installed_package_files`.
        if dependency.version == actual_version {
            PackageVerifyStatus::Ok
        } else {
            PackageVerifyStatus::NeedsUpdate
        }
    } else {
        package_version_status(&dependency.version, actual_version)
    }
}

/// The status of an *unpinned* dependency, whose empty `expected_version` means
/// "any version".
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
