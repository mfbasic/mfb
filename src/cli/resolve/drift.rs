//! plan-176-L: the dependency resolver's drift code. Resolution itself (`update`,
//! `install`, `resolve`, `select_node`) lives in `cli/resolve/mod.rs`.

use super::*;

/// How a single dependency's `project.json` entry differs from its `mfb.lock`
/// entry (plan-60-D §4.1).
///
/// Only `FloorMoved` is recoverable. Under `pin: false` the manifest's `version`
/// is an ABI *floor*, so moving it does not invalidate the locked selection —
/// the lock may still satisfy the new floor, and even when it does not, the
/// honest outcome is to install what was locked and say so. Every other class
/// means the lock no longer describes the project.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum DriftClass {
    /// Declared in `project.json`, absent from the lock.
    Added,
    /// Present in the lock, no longer declared.
    Removed,
    /// `pin: false` and the declared version (the ABI floor) moved.
    FloorMoved {
        declared: String,
        locked: String,
        selected: String,
    },
    /// `pin: true` and the declared version moved.
    PinMoved { declared: String, locked: String },
    /// Same ident, different package name.
    Renamed { declared: String, locked: String },
}

/// Diff a manifest's registry dependencies against a lock's packages, matched on
/// `ident` (plan-60-D §4.1).
///
/// Collects **all** differences rather than returning the first, so a project
/// with three drifted dependencies reports three lines. Pure: no I/O, no
/// registry. An empty result when the caller has already observed a
/// `projectHash` mismatch is the `Unattributable` case (§4.2) — see
/// `install`, which owns that decision because only it knows a mismatch occurred.
pub(crate) fn classify_drift(
    manifest: &std::collections::HashMap<String, JsonValue>,
    lock_packages: &[LockedPackage],
) -> Vec<(String, DriftClass)> {
    let declared: Vec<_> = manifest
        .get("packages")
        .and_then(|value| value.get::<Vec<JsonValue>>())
        .into_iter()
        .flatten()
        .filter_map(project_package_dependency)
        .filter(is_registry_dependency)
        .collect();

    let mut drift = Vec::new();

    for dep in &declared {
        match lock_packages.iter().find(|entry| entry.ident == dep.ident) {
            None => drift.push((dep.ident.clone(), DriftClass::Added)),
            Some(entry) => {
                if dep.name != entry.name {
                    drift.push((
                        dep.ident.clone(),
                        DriftClass::Renamed {
                            declared: dep.name.clone(),
                            locked: entry.name.clone(),
                        },
                    ));
                }
                if dep.version != entry.requested {
                    drift.push((
                        dep.ident.clone(),
                        if dep.pin {
                            DriftClass::PinMoved {
                                declared: dep.version.clone(),
                                locked: entry.requested.clone(),
                            }
                        } else {
                            DriftClass::FloorMoved {
                                declared: dep.version.clone(),
                                locked: entry.requested.clone(),
                                selected: entry.selected.clone(),
                            }
                        },
                    ));
                }
            }
        }
    }

    for entry in lock_packages {
        if !declared.iter().any(|dep| dep.ident == entry.ident) {
            drift.push((entry.ident.clone(), DriftClass::Removed));
        }
    }

    drift
}

/// Apply plan-60-D §4.1's outcome rule to a `projectHash` mismatch.
///
/// Called ONLY when the hash has already been observed to differ — that fact is
/// what makes an empty classification meaningful (§4.2).
///
/// Warnings go to stderr and return `Ok`; anything else returns `Err` naming
/// every offending dependency, so a project with three problems reports three
/// rather than one per re-run.
pub(super) fn report_drift(
    manifest: &std::collections::HashMap<String, JsonValue>,
    lock: &Lock,
) -> Result<(), String> {
    let drift = classify_drift(manifest, &lock.packages);

    // §4.2: the hash differs but nothing the lock records does. `pin` and
    // `source` are covered by `projectHash` yet absent from the lock, so a flip
    // of either lands here. Refusing is the conservative direction: proceeding
    // would install the old locked blob after a `source` change pointed the
    // dependency somewhere else entirely.
    if drift.is_empty() {
        return Err(
            "mfb.lock does not match project.json, but the difference is not in a field the \
             lock records (most likely `pin` or `source` changed); run `mfb pkg update`"
                .to_string(),
        );
    }

    let mut fatal = Vec::new();
    let mut warnings = Vec::new();
    for (ident, class) in &drift {
        match class {
            DriftClass::FloorMoved {
                declared,
                locked,
                selected,
            } => warnings.push(format!(
                "warning: {ident} is floating and project.json now declares {declared} as its ABI \
                 floor, but mfb.lock was resolved against {locked} and selects {selected}. \
                 Installing the locked selection. Run `mfb pkg update` to re-resolve."
            )),
            DriftClass::PinMoved { declared, locked } => fatal.push(format!(
                "{ident} is pinned to {declared} in project.json but mfb.lock records {locked}. \
                 Run `mfb pkg update` to re-resolve, or restore the pinned version."
            )),
            DriftClass::Added => fatal.push(format!(
                "{ident} is declared in project.json but absent from mfb.lock. \
                 Run `mfb pkg update`."
            )),
            DriftClass::Removed => fatal.push(format!(
                "{ident} is recorded in mfb.lock but no longer declared in project.json. \
                 Run `mfb pkg update`."
            )),
            DriftClass::Renamed { declared, locked } => fatal.push(format!(
                "{ident} is named {declared} in project.json but {locked} in mfb.lock. \
                 Run `mfb pkg update`."
            )),
        }
    }

    // Report every fatal class, not just the first — re-running to discover one
    // more problem each time is the behavior this letter replaces.
    if !fatal.is_empty() {
        return Err(fatal.join("\n"));
    }
    for warning in warnings {
        eprintln!("{warning}");
    }
    Ok(())
}
