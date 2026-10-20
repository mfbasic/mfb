//! The package resolver and `mfb.lock` writer (plan-10-B2).
//!
//! Resolution runs over the registry dependencies declared in `project.json`.
//! Each dependency is a node; the resolver picks, for every node, the highest
//! install-eligible version whose exported ABI is a **superset** of every
//! requirer's needs (`ABI_INDEX(V) ⊇ ABI_INDEX(anchor)`). A dependency is a
//! requirer of another when its compiled import edges name it; two requirers
//! that disagree on a symbol's hash are a diamond conflict, reported by name.
//! A `pin` dependency bypasses the search and takes its exact version.
//!
//! `mfb pkg update` re-resolves and writes a reviewable `mfb.lock`; `mfb pkg
//! install` applies a current lock by fetching blobs by hash only — never
//! resolving, never hitting `/index`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use tinyjson::JsonValue;

use crate::binary_repr;
use crate::json::json_string;
use crate::manifest::package::project_package_dependency;
use crate::manifest::{parse_project_json, validate_packages_array};

use mfb_repository::client;
use mfb_repository::server::IndexResponse;
// From a part, `super` is `cli::resolve`; `closure.rs` spells `super::pkg` and
// `super::local_paths_for_repo`, which live in `cli`.
use super::{local_paths_for_repo, pkg};

mod closure;
mod drift;
mod lockfile;
pub(crate) use closure::*;
pub(crate) use drift::*;
pub(crate) use lockfile::*;

struct Requirer {
    who: String,
    required: BTreeMap<String, String>,
    pin: Option<String>,
}

struct Node {
    name: String,
    ident: String,
    index: IndexResponse,
    requested: String,
    requirers: BTreeMap<String, Requirer>,
    selected: Option<String>,
    /// The `identKey` `project.json` pins for this dependency: the anchor the
    /// selected release is verified from (plan-168-C). Empty for no pin.
    pinned_ident_key: String,
}

/// A dependency whose verified `identKey` differs from the one `project.json`
/// pins: the owner rotated, or the selected release was published by another
/// owner through a signed transfer (plan-168-C).
pub(crate) struct PinChange {
    pub(crate) name: String,
    pub(crate) ident: String,
    pub(crate) ident_key: String,
    pub(crate) ident_fingerprint: String,
    pub(crate) publisher: String,
    pub(crate) crossed_transfer: bool,
}

/// What [`resolve`] produces: the lock, and the pins it moves.
pub(crate) struct Resolution {
    pub(crate) lock: Lock,
    pub(crate) pin_changes: Vec<PinChange>,
}

/// `mfb pkg update`: close the dependency set (bug-628), re-resolve the registry
/// dependencies and write `mfb.lock`, printing a diff against the previous lock.
pub(crate) fn update(project_dir: &Path) -> Result<(), String> {
    let (_manifest, contents) = read_manifest(project_dir)?;

    // bug-628: `apply_manifest_change` declares every dependency the declared
    // packages import, drops indirect entries nothing requires, and rewrites
    // `direct`/`requiredBy` — for every project, registry dependencies or not.
    //
    // plan-60-E (input from plan-60-C Corrections #3): a project with no
    // registry dependencies has nothing to resolve. `resolve()` would error
    // with "declares no registry dependencies to resolve", which is the wrong
    // answer for a project that legitimately has only `file://` packages — or
    // none at all. `apply_manifest_change`'s zero-registry path applies
    // plan-60-B §4.3's policy instead: nothing to lock, so a stale lock goes.
    apply_manifest_change(project_dir, &contents)?;

    let (manifest, _contents) = read_manifest(project_dir)?;
    if registry_dependency_count(&manifest) == 0 {
        println!("No registry dependencies to resolve; mfb.lock is not needed.");
    } else if let Some(lock) = read_lock(project_dir)? {
        println!(
            "Wrote {} resolved package(s) to mfb.lock",
            lock.packages.len()
        );
    }
    Ok(())
}

/// `mfb pkg install`: apply a current `mfb.lock` — fetch each locked blob by
/// hash, verify the full §3.5 chain against the locked ident key, and install.
/// Never resolves and never calls `/index`.
pub(crate) fn install(project_dir: &Path) -> Result<(), String> {
    let (manifest, _contents) = read_manifest(project_dir)?;
    let Some(lock) = read_lock(project_dir)? else {
        // plan-60-F §4.6: distinguish "no lock because nothing is declared" from
        // "no lock but dependencies are declared". The first is a correctly
        // configured project — after `mfb pkg remove` took the last dependency,
        // or in a fresh `mfb init` — and reporting it as an error told the user
        // to run a command that would do nothing.
        if registry_dependency_count(&manifest) == 0 {
            println!("nothing to install: project.json declares no registry dependencies");
            return Ok(());
        }
        return Err("no mfb.lock; run `mfb pkg update` to resolve dependencies first".to_string());
    };
    // A drifted request set means the lock may no longer describe the project.
    // Rather than refusing on the opaque hash alone, diff the fields the lock
    // records and decide per class (plan-60-D §4.1): a moved ABI floor on a
    // floating dependency is a warning, everything else is fatal.
    let current_hash = crate::audit::project_hash(&manifest);
    if lock.project_hash != current_hash {
        report_drift(&manifest, &lock)?;
    }

    let repo_url = client::repo_url_from_env();
    let paths = super::local_paths_for_repo(&repo_url)?;
    // If a signed-metadata root is pinned (plan-10-C2), verify the chain and
    // that it delegates this registry's server key before installing.
    client::verify_pinned_metadata(&repo_url, &paths)?;
    // The pinned registry key must be the one the lock was resolved against.
    let server_key = mfb_repository::local::read_pinned_server_key(&paths).map_err(|_| {
        "no pinned registry key; run `mfb repo auth <owner>` against the registry first".to_string()
    })?;
    if mfb_repository::crypto::fingerprint(&server_key) != lock.repo_fingerprint {
        return Err(
            "pinned registry key does not match the repoFingerprint in mfb.lock; refusing to install"
                .to_string(),
        );
    }

    let packages_dir = project_dir.join("packages");
    fs::create_dir_all(&packages_dir)
        .map_err(|err| format!("failed to create '{}': {err}", packages_dir.display()))?;
    for package in &lock.packages {
        let blob = client::fetch_blob(&repo_url, &package.hash)?;
        // bug-491: bind the fetched artifact to the LOCK before staging it.
        //
        // The plan-23 §3.5 chain that `install_verified_package` runs below
        // proves the blob is self-consistent and signed by the pinned owner — and
        // nothing more. `verify_attestation` binds the signature to the
        // `ident`/`version` it reads out of THAT SAME FILE, so an owner's other
        // artifact passes every one of those checks. Combined with the unsigned
        // `/index` version list (bug-189), a registry that maps `version -> hash`
        // freely could serve any artifact the owner ever signed for a requested
        // version and have it accepted here.
        //
        // The sibling `mfb pkg add` path already compares the downloaded header's
        // ident (`pkg.rs`, `header.ident != full_ident`); this is the same
        // comparison at the other entry point, widened to the three fields the
        // lock actually pins. `selected` is the lock's resolved version, NOT the
        // manifest's `version` — that one is an ABI floor for an unpinned
        // dependency and would reject a legitimate newer selection.
        let header = mfb_repository::package::parse_mfp_package(&blob)
            .map_err(|err| format!("registry returned a malformed package: {err}"))?;
        if header.ident != package.ident
            || header.version != package.selected
            || header.name != package.name
        {
            return Err(format!(
                "package `{}` does not match mfb.lock: the registry served \
                 `{}` version {} (name `{}`), but the lock pins `{}` version {} \
                 (name `{}`); refusing to install",
                package.name,
                header.ident,
                header.version,
                header.name,
                package.ident,
                package.selected,
                package.name
            ));
        }
        // `package.name` comes from `mfb.lock`, which an attacker who ships a repo
        // controls: stage the untrusted blob under an exclusively created name
        // inside `packages/`, verify it there, and only then rename it into place.
        super::install_verified_package(
            &packages_dir,
            &package.name,
            &blob,
            Some(&package.ident_key),
        )
        .map_err(|detail| {
            format!(
                "refusing to install `{}`@{}: {detail}",
                package.name, package.selected
            )
        })?;
        // plan-48-B §4.4: the `.mfp` verified, so download every vendor blob its
        // section-10 table names into `packages/<name>.vendor/`.
        super::pkg::install_vendor_blobs(&repo_url, project_dir, &package.name)?;
        // bug-489: all three are registry-authored data fields, sanitized at
        // the print site for the same reason as `Release State` above.
        println!(
            "Installed {} {} ({})",
            crate::terminal_safe::safe(&package.name),
            crate::terminal_safe::safe(&package.selected),
            crate::terminal_safe::safe(&package.state)
        );
    }
    Ok(())
}

/// Run the §8.3 resolver and assemble a [`Lock`]. Public for tests.
// coverage:off — drives the registry (fetch_index/fetch_blob/fetch_checkpoint)
// across a dependency graph; the pure selection/version logic it calls
// (select_node, is_superset, compare_versions) is unit-tested directly, and the
// full resolve is covered by the tests/ package-resolution integration harness.
pub(crate) fn resolve(
    manifest: &std::collections::HashMap<String, JsonValue>,
) -> Result<Resolution, String> {
    let project_name = manifest
        .get("name")
        .and_then(|value| value.get::<String>())
        .cloned()
        .unwrap_or_else(|| "project".to_string());
    let who_project = format!("project `{project_name}`");

    let repo_url = client::repo_url_from_env();
    let paths = super::local_paths_for_repo(&repo_url)?;

    // Seed one node per registry dependency (ident `<owner>#<package>`).
    let mut nodes: BTreeMap<String, Node> = BTreeMap::new();
    let registry_deps: Vec<_> = manifest
        .get("packages")
        .and_then(|value| value.get::<Vec<JsonValue>>())
        .into_iter()
        .flatten()
        .filter_map(project_package_dependency)
        .filter(is_registry_dependency)
        .collect();
    if registry_deps.is_empty() {
        return Err("project.json declares no registry dependencies to resolve".to_string());
    }

    for dep in &registry_deps {
        let (owner, package) = dep.ident.split_once('#').unwrap();
        let index = client::fetch_index(&repo_url, &paths, owner, package)?;
        let anchor = index
            .versions
            .iter()
            .find(|version| version.version == dep.version)
            .ok_or_else(|| {
                format!(
                    "registry has no version `{}` of `{}` to anchor resolution",
                    dep.version, dep.ident
                )
            })?;
        let mut requirers = BTreeMap::new();
        requirers.insert(
            who_project.clone(),
            Requirer {
                who: who_project.clone(),
                required: anchor.abi_map(),
                pin: if dep.pin {
                    Some(dep.version.clone())
                } else {
                    None
                },
            },
        );
        nodes.insert(
            dep.ident.clone(),
            Node {
                name: dep.name.clone(),
                ident: dep.ident.clone(),
                index,
                requested: dep.version.clone(),
                requirers,
                selected: None,
                pinned_ident_key: dep.ident_key.clone(),
            },
        );
    }

    // Fixpoint: select each node, then recompute the edges it contributes to
    // sibling nodes from its selected version's import table. Bounded so a
    // pathological graph errors instead of spinning.
    let mut blob_cache: BTreeMap<String, Vec<(String, BTreeMap<String, String>, Option<String>)>> =
        BTreeMap::new();
    // The selected releases' bytes, by hash, for the trust check below.
    let mut release_blobs: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let node_idents: Vec<String> = nodes.keys().cloned().collect();
    let mut converged = false;
    for _ in 0..(node_idents.len() * node_idents.len() + 4) {
        let mut changed = false;
        for ident in &node_idents {
            let selection = select_node(nodes.get(ident).unwrap())?;
            let previous = nodes.get(ident).unwrap().selected.clone();
            if previous.as_deref() == Some(selection.version.as_str()) {
                continue;
            }
            changed = true;
            nodes.get_mut(ident).unwrap().selected = Some(selection.version.clone());

            // Drop stale edges this node contributed, then re-add from the new
            // selection's imports.
            for other in &node_idents {
                if other != ident {
                    nodes.get_mut(other).unwrap().requirers.remove(ident);
                }
            }
            let imports = load_import_edges(
                &repo_url,
                &selection.hash,
                &mut blob_cache,
                &mut release_blobs,
            )?;
            for (imported_ident, required, pin) in imports {
                if imported_ident == *ident {
                    continue;
                }
                if let Some(target) = nodes.get_mut(&imported_ident) {
                    target.requirers.insert(
                        ident.clone(),
                        Requirer {
                            who: format!("`{ident}`"),
                            required,
                            pin,
                        },
                    );
                }
            }
        }
        if !changed {
            converged = true;
            break;
        }
    }

    // Non-convergence means the import-edge selection is still oscillating after
    // the bounded number of passes; error out instead of assembling an mfb.lock
    // from the last (unstable) selection (bug-219 — the comment above claimed this
    // already happened, but the loop merely fell through).
    if !converged {
        return Err(
            "dependency resolution did not converge: the registry import graph's version \
             selection oscillates. Pin the conflicting dependencies to break the cycle."
                .to_string(),
        );
    }

    // Every node must have converged to a selection.
    let mut packages = Vec::new();
    let mut pin_changes = Vec::new();
    for ident in &node_idents {
        let node = nodes.get(ident).unwrap();
        let selection = select_node(node)?;
        let version = node
            .index
            .versions
            .iter()
            .find(|version| version.version == selection.version)
            .expect("selected version exists");
        // plan-168-C: the lock records the identKey of THIS release, verified
        // for its publisher from the project's pin (or, with none, the
        // registry's name binding) through the signed transfer chain. The
        // index's current key is the wrong anchor for any release signed before
        // a rotation or by another owner.
        let blob = match release_blobs.get(&selection.hash) {
            Some(blob) => blob.clone(),
            None => client::fetch_blob(&repo_url, &selection.hash)?,
        };
        let anchor = if node.pinned_ident_key.is_empty() {
            mfb_repository::trust::Anchor::FirstUse
        } else {
            mfb_repository::trust::Anchor::Pinned(
                mfb_repository::package::decode_metadata_key(&node.pinned_ident_key, "identKey")
                    .map_err(|err| {
                        format!(
                            "the identKey pinned for `{}` is malformed: {err}",
                            node.name
                        )
                    })?,
            )
        };
        let (ident_key, ident_fingerprint) = match super::pkg::verify_registry_release(
            &repo_url,
            &paths,
            &node.index,
            version,
            &blob,
            anchor,
        ) {
            Ok(Some(trusted)) => {
                let ident_key = super::pkg::metadata_key(&trusted.key);
                if ident_key != node.pinned_ident_key {
                    pin_changes.push(PinChange {
                        name: node.name.clone(),
                        ident: node.ident.clone(),
                        ident_key: ident_key.clone(),
                        ident_fingerprint: mfb_repository::crypto::fingerprint(&trusted.key),
                        publisher: trusted.publisher.clone(),
                        crossed_transfer: trusted.crossed_transfer,
                    });
                }
                (ident_key, mfb_repository::crypto::fingerprint(&trusted.key))
            }
            // An unsigned release carries no key; `install` classifies it.
            Ok(None) => (
                node.index.ident_key.clone(),
                node.index.ident_fingerprint.clone(),
            ),
            Err(err) => {
                return Err(format!(
                    "{}: refusing to lock `{}`@{}: {err}",
                    err.rule(),
                    node.ident,
                    selection.version
                ))
            }
        };
        packages.push(LockedPackage {
            name: node.name.clone(),
            ident: node.ident.clone(),
            requested: node.requested.clone(),
            selected: selection.version.clone(),
            hash: selection.hash.clone(),
            ident_key,
            ident_fingerprint,
            state: version.state.clone(),
        });
    }
    packages.sort_by(|a, b| a.name.cmp(&b.name));

    // Consistency-proved rather than merely monotonic (bug-276 R2): a forked log
    // that keeps growing satisfies the signature and size/root checks on their
    // own, so the RFC-6962 proof against the pinned head is what makes this an
    // append-only guarantee. Since bug-582 that proof is demanded by the single
    // pin-advance primitive, so `fetch_checkpoint` is the same call; the name
    // here records why the log is being contacted.
    let checkpoint = client::verify_log_consistency(&repo_url, &paths)?;
    let repo_fingerprint = nodes
        .values()
        .next()
        .map(|node| node.index.server_fingerprint.clone())
        .unwrap_or_default();

    Ok(Resolution {
        lock: Lock {
            project_hash: crate::audit::project_hash(manifest),
            repo_fingerprint,
            checkpoint_size: checkpoint.size,
            checkpoint_root: checkpoint.root_hash,
            packages,
        },
        pin_changes,
    })
}

struct Selection {
    version: String,
    hash: String,
}

/// Select a node's version: the union of every requirer's needs (a hash
/// disagreement is a diamond conflict), then the exact pin or the highest
/// install-eligible superset version.
fn select_node(node: &Node) -> Result<Selection, String> {
    // Union the required symbol hashes; a disagreement is a diamond conflict.
    let mut required: BTreeMap<String, (String, String)> = BTreeMap::new();
    for requirer in node.requirers.values() {
        for (symbol, hash) in &requirer.required {
            if let Some((existing_hash, existing_who)) = required.get(symbol) {
                if existing_hash != hash {
                    return Err(format!(
                        "diamond conflict on `{}`: requirer {} needs symbol `{symbol}` with a \
                         different ABI than requirer {}; no single version can satisfy both",
                        node.ident, existing_who, requirer.who
                    ));
                }
            } else {
                required.insert(symbol.clone(), (hash.clone(), requirer.who.clone()));
            }
        }
    }
    let required: BTreeMap<String, String> = required
        .into_iter()
        .map(|(symbol, (hash, _who))| (symbol, hash))
        .collect();

    // A pin takes its exact version (any non-blocked state); pins must agree.
    let pins: Vec<&String> = node
        .requirers
        .values()
        .filter_map(|requirer| requirer.pin.as_ref())
        .collect();
    if let Some(first) = pins.first() {
        if pins.iter().any(|pin| pin != first) {
            return Err(format!(
                "conflicting pins for `{}`: requirers pin different exact versions",
                node.ident
            ));
        }
        let version = node
            .index
            .versions
            .iter()
            .find(|version| &&version.version == first)
            .ok_or_else(|| {
                format!(
                    "pinned version `{first}` of `{}` is not published",
                    node.ident
                )
            })?;
        if version.state == "blocked" || version.state == "legal-tombstoned" {
            return Err(format!(
                "pinned version `{first}` of `{}` is {} and cannot be selected",
                node.ident, version.state
            ));
        }
        return Ok(Selection {
            version: version.version.clone(),
            hash: version.hash.clone(),
        });
    }

    // Floating: the highest install-eligible superset version.
    let mut candidates: Vec<_> = node
        .index
        .versions
        .iter()
        .filter(|version| super::pkg::state_is_floating_eligible(&version.state))
        .filter(|version| is_superset(&version.abi_map(), &required))
        .collect();
    candidates.sort_by(|a, b| compare_versions(&b.version, &a.version));
    let chosen = candidates.first().ok_or_else(|| {
        let who: Vec<&str> = node.requirers.values().map(|r| r.who.as_str()).collect();
        format!(
            "no install-eligible version of `{}` satisfies its requirers ({})",
            node.ident,
            who.join(", ")
        )
    })?;
    Ok(Selection {
        version: chosen.version.clone(),
        hash: chosen.hash.clone(),
    })
}

/// Whether `exports` provides every `(symbol, hash)` in `required`.
pub(crate) fn is_superset(
    exports: &BTreeMap<String, String>,
    required: &BTreeMap<String, String>,
) -> bool {
    required
        .iter()
        .all(|(symbol, hash)| exports.get(symbol) == Some(hash))
}

/// Download a selected version's blob (cached by hash) and read its import
/// edges: `(imported ident, used-symbol hashes, pin)`.
// coverage:off — fetches a blob from the registry (fetch_blob); reached only
// from the network-bound resolver, covered by the tests/ integration harness.
#[allow(clippy::type_complexity)]
fn load_import_edges(
    repo_url: &str,
    hash: &str,
    cache: &mut BTreeMap<String, Vec<(String, BTreeMap<String, String>, Option<String>)>>,
    blobs: &mut BTreeMap<String, Vec<u8>>,
) -> Result<Vec<(String, BTreeMap<String, String>, Option<String>)>, String> {
    if let Some(edges) = cache.get(hash) {
        return Ok(edges.clone());
    }
    let blob = client::fetch_blob(repo_url, hash)?;
    blobs.insert(hash.to_string(), blob.clone());
    // Read the edges straight out of the blob. Staging it at a `hash`-derived
    // path in the shared temp dir was both predictable (a pre-planted symlink
    // there is written through) and traversable (a non-hex `hash` from the
    // registry index escapes `temp_dir`).
    let info = binary_repr::package_info_from_mfp(&blob)
        .map_err(|err| format!("failed to read resolver blob: {err}"))?;
    let edges = info
        .imports
        .into_iter()
        .filter(|import| import.package_ident.contains('#'))
        .map(|import| {
            let required = import
                .used_symbols
                .into_iter()
                .map(|symbol| (symbol.name, symbol.sig_hash))
                .collect();
            let pin = if import.pin {
                Some(import.version.clone())
            } else {
                None
            };
            (import.package_ident, required, pin)
        })
        .collect::<Vec<_>>();
    cache.insert(hash.to_string(), edges.clone());
    Ok(edges)
}

/// Compare one dotted component of a version.
///
/// A component that is not a `u64` (`"2x"`, or a number too large to fit) must not
/// be coerced to `0` — that silently ranks `"2x.0.0"` as `"0.0.0"`. Numeric
/// components compare as numbers; a numeric component outranks a malformed one;
/// two malformed components compare lexically, so the order is total and stable.
fn compare_version_components(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (a.parse::<u64>(), b.parse::<u64>()) {
        (Ok(left), Ok(right)) => left.cmp(&right),
        (Ok(_), Err(_)) => Ordering::Greater,
        (Err(_), Ok(_)) => Ordering::Less,
        (Err(_), Err(_)) => a.cmp(b),
    }
}

/// Compare two dotted version strings: numeric components compared as numbers,
/// a `-prerelease` suffix sorting below the same release.
pub(crate) fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    fn split(version: &str) -> (Vec<&str>, Option<&str>) {
        let (release, pre) = match version.split_once('-') {
            Some((release, pre)) => (release, Some(pre)),
            None => (version, None),
        };
        (release.split('.').collect(), pre)
    }
    let (a_nums, a_pre) = split(a);
    let (b_nums, b_pre) = split(b);
    let width = a_nums.len().max(b_nums.len());
    for index in 0..width {
        // A version shorter than the other is padded with implicit zeroes, so
        // `1.2` and `1.2.0` are the same version.
        let left = a_nums.get(index).copied().unwrap_or("0");
        let right = b_nums.get(index).copied().unwrap_or("0");
        match compare_version_components(left, right) {
            Ordering::Equal => {}
            other => return other,
        }
    }
    // Equal release: a version WITHOUT a pre-release outranks one with.
    match (a_pre, b_pre) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(left), Some(right)) => left.cmp(right),
    }
}

fn read_manifest(
    project_dir: &Path,
) -> Result<(std::collections::HashMap<String, JsonValue>, String), String> {
    let project_path = project_dir.join("project.json");
    let contents = fs::read_to_string(&project_path)
        .map_err(|err| format!("failed to read '{}': {err}", project_path.display()))?;
    let manifest = parse_project_json(&contents, &project_path)?;
    validate_packages_array(&manifest)?;
    Ok((manifest, contents))
}

#[cfg(test)]
mod tests;
