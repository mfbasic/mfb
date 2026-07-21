//! `mfb pkg` consumer-side package commands, plus the *implementations* of the
//! publisher-side commands.
//!
//! Note the deliberate split (plan-60-A §4.1): the five publisher-side commands
//! — `publish`, `check-abi`, `release-state`, `transfer`, `transfer-accept` —
//! are **dispatched** from `mfb repo` in `super::repo`, but their
//! implementations stay here, next to the `pkg.rs`-private helpers they use
//! (`install_vendor_blobs`, `hex_bytes`). Only the command surface moved; file
//! organization is a separate concern from which word invokes a command.

use std::fs;
use std::path::{Path, PathBuf};
use tinyjson::JsonValue;

use crate::binary_repr;
use crate::doc;
use crate::manifest::package::{
    package_file_url_path, project_json_with_package, project_package_dependency, read_mfp_header,
    ProjectPackageDependency,
};
use crate::manifest::{
    parse_project_json, project_kind, validate_packages_array, validate_project_manifest,
};
use crate::target;
use crate::PKG_HELP;

use super::repo::REPO_HELP_HINT;

use super::build::{build_project, BuildOptions, Verbosity};

pub(crate) enum PkgCommandError {
    Usage(String),
    Failed(String),
}

pub(crate) fn run_pkg_command(args: &[String]) -> Result<(), PkgCommandError> {
    match args {
        [command, rest @ ..] if command == "add" => {
            let options = parse_add_options(rest)?;
            add_package(Path::new("."), &options).map_err(PkgCommandError::Failed)
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
        [command] if command == "install" => {
            super::resolve::install(Path::new(".")).map_err(PkgCommandError::Failed)
        }
        [command, location] if command == "install" => {
            super::resolve::install(Path::new(location)).map_err(PkgCommandError::Failed)
        }
        [command, ..] if command == "install" => Err(PkgCommandError::Usage(format!(
            "mfb pkg install accepts at most one [location]\n\n{PKG_HELP}"
        ))),
        [command] if command == "update" => {
            super::resolve::update(Path::new(".")).map_err(PkgCommandError::Failed)
        }
        [command, location] if command == "update" => {
            super::resolve::update(Path::new(location)).map_err(PkgCommandError::Failed)
        }
        [command, ..] if command == "update" => Err(PkgCommandError::Usage(format!(
            "mfb pkg update accepts at most one [location]\n\n{PKG_HELP}"
        ))),
        [command, ..] if command == "validate" => Err(PkgCommandError::Usage(format!(
            "mfb pkg validate requires exactly one <package>\n\n{PKG_HELP}"
        ))),
        [command, ..] if command == "add" => Err(PkgCommandError::Usage(format!(
            "mfb pkg add requires exactly one <url>\n\n{PKG_HELP}"
        ))),
        [command, ..] if command == "info" => Err(PkgCommandError::Usage(format!(
            "mfb pkg info requires exactly one <package>\n\n{PKG_HELP}"
        ))),
        [command, ..] if command == "verify" => Err(PkgCommandError::Usage(format!(
            "mfb pkg verify accepts only the optional --proof flag\n\n{PKG_HELP}"
        ))),
        [] => Err(PkgCommandError::Usage(format!(
            "mfb pkg requires a subcommand\n\n{PKG_HELP}"
        ))),
        // The five publisher-side commands moved to `mfb repo` (plan-60-A).
        // Name the new location rather than falling through to a bare "unknown
        // pkg command": this is not an alias — it still exits 2 and does
        // nothing — but it is the difference between a five-second fix and a
        // grep through the help text.
        [command, ..]
            if matches!(
                command.as_str(),
                "publish" | "check-abi" | "release-state" | "transfer" | "transfer-accept"
            ) =>
        {
            Err(PkgCommandError::Usage(format!(
                "mfb pkg {command} has moved to mfb repo {command}\n\n{REPO_HELP_HINT}"
            )))
        }
        [command, ..] => Err(PkgCommandError::Usage(format!(
            "unknown pkg command `{command}`\n\n{PKG_HELP}"
        ))),
    }
}

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
        target: target::BuildTarget::host(),
        sign_owner: Some(owner.to_string()),
        app_mode: false,
        app_debug: false,
        regalloc: target::shared::code::regalloc::active_kind(),
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
    // `verify_log_consistency` rather than `fetch_checkpoint` (bug-276 R2):
    // fetch_checkpoint only enforces monotonicity, so a fork that simply grows
    // passes it. Demanding the RFC-6962 consistency proof against the pinned head
    // is what actually establishes the new head extends the history this client
    // already saw.
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

/// `mfb repo transfer <owner>#<package> <to-owner>` (plan-10-D1): the current
/// owner offers a package to a recipient (signed with the local ident key).
pub(crate) fn transfer_offer(ident: &str, to_owner: &str) -> Result<(), String> {
    let Some((from_owner, _)) = ident.split_once('#') else {
        return Err("ident must use <owner>#<package>".to_string());
    };
    let repo_url = mfb_repository::client::repo_url_from_env();
    let paths = super::local_paths_for_repo(&repo_url)?;
    let response =
        mfb_repository::client::transfer_offer(&repo_url, &paths, ident, from_owner, to_owner)?;
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
        target: target::BuildTarget::host(),
        sign_owner: None,
        app_mode: false,
        app_debug: false,
        regalloc: target::shared::code::regalloc::active_kind(),
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
/// The parsed form of `mfb pkg add <target> [--pin|--no-pin]` (plan-60-C §4.3).
struct AddOptions {
    target: String,
    pin: Option<bool>,
}

/// Parse `add`'s arguments, following `run_pkg_doc`'s shape (`:1177`): one
/// positional, explicit flags, unknown-flag rejection, second-positional
/// rejection.
///
/// `--pin` and `--no-pin` together are a **usage error**, not last-flag-wins:
/// the two orderings would otherwise mean different things with no way to tell
/// from the command line which was intended (§4.1).
fn parse_add_options(args: &[String]) -> Result<AddOptions, PkgCommandError> {
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
fn infer_pin(has_explicit_version: bool, pin_flag: Option<bool>) -> bool {
    match pin_flag {
        Some(explicit) => explicit,
        None => has_explicit_version,
    }
}

fn add_package(project_dir: &Path, options: &AddOptions) -> Result<(), String> {
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

fn add_package_from_file(project_dir: &Path, url: &str) -> Result<(), String> {
    let source_path = package_file_url_path(url)?;
    let package = read_mfp_header(&source_path)?;

    // plan-48-B (Open Decisions): a `file://` add is a local copy with no
    // registry to fetch vendor blobs from. A package that vendors native
    // libraries would install but never build, so refuse it explicitly rather
    // than leaving a silently unusable install.
    let (_name, native_libraries) = binary_repr::read_package_native_libraries(&source_path)?;
    if native_libraries
        .entries
        .iter()
        .flat_map(|entry| &entry.locators)
        .any(|locator| locator.lib_type == crate::manifest::libraries::LibType::Vendor)
    {
        return Err(format!(
            "`{}` vendors native libraries, which a `file://` add cannot fetch (there is no \
             registry). Publish it and `mfb pkg add {}#…` instead.",
            package.name, package.ident
        ));
    }

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
fn add_package_from_registry(
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
        ident_key: index.ident_key.clone(),
    };
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
    use std::collections::HashSet;

    let installed = project_dir
        .join("packages")
        .join(format!("{package_name}.mfp"));
    let (_name, table) = binary_repr::read_package_native_libraries(&installed)?;
    let vendor_dir = crate::manifest::libraries::imported_vendor_dir(project_dir, package_name);

    let mut seen: HashSet<String> = HashSet::new();
    let mut placed = 0usize;
    for entry in &table.entries {
        for locator in &entry.locators {
            if locator.lib_type != LibType::Vendor {
                continue;
            }
            let Some(hash) = locator.hash else { continue };
            let hash_hex = hex_bytes(&hash);
            if !seen.insert(hash_hex.clone()) {
                continue;
            }
            // `source` came from the `.mfp` — untrusted input. plan-46-B §4.1's
            // decoder already re-checked the bare-filename rule, but do not assume
            // it: re-validate before `source` becomes a path.
            if let Err(reason) = crate::manifest::libraries::source_is_bare(&locator.source) {
                return Err(format!(
                    "package `{package_name}` names an unsafe vendored file \"{}\": {reason}",
                    locator.source
                ));
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
            super::install_vendor_file(&vendor_dir, &locator.source, &bytes)?;
            placed += 1;
        }
    }
    if placed > 0 {
        println!(
            "Downloaded {placed} vendor {} for {package_name}",
            if placed == 1 { "library" } else { "libraries" }
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
fn select_index_version<'a>(
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
// coverage:off — the interesting branches require a live registry ident chain
// (fetch_ident_chain) reached only after a real key rotation; covered by the
// tests/ ident-rotation integration harness.
fn follow_rotated_pin(
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
        PkgCommandError::Usage(format!("mfb pkg doc requires <name-or-path>\n\n{PKG_HELP}"))
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

    println!("Package: {}", crate::terminal_safe::safe(&header.name));
    println!("Ident: {}", empty_marker(&header.ident));
    println!("Version: {}", crate::terminal_safe::safe(&header.version));
    println!("Ident Key: {}", empty_marker(&header.ident_key));
    println!("Signing Key: {}", empty_marker(&header.signing_key));
    println!(
        "Proof: {}",
        if header.proof.is_empty() {
            std::borrow::Cow::Borrowed("<none>")
        } else {
            crate::terminal_safe::safe(&header.proof)
        }
    );
    println!(
        "Attestation: {}",
        if header.attestation.is_empty() {
            std::borrow::Cow::Borrowed("<none>")
        } else {
            crate::terminal_safe::safe(&header.attestation)
        }
    );
    println!("Author: {}", empty_marker(&header.author));
    println!("URL: {}", empty_marker(&header.url));
    // Best-effort registry release state (plan-10-C1): only shown when the
    // package has a registry ident and the configured registry answers. Silent
    // otherwise, so offline `pkg info` is unchanged.
    if let Some((owner, package_name)) = header.ident.split_once('#') {
        let repo_url = mfb_repository::client::repo_url_from_env();
        if let Ok(paths) = super::local_paths_for_repo(&repo_url) {
            if let Ok(index) =
                mfb_repository::client::fetch_index(&repo_url, &paths, owner, package_name)
            {
                if let Some(version) = index
                    .versions
                    .iter()
                    .find(|version| version.version == header.version)
                {
                    println!("Release State: {}", version.state);
                }
            }
        }
    }
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

/// `<empty>` for a blank field, otherwise the value with terminal-unsafe code
/// points escaped. Header fields come from an untrusted `.mfp`, so an embedded
/// ESC/newline or bidi override must not reach the operator's terminal verbatim
/// (bug-210; `read_mfp_string` only enforces valid UTF-8).
fn empty_marker(value: &str) -> std::borrow::Cow<'_, str> {
    if value.is_empty() {
        std::borrow::Cow::Borrowed("<empty>")
    } else {
        crate::terminal_safe::safe(value)
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

#[cfg(test)]
mod tests {
    use super::*;

    const UNSIGNED_FIXTURE: &str =
        "tests/syntax/packages/package-trap-builtin/golden/trap_builtin_pkg.mfp";

    fn s(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    fn usage(result: Result<(), PkgCommandError>) -> String {
        match result {
            Err(PkgCommandError::Usage(message)) => message,
            Err(PkgCommandError::Failed(message)) => {
                panic!("expected usage error, got failure: {message}")
            }
            Ok(()) => panic!("expected usage error, got Ok"),
        }
    }

    fn index_version(
        version: &str,
        state: &str,
        published_at: i64,
    ) -> mfb_repository::server::IndexVersion {
        mfb_repository::server::IndexVersion {
            version: version.to_string(),
            hash: format!("hash-{version}"),
            published_at,
            state: state.to_string(),
            abi_index: serde_json::Value::Null,
            log_entry: None,
        }
    }

    fn index(
        versions: Vec<mfb_repository::server::IndexVersion>,
    ) -> mfb_repository::server::IndexResponse {
        mfb_repository::server::IndexResponse {
            ident: "ada#shape".to_string(),
            owner: "ada".to_string(),
            ident_key: "ed25519:ik".to_string(),
            ident_fingerprint: "if".to_string(),
            name_binding_signature: String::new(),
            server_fingerprint: "sf".to_string(),
            versions,
        }
    }

    #[test]
    fn signature_type_name_maps_known_and_unknown() {
        assert_eq!(signature_type_name(0), "unsigned");
        assert_eq!(signature_type_name(1), "Ed25519");
        assert_eq!(signature_type_name(9), "unknown (9)");
    }

    #[test]
    fn hex_bytes_formats_lowercase_two_digit() {
        assert_eq!(hex_bytes(&[0x00, 0x0f, 0xab, 0xff]), "000fabff");
        assert_eq!(hex_bytes(&[]), "");
    }

    #[test]
    fn empty_marker_marks_empty_strings() {
        assert_eq!(empty_marker(""), "<empty>");
        assert_eq!(empty_marker("value"), "value");
    }

    #[test]
    fn package_export_kind_names() {
        use binary_repr::BinaryReprExportKind::*;
        assert_eq!(package_export_kind_name(Func), "FUNC");
        assert_eq!(package_export_kind_name(Sub), "SUB");
        assert_eq!(package_export_kind_name(Type), "TYPE");
        assert_eq!(package_export_kind_name(Union), "UNION");
        assert_eq!(package_export_kind_name(Enum), "ENUM");
    }

    #[test]
    fn package_verify_status_messages() {
        assert_eq!(PackageVerifyStatus::Ok.message(), "OK");
        assert_eq!(PackageVerifyStatus::NeedsUpdate.message(), "Needs Update");
        assert_eq!(
            PackageVerifyStatus::InvalidPackage.message(),
            "Invalid Package"
        );
    }

    #[test]
    fn state_is_floating_eligible_classifies_states() {
        assert!(state_is_floating_eligible("available"));
        assert!(state_is_floating_eligible("deprecated"));
        assert!(!state_is_floating_eligible("yanked"));
        assert!(!state_is_floating_eligible("blocked"));
        assert!(!state_is_floating_eligible("legal-tombstoned"));
    }

    #[test]
    fn select_index_version_empty_index_errors() {
        let index = index(Vec::new());
        assert!(select_index_version(&index, None)
            .unwrap_err()
            .contains("no published versions"));
    }

    #[test]
    fn select_index_version_exact_request() {
        let index = index(vec![
            index_version("1.0.0", "available", 1),
            index_version("2.0.0", "available", 2),
        ]);
        let chosen = select_index_version(&index, Some("1.0.0")).expect("chosen");
        assert_eq!(chosen.version, "1.0.0");
        // A missing exact version errors.
        assert!(select_index_version(&index, Some("9.9.9"))
            .unwrap_err()
            .contains("no version `9.9.9`"));
    }

    #[test]
    fn select_index_version_exact_request_rejects_blocked() {
        let index = index(vec![index_version("1.0.0", "blocked", 1)]);
        assert!(select_index_version(&index, Some("1.0.0"))
            .unwrap_err()
            .contains("cannot be installed"));
    }

    #[test]
    fn select_index_version_floating_picks_newest_eligible() {
        let index = index(vec![
            index_version("1.0.0", "available", 10),
            index_version("2.0.0", "yanked", 20),
            index_version("1.5.0", "deprecated", 15),
        ]);
        // Newest floating-eligible is 1.5.0 (2.0.0 is yanked, pin-only).
        let chosen = select_index_version(&index, None).expect("chosen");
        assert_eq!(chosen.version, "1.5.0");
    }

    #[test]
    fn select_index_version_floating_none_eligible_errors() {
        let index = index(vec![index_version("1.0.0", "yanked", 1)]);
        assert!(select_index_version(&index, None)
            .unwrap_err()
            .contains("no install-eligible version"));
    }

    #[test]
    fn a_pinned_dependency_demands_an_exact_version() {
        let dependency = |version: &str, pin: bool| ProjectPackageDependency {
            name: "shape".to_string(),
            ident: "ada#shape".to_string(),
            version: version.to_string(),
            pin,
            source: "ada#shape".to_string(),
            ident_key: String::new(),
        };
        let status = |version: &str, pin: bool, actual: &str| {
            package_dependency_status(&dependency(version, pin), "shape", "ada#shape", actual)
        };

        assert_eq!(status("1.0.0", true, "1.0.0"), PackageVerifyStatus::Ok);
        assert_eq!(
            status("1.0.0", true, "1.2.0"),
            PackageVerifyStatus::NeedsUpdate
        );
        // An empty pinned version can never match: builds reject it too
        // (`installed_package_files`), so `pkg verify` must not report `Ok`.
        assert_eq!(status("", true, "1.0.0"), PackageVerifyStatus::NeedsUpdate);
        // Unpinned: an empty version means "any version".
        assert_eq!(status("", false, "1.0.0"), PackageVerifyStatus::Ok);
        assert_eq!(status("1.0.0", false, "1.0.0"), PackageVerifyStatus::Ok);
        assert_eq!(
            status("1.0.0", false, "1.2.0"),
            PackageVerifyStatus::NeedsUpdate
        );
        // A name or ident disagreement outranks the version check either way.
        assert_eq!(
            package_dependency_status(&dependency("1.0.0", true), "other", "ada#shape", "1.0.0"),
            PackageVerifyStatus::InvalidPackage
        );
    }

    #[test]
    fn package_verify_line_formats_with_and_without_version() {
        let dependency = ProjectPackageDependency {
            name: "shape".to_string(),
            ident: "ada#shape".to_string(),
            version: "1.0.0".to_string(),
            pin: false,
            source: "ada#shape".to_string(),
            ident_key: String::new(),
        };
        assert_eq!(
            package_verify_line(
                &dependency,
                &PackageVerifyResult {
                    version: String::new(),
                    status: PackageVerifyStatus::InvalidPackage,
                }
            ),
            "shape @ 1.0.0 : Invalid Package"
        );
        assert_eq!(
            package_verify_line(
                &dependency,
                &PackageVerifyResult {
                    version: "1.2.0".to_string(),
                    status: PackageVerifyStatus::Ok,
                }
            ),
            "shape @ 1.0.0 : OK (1.2.0)"
        );
    }

    #[test]
    fn run_pkg_requires_a_subcommand() {
        assert!(usage(run_pkg_command(&s(&[]))).contains("mfb pkg requires a subcommand"));
    }

    #[test]
    fn run_pkg_rejects_unknown_command() {
        assert!(usage(run_pkg_command(&s(&["frobnicate"]))).contains("unknown pkg command"));
    }

    /// plan-42 §4.5: the top-level screen advertises only a few pkg commands, so
    /// pkg's own usage errors must interpolate `PKG_HELP` — the complete list —
    /// not the trimmed top-level `USAGE` they used to show.
    ///
    /// plan-60-A: the witness commands changed. This test used to probe for
    /// `check-abi`/`release-state`/`transfer-accept`, chosen because they were
    /// in `PKG_HELP` but not in `USAGE`. All three moved to `mfb repo`, so they
    /// are no longer valid witnesses *for pkg* — the assertion's intent
    /// (pkg errors show the full pkg set) is unchanged, only the commands that
    /// can witness it. `info` and `validate` are the surviving pkg-only
    /// commands absent from the top-level screen.
    #[test]
    fn run_pkg_usage_errors_show_the_full_pkg_command_set() {
        let message = usage(run_pkg_command(&s(&["frobnicate"])));
        // Present in PKG_HELP, absent from the trimmed top-level screen.
        for command in ["info", "validate"] {
            assert!(
                message.contains(command),
                "pkg error must list `{command}`: {message}"
            );
        }
        assert!(message.contains("Usage: mfb pkg <command>"));
        // ...and must not be the top-level screen.
        assert!(!message.contains("Project Setup:"));
    }

    /// plan-60-A §1: every command is listed under exactly one parent. `PKG_HELP`
    /// must not advertise a command that `mfb pkg` no longer dispatches, and
    /// `REPO_HELP` must advertise every command it now owns. Without this, the
    /// help text can drift back into naming a command that hard-errors.
    #[test]
    fn help_lists_each_moved_command_under_repo_only() {
        for command in [
            "publish",
            "check-abi",
            "release-state",
            "transfer",
            "transfer-accept",
        ] {
            assert!(
                crate::REPO_HELP.contains(&format!("repo {command}")),
                "REPO_HELP must document `repo {command}`"
            );
            assert!(
                !crate::PKG_HELP.contains(&format!("  {command} ")),
                "PKG_HELP must not still list `{command}` as a pkg command"
            );
        }
        // The surviving consumer-side commands stay exactly where they were.
        for command in [
            "add", "info", "doc", "verify", "validate", "install", "update",
        ] {
            assert!(
                crate::PKG_HELP.contains(command),
                "PKG_HELP must still document `{command}`"
            );
        }
    }

    #[test]
    fn run_pkg_usage_errors_for_wrong_arity() {
        assert!(usage(run_pkg_command(&s(&["add"]))).contains("mfb pkg add requires"));
        assert!(usage(run_pkg_command(&s(&["info"]))).contains("mfb pkg info requires"));
        assert!(usage(run_pkg_command(&s(&["verify", "extra", "junk"])))
            .contains("mfb pkg verify accepts only"));
        assert!(usage(run_pkg_command(&s(&["validate"]))).contains("mfb pkg validate requires"));
    }

    /// plan-60-A: the five publisher-side commands moved to `mfb repo`. They are
    /// a hard error under `pkg` — no alias, no deprecation shim — but the error
    /// names the new location rather than falling through to the generic
    /// "unknown pkg command" arm.
    ///
    /// This replaces the five arity assertions that used to live in
    /// `run_pkg_usage_errors_for_wrong_arity`: arity is now `repo`'s
    /// responsibility, and is asserted in `super::repo`'s tests. What must be
    /// pinned *here* is that the old spelling no longer reaches an
    /// implementation, whatever its arity.
    #[test]
    fn run_pkg_rejects_the_moved_publisher_commands() {
        for command in [
            "publish",
            "check-abi",
            "release-state",
            "transfer",
            "transfer-accept",
        ] {
            // Every arity must be rejected identically — a bare command, a
            // plausible one, and an over-long one. If any of these reached a
            // surviving `pkg` arm the move would be incomplete.
            for args in [
                vec![command],
                vec![command, "a"],
                vec![command, "a", "b"],
                vec![command, "a", "b", "c"],
            ] {
                let message = usage(run_pkg_command(&s(&args)));
                assert!(
                    message.contains(&format!(
                        "mfb pkg {command} has moved to mfb repo {command}"
                    )),
                    "`pkg {args:?}` must name the new location: {message}"
                );
                // Not the generic fallback — that would leave the user grepping.
                assert!(!message.contains("unknown pkg command"), "{message}");
            }
        }
    }

    /// Parse `add` arguments, panicking with the usage text on failure.
    /// `PkgCommandError` has no `Debug`, so this unwraps explicitly rather than
    /// deriving `Debug` on a production type for test ergonomics.
    fn parsed(args: &[&str]) -> AddOptions {
        match parse_add_options(&s(args)) {
            Ok(options) => options,
            Err(PkgCommandError::Usage(message) | PkgCommandError::Failed(message)) => {
                panic!("expected `{args:?}` to parse, got: {message}")
            }
        }
    }

    /// A bare `add` invocation with no flags — the common shape in these tests.
    fn add_opts(target: &str) -> AddOptions {
        AddOptions {
            target: target.to_string(),
            pin: None,
        }
    }

    /// plan-60-C §4.1, one case per row of the inference matrix.
    ///
    /// | Invocation | `pin` |
    /// |---|---|
    /// | `add alice#shape` | `false` |
    /// | `add alice#shape@1.4.0` | `true` |
    /// | `add alice#shape --pin` | `true` |
    /// | `add alice#shape@1.4.0 --no-pin` | `false` |
    #[test]
    fn infer_pin_follows_the_matrix() {
        // No flag: the presence of an explicit @version decides.
        assert!(!infer_pin(false, None), "bare add floats");
        assert!(infer_pin(true, None), "an explicit @version implies a pin");
        // An explicit flag always wins, in both directions.
        assert!(infer_pin(false, Some(true)), "--pin on a bare add");
        assert!(
            !infer_pin(true, Some(false)),
            "--no-pin overrides the @version implication; the version becomes the ABI floor"
        );
        // ...and is honoured even when it merely restates the default.
        assert!(infer_pin(true, Some(true)));
        assert!(!infer_pin(false, Some(false)));
    }

    #[test]
    fn parse_add_options_reads_the_target_and_flags() {
        let parse = parsed;

        let bare = parse(&["ada#shape"]);
        assert_eq!(bare.target, "ada#shape");
        assert_eq!(bare.pin, None, "no flag means infer, not a default");

        assert_eq!(parse(&["ada#shape", "--pin"]).pin, Some(true));
        assert_eq!(parse(&["ada#shape", "--no-pin"]).pin, Some(false));
        // Flags may precede the positional.
        let leading = parse(&["--no-pin", "ada#shape"]);
        assert_eq!(leading.target, "ada#shape");
        assert_eq!(leading.pin, Some(false));
    }

    /// `--pin --no-pin` is a usage error rather than last-flag-wins: the two
    /// orderings would otherwise mean different things with no way to tell from
    /// the command line which was intended (§4.1).
    #[test]
    fn parse_add_options_rejects_bad_argument_shapes() {
        let err = |args: &[&str]| usage(parse_add_options(&s(args)).map(|_| ()));

        assert!(err(&["ada#shape", "--pin", "--no-pin"]).contains("cannot take both"));
        // ...in either order — neither wins.
        assert!(err(&["ada#shape", "--no-pin", "--pin"]).contains("cannot take both"));
        assert!(err(&["--bogus", "ada#shape"]).contains("unknown flag"));
        assert!(err(&["a", "b"]).contains("accepts exactly one"));
        assert!(err(&[]).contains("mfb pkg add requires"));
        assert!(err(&["--pin"]).contains("mfb pkg add requires"));
    }

    /// §4.2: `--no-pin` on a `file://` target is a usage error naming the
    /// reason. `--pin` is accepted as a redundant statement of the truth.
    #[test]
    fn add_rejects_no_pin_on_a_file_url() {
        let options = parsed(&["file:///tmp/x.mfp", "--no-pin"]);
        let err = add_package(Path::new("."), &options).expect_err("must refuse");
        assert!(err.contains("no registry"), "{err}");
        assert!(err.contains("--no-pin"), "{err}");

        // `--pin` on a file:// target is a no-op, not an error: it gets past the
        // §4.2 guard and fails later, on the missing file.
        let pinned = parsed(&["file:///tmp/does-not-exist.mfp", "--pin"]);
        let err = add_package(Path::new("."), &pinned).expect_err("missing file");
        assert!(
            !err.contains("no registry"),
            "--pin must not be refused: {err}"
        );
    }

    #[test]
    fn add_package_rejects_bad_target() {
        // Neither a file:// URL nor an <owner>#<package> ident.
        let err = add_package(Path::new("."), &add_opts("just-a-name")).unwrap_err();
        assert!(err.contains("expects a file:// URL or an <owner>#<package>"));
    }

    #[test]
    fn add_package_from_registry_rejects_malformed_ident() {
        assert!(add_package_from_registry(Path::new("."), "no-hash", None)
            .unwrap_err()
            .contains("must use <owner>#<package>"));
        assert!(add_package_from_registry(Path::new("."), "#pkg", None)
            .unwrap_err()
            .contains("must use <owner>#<package>"));
        assert!(add_package_from_registry(Path::new("."), "owner#", None)
            .unwrap_err()
            .contains("must use <owner>#<package>"));
        // Empty version after `@`.
        assert!(
            add_package_from_registry(Path::new("."), "ada#shape@", None)
                .unwrap_err()
                .contains("must not be empty")
        );
    }

    #[test]
    fn transfer_offer_rejects_bad_ident() {
        assert!(transfer_offer("no-hash", "bob")
            .unwrap_err()
            .contains("<owner>#<package>"));
    }

    #[test]
    fn transfer_accept_requires_at_sign() {
        assert!(transfer_accept("ada#shape")
            .unwrap_err()
            .contains("<owner>#<package>@<to-owner>"));
    }

    #[test]
    fn set_release_state_rejects_bad_state() {
        assert!(set_release_state(Path::new("."), "bogus", None)
            .unwrap_err()
            .contains("state must be one of"));
    }

    #[test]
    fn set_release_state_requires_package_project() {
        // A directory without a project.json fails manifest validation.
        let dir = tempfile::tempdir().expect("temp dir");
        assert!(set_release_state(dir.path(), "available", None)
            .unwrap_err()
            .contains("validation failed"));
    }

    #[test]
    fn run_pkg_doc_rejects_bad_arguments() {
        assert!(usage(run_pkg_doc(&s(&["--out"]))).contains("--out requires a file"));
        assert!(usage(run_pkg_doc(&s(&["--bogus"]))).contains("unknown flag"));
        assert!(usage(run_pkg_doc(&s(&["a", "b"]))).contains("accepts exactly one"));
        assert!(usage(run_pkg_doc(&s(&[]))).contains("mfb pkg doc requires"));
    }

    #[test]
    fn write_package_doc_reports_missing_package() {
        let err = write_package_doc("no-such-package", Path::new("/tmp/out.html")).unwrap_err();
        assert!(err.contains("no package named"));
    }

    #[test]
    fn write_package_doc_renders_from_a_real_package() {
        let dir = tempfile::tempdir().expect("temp dir");
        let out = dir.path().join("doc.html");
        write_package_doc(UNSIGNED_FIXTURE, &out).expect("doc render");
        assert!(out.is_file());
    }

    #[test]
    fn print_package_info_reads_a_real_package() {
        // Exercises the whole info printer over an unsigned fixture. It attempts
        // a best-effort registry lookup for the release state; the ident has no
        // `#`, so that lookup is skipped and no network call is made.
        assert!(print_package_info(Path::new(UNSIGNED_FIXTURE)).is_ok());
    }

    #[test]
    fn print_package_info_reports_missing_file() {
        assert!(print_package_info(Path::new("/no/such/pkg.mfp"))
            .unwrap_err()
            .contains("failed to read"));
    }

    #[test]
    fn validate_package_file_reports_missing_package() {
        let dir = tempfile::tempdir().expect("temp dir");
        let err = validate_package_file(dir.path(), "no-such-package").unwrap_err();
        assert!(err.contains("no package named"));
    }

    #[test]
    fn validate_package_file_validates_an_unsigned_fixture() {
        let dir = tempfile::tempdir().expect("temp dir");
        // Unsigned package: container + payload hash checks pass, no trust chain.
        assert!(validate_package_file(dir.path(), UNSIGNED_FIXTURE).is_ok());
    }

    #[test]
    fn validate_package_file_rejects_garbage_container() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("garbage.mfp");
        std::fs::write(&path, b"not a container").expect("write");
        assert!(validate_package_file(dir.path(), path.to_str().unwrap()).is_err());
    }

    #[test]
    fn project_pinned_ident_key_finds_declared_pin() {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(
            dir.path().join("project.json"),
            concat!(
                "{\"name\":\"app\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",",
                "\"sources\":[{\"root\":\"src\"}],",
                "\"packages\":[{\"name\":\"shape\",\"ident\":\"ada#shape\",\"version\":\"1.0.0\",\"pin\":true,\"source\":\"ada#shape\",\"identKey\":\"ed25519:pinned\"}]}"
            ),
        )
        .expect("manifest");
        assert_eq!(
            project_pinned_ident_key(dir.path(), "shape"),
            Some("ed25519:pinned".to_string())
        );
        // An undeclared package has no pin.
        assert_eq!(project_pinned_ident_key(dir.path(), "other"), None);
        // A missing manifest yields None (best-effort).
        let empty = tempfile::tempdir().expect("temp dir");
        assert_eq!(project_pinned_ident_key(empty.path(), "shape"), None);
    }

    #[test]
    fn verify_packages_reports_no_manifest() {
        let dir = tempfile::tempdir().expect("temp dir");
        assert!(verify_packages(dir.path(), false)
            .unwrap_err()
            .contains("failed to read"));
    }

    #[test]
    fn verify_packages_ok_with_no_dependencies() {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(
            dir.path().join("project.json"),
            "{\"name\":\"app\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"sources\":[{\"root\":\"src\"}]}",
        )
        .expect("manifest");
        assert!(verify_packages(dir.path(), false).is_ok());
    }

    #[test]
    fn check_abi_requires_package_project() {
        let dir = tempfile::tempdir().expect("temp dir");
        assert!(check_abi(dir.path())
            .unwrap_err()
            .contains("validation failed"));
    }

    /// plan-60-A: `publish` dispatches from `mfb repo` now, so this drives the
    /// implementation directly rather than through `run_pkg_command` (which
    /// correctly rejects the old spelling — see
    /// `run_pkg_rejects_the_moved_publisher_commands`). The assertion is
    /// unchanged: an empty directory is not a package project, and `publish`
    /// must refuse it before doing anything else. The `repo` dispatch that now
    /// reaches this code is covered by
    /// `super::repo::tests::repo_publisher_commands_pin_their_arity`.
    #[test]
    fn publish_requires_package_project() {
        let dir = tempfile::tempdir().expect("temp dir");
        assert!(publish_package_project("ada", dir.path())
            .unwrap_err()
            .contains("validation failed"));
    }

    #[test]
    fn print_publish_verify_report_handles_both_diagnostic_states() {
        // With diagnostics.
        print_publish_verify_report(&mfb_repository::server::ValidatePackageResponse {
            valid: false,
            content_hash: "abc".to_string(),
            abi_index: serde_json::Value::Null,
            diagnostics: vec!["one".to_string(), "two".to_string()],
        });
        // Without diagnostics (the <none> branch) and an empty content hash.
        print_publish_verify_report(&mfb_repository::server::ValidatePackageResponse {
            valid: true,
            content_hash: String::new(),
            abi_index: serde_json::Value::Null,
            diagnostics: Vec::new(),
        });
    }

    #[test]
    fn verify_package_dependency_reports_missing_package_as_invalid() {
        let dir = tempfile::tempdir().expect("temp dir");
        let dependency = ProjectPackageDependency {
            name: "absent".to_string(),
            ident: "ada#absent".to_string(),
            version: "1.0.0".to_string(),
            pin: false,
            source: "ada#absent".to_string(),
            ident_key: String::new(),
        };
        // Neither a .mfp nor a source manifest exists -> InvalidPackage.
        assert_eq!(
            verify_package_dependency(dir.path(), &dependency).status,
            PackageVerifyStatus::InvalidPackage
        );
    }

    #[test]
    fn add_package_from_file_reports_missing_source() {
        // A file:// URL to a non-existent .mfp surfaces a read error before any
        // manifest work.
        let err = add_package(Path::new("."), &add_opts("file:///no/such/pkg.mfp")).unwrap_err();
        assert!(!err.is_empty());
    }
}
