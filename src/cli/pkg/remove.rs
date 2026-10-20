//! plan-176-L: `mfb pkg` — the remove commands. The dispatcher `run_pkg_command` and the
//! shared `use` header live in `cli/pkg/mod.rs`.

use super::*;

/// `mfb pkg add <target>`: a `file://…​.mfp` URL copies a local package (the
/// original path, kept as a `source: "file:"` special case), while a
/// `<owner>#<package>[@version]` ident installs from the configured registry
/// (plan-10-A) — pinning the registry-vouched identKey, downloading the blob,
/// and verifying the full plan-23 §3.5 chain before it is installed.
/// Parse `remove`'s arguments: exactly one positional plus `--yes` (§4.1).
pub(super) fn parse_remove_options(args: &[String]) -> Result<(String, bool), PkgCommandError> {
    let mut target: Option<String> = None;
    let mut assume_yes = false;
    for arg in args {
        match arg.as_str() {
            "--yes" => assume_yes = true,
            flag if flag.starts_with("--") => {
                return Err(PkgCommandError::Usage(format!("unknown flag `{flag}`")));
            }
            value => {
                if target.is_some() {
                    return Err(PkgCommandError::Usage(
                        "mfb pkg remove accepts exactly one <owner>#<package>".to_string(),
                    ));
                }
                target = Some(value.to_string());
            }
        }
    }
    let target = target.ok_or_else(|| {
        PkgCommandError::Usage(format!(
            "mfb pkg remove requires <owner>#<package>\n\n{PKG_HELP}"
        ))
    })?;
    Ok((target, assume_yes))
}

/// `mfb pkg remove <owner>#<pkg> [--yes]` (plan-60-F).
///
/// Removes the target **and every package that transitively imports it**. The
/// cascade is not a convenience: `resolve()` silently drops an import edge
/// naming an undeclared ident, so removing only the named package would leave a
/// dangling import that resolves clean and fails at build time.
pub(super) fn run_remove(project_dir: &Path, target: &str, assume_yes: bool) -> Result<(), String> {
    let project_path = project_dir.join("project.json");
    let contents = fs::read_to_string(&project_path)
        .map_err(|err| format!("failed to read '{}': {err}", project_path.display()))?;
    let manifest = parse_project_json(&contents, &project_path)?;
    validate_packages_array(&manifest)?;

    let declared: Vec<_> = manifest
        .get("packages")
        .and_then(|value| value.get::<Vec<JsonValue>>())
        .into_iter()
        .flatten()
        .filter_map(project_package_dependency)
        .filter(|dep| dep.ident.contains('#'))
        .collect();

    if !declared.iter().any(|dep| dep.ident == target) {
        return Err(format!("`{target}` is not declared in project.json"));
    }

    // §4.2 steps 1-3: read each installed package's import table to build the
    // edge map the closure reverses.
    let mut imports: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    for dep in &declared {
        let mfp = project_dir
            .join("packages")
            .join(format!("{}.mfp", dep.name));
        if !mfp.is_file() {
            // §4.4: a missing .mfp means the closure CANNOT be trusted — it may
            // omit a package that imports the target. `--yes` deliberately does
            // not bypass this; it is a correctness gate, not a confirmation.
            // Proceeding would print a confident list, remove too little, and
            // leave exactly the dangling-import state the cascade prevents.
            return Err(format!(
                "cannot determine what depends on {target} — {} is declared in project.json but \
                 not installed ({} is missing). Run `mfb pkg install` first.",
                dep.ident,
                mfp.display()
            ));
        }
        let info = binary_repr::read_package_info(&mfp)?;
        imports.insert(
            dep.ident.clone(),
            info.imports
                .iter()
                .map(|import| import.package_ident.clone())
                .filter(|ident| ident.contains('#'))
                .collect(),
        );
    }

    let closure = removal_closure(target, &imports);

    // §4.3: no prompt when only the named package goes — the user named it and
    // nothing else is affected.
    if closure.len() > 1 {
        println!("Removing {target} will also remove packages that import it:");
        println!();
        for (ident, because) in &closure {
            match because {
                None => println!("  {ident:<24}(named)"),
                Some(importer) => println!("  {ident:<24}imports {importer}"),
            }
        }
        println!();
        let question = format!("Remove all {} packages?", closure.len());
        if !super::confirm(&question, assume_yes)? {
            println!("Nothing removed.");
            return Ok(());
        }
    }

    let removed_idents: Vec<&str> = closure.iter().map(|(ident, _)| ident.as_str()).collect();
    let removed_names: Vec<String> = declared
        .iter()
        .filter(|dep| removed_idents.contains(&dep.ident.as_str()))
        .map(|dep| dep.name.clone())
        .collect();

    let updated = project_json_without_packages(&contents, &removed_idents)?;
    super::resolve::apply_manifest_change(project_dir, &updated)?;

    // §4.5: cleanup runs ONLY after resolution succeeded. If it had failed,
    // nothing above this line touched the working tree.
    for name in &removed_names {
        remove_installed_package_files(project_dir, name);
    }

    println!(
        "Removed {} package(s): {}",
        removed_names.len(),
        removed_idents.join(", ")
    );
    Ok(())
}

/// Delete an uninstalled package's files (plan-60-F §4.5).
///
/// Missing is not an error — the goal state is "absent" and it is already met.
/// A deletion failure is a **warning**, not a failure: `project.json` and
/// `mfb.lock` are already consistent at this point, so failing the command would
/// misreport a completed removal as failed. Name the path so it can be cleaned
/// up by hand.
pub(crate) fn remove_installed_package_files(project_dir: &Path, name: &str) {
    let mfp = project_dir.join("packages").join(format!("{name}.mfp"));
    if mfp.exists() {
        if let Err(err) = fs::remove_file(&mfp) {
            eprintln!("warning: failed to delete '{}': {err}", mfp.display());
        }
    }
    // Reuse the helper rather than re-deriving the path, so the two stay in step.
    let vendor = crate::manifest::libraries::imported_vendor_dir(project_dir, name);
    if vendor.exists() {
        if let Err(err) = fs::remove_dir_all(&vendor) {
            eprintln!("warning: failed to delete '{}': {err}", vendor.display());
        }
    }
}

/// Transitive reverse-dependency closure from `target` (plan-60-F §4.2 step 4).
///
/// `imports` maps an ident to the idents it imports. Returns each ident to
/// remove paired with the **direct** importer that pulled it in — `None` for the
/// named target — so a multi-level cascade can be explained line by line rather
/// than dumped as a flat set the user has to reverse-engineer.
///
/// Worklist + visited set, deliberately: an import cycle between two packages
/// must terminate, and a diamond must yield each ident exactly once. Recursion
/// would do neither without the same bookkeeping.
///
/// Pure — no filesystem, no registry — because the cascade's correctness is the
/// whole value of `remove`, and it should be testable without either.
pub(super) fn removal_closure(
    target: &str,
    imports: &std::collections::BTreeMap<String, Vec<String>>,
) -> Vec<(String, Option<String>)> {
    // Reverse the edges: ident -> everything that imports it.
    let mut importers: std::collections::BTreeMap<&str, Vec<&str>> =
        std::collections::BTreeMap::new();
    for (importer, imported) in imports {
        for dependency in imported {
            importers
                .entry(dependency.as_str())
                .or_default()
                .push(importer.as_str());
        }
    }

    let mut closure = vec![(target.to_string(), None)];
    let mut visited: std::collections::BTreeSet<&str> = [target].into_iter().collect();
    let mut queue = std::collections::VecDeque::from([target]);

    while let Some(current) = queue.pop_front() {
        for importer in importers.get(current).into_iter().flatten() {
            // `visited` is what makes a cycle terminate and a diamond yield one
            // entry: an ident already scheduled is never scheduled again.
            if visited.insert(importer) {
                closure.push((importer.to_string(), Some(current.to_string())));
                queue.push_back(importer);
            }
        }
    }
    closure
}
