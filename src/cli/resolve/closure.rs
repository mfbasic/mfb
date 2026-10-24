//! plan-176-L: the dependency resolver's closure code. Resolution itself (`update`,
//! `install`, `resolve`, `select_node`) lives in `cli/resolve/mod.rs`.

use super::*;
use std::collections::BTreeSet;

/// What closing the dependency set changed beyond the caller's own edit.
#[derive(Debug, Default)]
pub(crate) struct ClosureChanges {
    /// `(name, the declared package that needs it)` for each entry declared.
    pub(crate) added: Vec<(String, String)>,
    /// The indirect entries dropped because nothing requires them any more.
    pub(crate) removed: Vec<String>,
}

/// Apply a proposed `project.json` **resolve-first** (plan-60-B §4.2): resolve
/// the new manifest before anything touches disk, so a resolution failure leaves
/// the project exactly as it was.
///
/// Callers pass the complete proposed manifest *text*, not a mutation closure —
/// the existing editors (`project_json_with_package`,
/// `project_json_with_updated_ident_key`) work by surgical string edit to
/// preserve formatting and comments, and this signature keeps that property.
///
/// The ordering is the whole point. `add`/`update`/`remove` previously wrote
/// `project.json` and only then resolved, so a failed resolve left a manifest
/// naming a dependency that could not be locked — and every subsequent
/// `mfb pkg install` hard-errored on the now-stale lock.
///
/// **The accepted failure window is between steps 5 and 7.** If `install` fails
/// (network, a blob that does not verify), `project.json` and `mfb.lock` are
/// already written and mutually consistent — `projectHash` matches, because the
/// lock was resolved from this exact manifest text. Only `packages/` is
/// incomplete, which `mfb pkg install` recovers. That is strictly better than
/// the alternative it replaces: a partially-populated `packages/` paired with a
/// manifest that never mentioned the new dependency.
///
/// bug-628: the proposed text is first **closed** — every dependency a declared
/// package imports is declared (`direct: false`), indirect entries nothing
/// requires are dropped, and `direct`/`requiredBy` are rewritten on every entry
/// (`manifest::closure::reconcile`). A registry package's import table is only
/// readable once it is installed, so after each install the set is closed again
/// and, if that declared anything more, resolved and installed again. Only the
/// first round keeps the resolve-first guarantee; a later round that fails leaves
/// a consistent, installed manifest that is merely missing that layer, which the
/// build reports and `mfb pkg update` retries.
///
/// There is no round budget: each round installs one more layer, so a long but
/// legitimate chain takes as many rounds as it has layers. A round that would
/// change the manifest again without having installed any release new to this
/// change is a genuine cycle, and is an error naming the releases it installed.
pub(crate) fn apply_manifest_change(
    project_dir: &Path,
    new_contents: &str,
) -> Result<ClosureChanges, String> {
    let project_path = project_dir.join("project.json");

    // 1. Parse and validate the *proposed* text. Nothing is written if it is
    //    malformed, so a caller that builds a bad manifest cannot corrupt the
    //    project.
    let manifest = parse_project_json(new_contents, &project_path)?;
    validate_packages_array(&manifest)?;

    let mut changes = ClosureChanges::default();
    let mut contents = close_dependency_set(project_dir, new_contents, &mut changes)?;
    // Termination: `close_dependency_set` returns a manifest closed against what
    // is on disk, and is a function of that manifest and `packages/`; closing a
    // closed manifest over an unchanged disk returns it unchanged. A round
    // changes `packages/` only through `install()`, so the close after it can
    // declare something new only when that install wrote a release (`ident`,
    // `selected`) this change had not installed before. A round loops only if
    // it adds such a release to `installed`; the registry's releases are finite
    // and none is added twice, so the loop ends after at most (releases the
    // closure names + 1) rounds — no fixed budget, which a long but legitimate
    // chain would exceed. A round that installed nothing new yet re-closed to a
    // different manifest is not progress (a resolution alternating between
    // versions would do that forever), so it is an error, not another round.
    let mut installed: BTreeSet<(String, String)> = BTreeSet::new();
    loop {
        // The committed text can differ from `contents` by the pins resolution
        // moved (plan-168-C); the next round starts from what was written.
        contents = apply_closed_manifest(project_dir, &contents)?;
        let locked: Vec<(String, String)> = read_lock(project_dir)?
            .map(|lock| {
                lock.packages
                    .into_iter()
                    .map(|package| (package.ident, package.selected))
                    .collect()
            })
            .unwrap_or_default();
        let mut installed_new = false;
        for release in &locked {
            installed_new |= installed.insert(release.clone());
        }
        let closed = close_dependency_set(project_dir, &contents, &mut changes)?;
        if closed == contents {
            for (name, requirer) in &changes.added {
                println!("Declared package {name} (required by {requirer})");
            }
            for name in &changes.removed {
                // An orphan that was also installed goes with its entry.
                super::pkg::remove_installed_package_files(project_dir, name);
                println!("Removed package {name} (no declared package requires it)");
            }
            return Ok(changes);
        }
        if !installed_new {
            let releases: Vec<String> = locked
                .iter()
                .map(|(ident, version)| format!("`{ident}`@{version}"))
                .collect();
            return Err(format!(
                "the dependency closure of '{}' does not converge: after installing {} it \
                 changed again without any release new to this change",
                project_path.display(),
                if releases.is_empty() {
                    "no registry package".to_string()
                } else {
                    releases.join(", ")
                }
            ));
        }
        contents = closed;
    }
}

/// Close `contents` (bug-628): reconcile against the import tables readable now,
/// copy each located compiled dependency into `packages/`, and declare each
/// needed registry package from its `/index`, until nothing more can be learned
/// without an install. A needed package that is neither local nor a registry
/// ident cannot be located and is an error naming it.
fn close_dependency_set(
    project_dir: &Path,
    contents: &str,
    changes: &mut ClosureChanges,
) -> Result<String, String> {
    let project_path = project_dir.join("project.json");
    let mut contents = contents.to_string();
    // Termination: `reconcile` is a function of the manifest and the files on
    // disk, and returns a manifest closed against them. The passes run in
    // epochs separated by copies into `packages/`, the only change to disk here.
    //
    // - Each name is copied at most once per closure (`copied`). A later request
    //   for a name already copied names a file `reconcile` has already read (a
    //   declared package's installed `.mfp` is what it reads first), so it is
    //   not copied again and is not progress. The names are those of `.mfp`
    //   files on disk, a finite set, so there are finitely many epochs.
    // - Within an epoch the disk is fixed. Its first pass ends closed, so no
    //   entry is orphaned; a later pass of the epoch only adds entries (the
    //   registry packages declared below, each needed by a declared package),
    //   which only adds requirements, so nothing is ever dropped and a declared
    //   registry ident is never undeclared again. So each registry-declaring
    //   pass of an epoch declares idents new to the epoch (`requested`), and the
    //   idents the import tables on disk name are finite.
    //
    // So the loop ends — no fixed budget, which a long but legitimate chain
    // would exceed. An ident needed again within one epoch contradicts the
    // second point and is an error naming it, rather than another pass.
    let mut copied: BTreeSet<String> = BTreeSet::new();
    let mut requested: BTreeSet<String> = BTreeSet::new();
    loop {
        let result = crate::manifest::closure::reconcile(project_dir, &contents)?;
        contents = result.contents;
        changes.added.extend(result.added);
        changes.removed.extend(result.removed);
        let mut copied_new = false;
        for (name, file) in &result.installs {
            if copied.insert(name.clone()) {
                super::pkg::install_local_package(project_dir, name, file)?;
                copied_new = true;
            }
        }
        if copied_new {
            // The copies make their import tables readable: a new epoch.
            requested.clear();
            continue;
        }
        if result.unresolved.is_empty() {
            return Ok(contents);
        }
        let (registry, local): (Vec<_>, Vec<_>) = result
            .unresolved
            .into_iter()
            .partition(|(_, need)| need.ident.contains('#'));
        if !local.is_empty() {
            let lines: Vec<String> = local
                .iter()
                .map(|(requirer, need)| {
                    format!(
                        "package `{requirer}` requires `{}`, which cannot be located from \
                         `{requirer}`; add it with `mfb pkg add`",
                        need.name
                    )
                })
                .collect();
            return Err(lines.join("\n"));
        }
        let again: Vec<String> = registry
            .iter()
            .filter(|(_, need)| !requested.insert(need.ident.clone()))
            .map(|(_, need)| format!("`{}`", need.ident))
            .collect();
        if !again.is_empty() {
            return Err(format!(
                "the dependency closure of '{}' does not converge: {} declared again without \
                 any new package installed",
                project_path.display(),
                again.join(", ")
            ));
        }
        for (requirer, need) in registry {
            let manifest = parse_project_json(&contents, &project_path)?;
            let dependency = registry_dependency_for(&need)?;
            contents = crate::manifest::package::project_json_with_package(
                &contents,
                &manifest,
                &dependency,
            )?;
            changes.added.push((need.name.clone(), requirer));
        }
    }
}

/// The `packages[]` entry for a registry package another declared package
/// imports: its requirer's version and pin, and the identKey of that release
/// verified for its publisher through the signed transfer chain from the
/// registry's name binding (trust-on-first-use, as `mfb pkg add` pins it;
/// plan-168-C).
// coverage:off — fetches `/index` and a blob; covered by the tests/ package-add
// integration harness.
fn registry_dependency_for(
    need: &crate::manifest::closure::Requirement,
) -> Result<crate::manifest::package::ProjectPackageDependency, String> {
    let (owner, package) = need
        .ident
        .split_once('#')
        .ok_or_else(|| format!("`{}` is not a registry ident", need.ident))?;
    let repo_url = client::repo_url_from_env();
    let paths = super::local_paths_for_repo(&repo_url)?;
    let index = client::fetch_index(&repo_url, &paths, owner, package)?;
    let version = need.version.trim_start_matches('=').to_string();
    let release = index
        .versions
        .iter()
        .find(|entry| entry.version == version)
        .ok_or_else(|| {
            format!(
                "registry has no version `{version}` of `{}` to anchor resolution",
                need.ident
            )
        })?;
    let blob = client::fetch_blob(&repo_url, &release.hash)?;
    let ident_key = match super::pkg::verify_registry_release(
        &repo_url,
        &paths,
        &index,
        release,
        &blob,
        mfb_repository::trust::Anchor::FirstUse,
    ) {
        Ok(Some(trusted)) => super::pkg::metadata_key(&trusted.key),
        Ok(None) => index.ident_key.clone(),
        Err(err) => {
            return Err(format!(
                "{}: refusing to declare `{}`@{version}: {err}",
                err.rule(),
                need.ident
            ))
        }
    };
    Ok(crate::manifest::package::ProjectPackageDependency {
        name: need.name.clone(),
        ident: need.ident.clone(),
        version,
        pin: need.pin,
        source: need.ident.clone(),
        ident_key,
        direct: Some(false),
        required_by: Some(Vec::new()),
    })
}

/// Commit an already-closed manifest (plan-60-B §4.2). Returns the text
/// written, which differs from `new_contents` by the pins resolution moved.
fn apply_closed_manifest(project_dir: &Path, new_contents: &str) -> Result<String, String> {
    let project_path = project_dir.join("project.json");
    let manifest = parse_project_json(new_contents, &project_path)?;

    // 2. No registry dependencies → the §4.3 path. `resolve()` cannot run on an
    //    empty dependency set and a synthesized empty lock cannot be installed
    //    (its `repoFingerprint` would be empty, which `install` rejects), so
    //    there is nothing to lock and the lock must go.
    if registry_dependency_count(&manifest) == 0 {
        fs::write(&project_path, new_contents)
            .map_err(|err| format!("failed to write '{}': {err}", project_path.display()))?;
        let lock = lock_path(project_dir);
        if lock.exists() {
            fs::remove_file(&lock)
                .map_err(|err| format!("failed to remove '{}': {err}", lock.display()))?;
        }
        return Ok(new_contents.to_string());
    }

    // Read the previous lock *before* resolving, so the diff below can compare
    // against it.
    let previous = read_lock(project_dir)?;

    // 3. Resolve. THIS MUST PRECEDE EVERY WRITE — it is the guarantee.
    let Resolution { lock, pin_changes } = resolve(&manifest)?;

    // plan-168-C: a pin is the verified identKey of the release it selects.
    // When the owner rotated, or the selected release comes from another owner
    // through a signed transfer, the pin moves with it — in the text, before
    // anything is written, so an offline `mfb build` checks the same key the
    // lock does.
    let mut contents = new_contents.to_string();
    for change in &pin_changes {
        contents = crate::manifest::package::project_json_with_updated_ident_key(
            &contents,
            &change.name,
            &change.ident_key,
        )?;
    }

    // 4-7. Resolution succeeded, so the change is viable; commit it.
    print_lock_diff(previous.as_ref(), &lock);
    fs::write(&project_path, &contents)
        .map_err(|err| format!("failed to write '{}': {err}", project_path.display()))?;
    write_lock(project_dir, &lock)?;
    for change in &pin_changes {
        if change.crossed_transfer {
            super::pkg::print_transfer_notice(&change.ident, &change.publisher);
        } else {
            println!(
                "notice: owner `{}` rotated their ident; updated the pinned identKey for `{}` to fingerprint {}",
                crate::terminal_safe::safe(&change.publisher),
                crate::terminal_safe::safe(&change.name),
                change.ident_fingerprint
            );
        }
    }
    install(project_dir)?;
    Ok(contents)
}

/// Is this dependency a **registry** resolution node?
///
/// Two conditions, and both matter:
///
/// - The ident must be `<owner>#<package>`. A bare name has no registry
///   coordinates to look up.
/// - The source must not be a `file://` URL. **This is the plan-60-C §5 fix.**
///   `add_package_from_file` copies the ident out of the `.mfp` *header*
///   (`add_package_from_file` in `src/cli/pkg/add.rs`), not out of the URL, so a package that was published
///   and then added by file carries a registry-shaped ident. Keying only on the
///   ident therefore admitted it as a resolver node, and `mfb pkg update`
///   silently replaced the user's local file with whatever version the registry
///   currently served — a different package than the one they added, with no
///   diagnostic. See `spike_file_added_package_with_registry_ident_survives_update`
///   in `tests/repo_acceptance.rs` for the reproduction.
///
/// A `file://` dependency still contributes to `projectHash`, so it still
/// requires the lock rewrite; it is simply not something the resolver selects.
///
/// **Single source of truth on purpose.** `resolve()`'s seeding and
/// `registry_dependency_count` must agree exactly: if they drift,
/// `apply_manifest_change` either calls `resolve()` on a set it will reject with
/// "declares no registry dependencies to resolve", or takes the zero-dependency
/// path while real dependencies still need locking.
pub(super) fn is_registry_dependency(
    dep: &crate::manifest::package::ProjectPackageDependency,
) -> bool {
    dep.ident.contains('#') && !dep.source.starts_with("file://")
}

/// How many registry dependencies does this manifest declare? See
/// [`is_registry_dependency`] for what counts and why.
pub(super) fn registry_dependency_count(
    manifest: &std::collections::HashMap<String, JsonValue>,
) -> usize {
    manifest
        .get("packages")
        .and_then(|value| value.get::<Vec<JsonValue>>())
        .into_iter()
        .flatten()
        .filter_map(project_package_dependency)
        .filter(is_registry_dependency)
        .count()
}
