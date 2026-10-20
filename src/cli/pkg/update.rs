//! plan-176-L: `mfb pkg` — the update commands. The dispatcher `run_pkg_command` and the
//! shared `use` header live in `cli/pkg/mod.rs`.

use super::*;

/// The parsed form of `mfb pkg update [<ident>[@version]] [--pin|--no-pin] [--yes]`
/// (plan-60-E §4.4).
pub(super) struct UpdateOptions {
    /// `None` is the bare form: re-resolve everything declared.
    pub(super) target: Option<String>,
    pub(super) pin: Option<bool>,
    pub(super) assume_yes: bool,
}

/// Parse `update`'s arguments (plan-60-E §4.4).
///
/// **The `[location]` form is gone.** A positional now means an ident, and
/// `mfb pkg update foo` cannot mean both a path and a package. Nothing in the
/// tree passed a path, so this removes an unused shape rather than a used one.
pub(super) fn parse_update_options(args: &[String]) -> Result<UpdateOptions, PkgCommandError> {
    let mut target: Option<String> = None;
    let mut pin = false;
    let mut no_pin = false;
    let mut assume_yes = false;

    for arg in args {
        match arg.as_str() {
            "--pin" => pin = true,
            "--no-pin" => no_pin = true,
            "--yes" => assume_yes = true,
            flag if flag.starts_with("--") => {
                return Err(PkgCommandError::Usage(format!("unknown flag `{flag}`")));
            }
            value => {
                if target.is_some() {
                    return Err(PkgCommandError::Usage(
                        "mfb pkg update accepts at most one <owner>#<package>[@version]"
                            .to_string(),
                    ));
                }
                target = Some(value.to_string());
            }
        }
    }

    if pin && no_pin {
        return Err(PkgCommandError::Usage(
            "mfb pkg update cannot take both --pin and --no-pin".to_string(),
        ));
    }

    Ok(UpdateOptions {
        target,
        pin: if pin {
            Some(true)
        } else if no_pin {
            Some(false)
        } else {
            None
        },
        assume_yes,
    })
}

/// `mfb pkg update` — bare form re-resolves everything; targeted form updates
/// one declared dependency (plan-60-E).
pub(super) fn run_update(project_dir: &Path, options: &UpdateOptions) -> Result<(), String> {
    let Some(target) = options.target.as_deref() else {
        // Bare form: unchanged behavior.
        return super::resolve::update(project_dir);
    };
    let (ident, requested_version) = match target.split_once('@') {
        Some((ident, version)) if !version.is_empty() => (ident, Some(version)),
        Some((_, _)) => return Err("version after `@` must not be empty".to_string()),
        None => (target, None),
    };

    // §4.4: the not-declared check runs BEFORE any network access, so a typo
    // costs a local error rather than a round trip.
    let project_path = project_dir.join("project.json");
    let contents = fs::read_to_string(&project_path)
        .map_err(|err| format!("failed to read '{}': {err}", project_path.display()))?;
    let manifest = parse_project_json(&contents, &project_path)?;
    validate_packages_array(&manifest)?;

    let declared = manifest
        .get("packages")
        .and_then(|value| value.get::<Vec<JsonValue>>())
        .into_iter()
        .flatten()
        .filter_map(project_package_dependency)
        .find(|dep| dep.ident == ident)
        .ok_or_else(|| {
            format!("`{ident}` is not declared in project.json; use `mfb pkg add {ident}`")
        })?;

    update_declared_dependency(
        project_dir,
        &contents,
        &declared,
        requested_version,
        options,
    )
}

/// Resolve the target version, confirm if the pin will move, and apply
/// (plan-60-E §4.2, §4.5, §4.6).
// coverage:off — reaches a live registry via fetch_index; the selection matrix
// (`select_update_version`), the manifest rewrite
// (`project_json_with_updated_version`) and the not-declared check are all
// unit-tested directly, and the full path is covered by the tests/ registry
// integration harness.
fn update_declared_dependency(
    project_dir: &Path,
    contents: &str,
    declared: &ProjectPackageDependency,
    requested_version: Option<&str>,
    options: &UpdateOptions,
) -> Result<(), String> {
    let (owner, package) = declared
        .ident
        .split_once('#')
        .ok_or_else(|| "registry ident must use <owner>#<package>".to_string())?;

    let new_version = match requested_version {
        // An exact `@version` is always honored: the user named it themselves,
        // so the §4.3 advisory does not apply and this is the escape hatch it
        // points at.
        Some(version) => version.to_string(),
        None => {
            let repo_url = mfb_repository::client::repo_url_from_env();
            let paths = super::local_paths_for_repo(&repo_url)?;
            let index = mfb_repository::client::fetch_index(&repo_url, &paths, owner, package)?;
            match select_update_version(&declared.version, &index.versions) {
                UpdateChoice::AlreadyLatest(version) => {
                    println!(
                        "{} is already on the latest eligible version ({version})",
                        declared.ident
                    );
                    // Nothing changed: no manifest write, no resolve, no install.
                    return Ok(());
                }
                UpdateChoice::NoCompatibleCandidate => {
                    return Err(format!(
                        "no newer version of {} is ABI-compatible with the declared {}; \
                         use `@<version>` to take one anyway",
                        declared.ident, declared.version
                    ));
                }
                UpdateChoice::Select { version, skipped } => {
                    if let Some((newest, dropped)) = skipped {
                        let symbols = if dropped.is_empty() {
                            "some exports".to_string()
                        } else {
                            dropped.join(", ")
                        };
                        eprintln!(
                            "{} {newest} is available but drops symbols the currently declared {} \
                             exports ({symbols}); selecting {version} instead. Use `@{newest}` to \
                             take it anyway.",
                            declared.ident, declared.version
                        );
                    }
                    version
                }
            }
        }
    };

    if new_version == declared.version && options.pin.is_none() {
        println!(
            "{} is already at {new_version}; nothing to do",
            declared.ident
        );
        return Ok(());
    }

    // §4.5: moving a PINNED dependency changes a deliberate choice, so ask.
    // A floating one is not prompted — its version is a floor, not a promise.
    if declared.pin && options.pin != Some(false) {
        let question = format!(
            "{} is pinned to {}. Updating it to {new_version} will change the pin.\nContinue?",
            declared.ident, declared.version
        );
        if !super::confirm(&question, options.assume_yes)? {
            // The user answered the question that was asked. Not a failure.
            println!("Left {} at {}.", declared.ident, declared.version);
            return Ok(());
        }
    }

    // §4.1: pin state is PRESERVED unless a flag was passed.
    let updated =
        project_json_with_updated_version(contents, &declared.ident, &new_version, options.pin)?;
    super::resolve::apply_manifest_change(project_dir, &updated)?;

    let pin = options.pin.unwrap_or(declared.pin);
    let suffix = if pin { "(pinned)" } else { "(floating)" };
    println!(
        "Updated {} {} -> {new_version} {suffix}",
        declared.ident, declared.version
    );
    Ok(())
}

/// What a targeted `mfb pkg update` (no `@version`) decided to do.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum UpdateChoice {
    /// Nothing eligible is newer than what is declared.
    AlreadyLatest(String),
    /// Move to this version. `skipped` names a newer version that was rejected
    /// by the ABI filter, so the user learns it exists (§4.3).
    Select {
        version: String,
        skipped: Option<(String, Vec<String>)>,
    },
    /// Something newer exists but nothing newer is ABI-compatible.
    NoCompatibleCandidate,
}

/// plan-60-E §4.2 + §4.3: choose the version a bare targeted update moves to.
///
/// Pure — takes the declared version and the index's versions, so the whole
/// selection/advisory matrix is testable without a registry.
///
/// Three filters, in order:
///
/// 1. **Eligibility.** `state_is_floating_eligible` excludes `yanked`, which the
///    spec makes reachable only by exact pin. A bare update must never move a
///    dependency *onto* a yanked release.
/// 2. **Newer.** Only versions strictly greater than the declared one.
/// 3. **ABI.** Keep candidates whose exports are a superset of the currently
///    declared version's. This is a *pre-flight advisory*, not a proof: it shows
///    the candidate still exports what the declared version did, but not that it
///    satisfies the union of every requirer's needs — that union is built by
///    `resolve()` from sibling import tables and does not exist yet. `resolve()`
///    remains the authority; when it disagrees afterwards, it wins.
///
/// The ABI filter exists because `select_node`'s pin branch takes an exact
/// version with **no** ABI check, so without it a targeted update could move a
/// pinned dependency onto a release that dropped a symbol the project uses —
/// resolving cleanly and failing at build time.
pub(super) fn select_update_version(
    declared_version: &str,
    versions: &[mfb_repository::server::IndexVersion],
) -> UpdateChoice {
    let baseline = versions
        .iter()
        .find(|version| version.version == declared_version)
        .map(|version| version.abi_map())
        .unwrap_or_default();

    let mut newer: Vec<_> = versions
        .iter()
        .filter(|version| state_is_floating_eligible(&version.state))
        .filter(|version| {
            super::resolve::compare_versions(&version.version, declared_version)
                == std::cmp::Ordering::Greater
        })
        .collect();
    if newer.is_empty() {
        return UpdateChoice::AlreadyLatest(declared_version.to_string());
    }
    // Highest first, so the first ABI-compatible entry is the best choice and
    // anything before it is what got skipped.
    newer.sort_by(|a, b| super::resolve::compare_versions(&b.version, &a.version));

    let newest = newer[0];
    match newer
        .iter()
        .find(|version| super::resolve::is_superset(&version.abi_map(), &baseline))
    {
        None => UpdateChoice::NoCompatibleCandidate,
        Some(chosen) if chosen.version == newest.version => UpdateChoice::Select {
            version: chosen.version.clone(),
            skipped: None,
        },
        Some(chosen) => {
            // §4.3: do not silently take an older version. Name the newer one
            // and the symbols it drops, so the escape hatch is discoverable.
            let newest_exports = newest.abi_map();
            let mut dropped: Vec<String> = baseline
                .keys()
                .filter(|symbol| !newest_exports.contains_key(*symbol))
                .cloned()
                .collect();
            dropped.sort();
            UpdateChoice::Select {
                version: chosen.version.clone(),
                skipped: Some((newest.version.clone(), dropped)),
            }
        }
    }
}
