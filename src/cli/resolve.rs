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
use crate::json_string;
use crate::manifest::package::project_package_dependency;
use crate::manifest::{parse_project_json, validate_packages_array};

use mfb_repository::client;
use mfb_repository::server::IndexResponse;

const LOCKFILE_VERSION: i64 = 1;

/// One resolved dependency written to `mfb.lock`.
pub(crate) struct LockedPackage {
    pub(crate) name: String,
    pub(crate) ident: String,
    pub(crate) requested: String,
    pub(crate) selected: String,
    pub(crate) hash: String,
    /// The pinned owner ident key (metadata form) — the trust anchor `install`
    /// verifies against offline (plan-23 §3.5); no signing-key status/window.
    pub(crate) ident_key: String,
    pub(crate) ident_fingerprint: String,
    pub(crate) state: String,
}

/// The resolved lock: the registry it targets, the pinned log checkpoint, and
/// the selected packages.
pub(crate) struct Lock {
    pub(crate) project_hash: String,
    pub(crate) repo_fingerprint: String,
    pub(crate) checkpoint_size: i64,
    pub(crate) checkpoint_root: String,
    pub(crate) packages: Vec<LockedPackage>,
}

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
}

/// `mfb pkg update`: re-resolve the registry dependencies and write `mfb.lock`,
/// printing a diff against the previous lock.
pub(crate) fn update(project_dir: &Path) -> Result<(), String> {
    let (manifest, _contents) = read_manifest(project_dir)?;
    let previous = read_lock(project_dir)?;
    let lock = resolve(&manifest)?;
    print_lock_diff(previous.as_ref(), &lock);
    write_lock(project_dir, &lock)?;
    println!(
        "Wrote {} resolved package(s) to mfb.lock",
        lock.packages.len()
    );
    // Apply the freshly written lock so the working tree matches it.
    install(project_dir)
}

/// `mfb pkg install`: apply a current `mfb.lock` — fetch each locked blob by
/// hash, verify the full §3.5 chain against the locked ident key, and install.
/// Never resolves and never calls `/index`.
pub(crate) fn install(project_dir: &Path) -> Result<(), String> {
    let (manifest, _contents) = read_manifest(project_dir)?;
    let Some(lock) = read_lock(project_dir)? else {
        return Err("no mfb.lock; run `mfb pkg update` to resolve dependencies first".to_string());
    };
    // A drifted request set means the lock no longer describes the project.
    let current_hash = crate::audit::project_hash(&manifest);
    if lock.project_hash != current_hash {
        return Err(
            "mfb.lock is stale (project.json changed since it was written); run `mfb pkg update`"
                .to_string(),
        );
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
        println!(
            "Installed {} {} ({})",
            package.name, package.selected, package.state
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
) -> Result<Lock, String> {
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
        .filter(|dep| dep.ident.contains('#'))
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
            },
        );
    }

    // Fixpoint: select each node, then recompute the edges it contributes to
    // sibling nodes from its selected version's import table. Bounded so a
    // pathological graph errors instead of spinning.
    let mut blob_cache: BTreeMap<String, Vec<(String, BTreeMap<String, String>, Option<String>)>> =
        BTreeMap::new();
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
            let imports = load_import_edges(&repo_url, &selection.hash, &mut blob_cache)?;
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
    for ident in &node_idents {
        let node = nodes.get(ident).unwrap();
        let selection = select_node(node)?;
        let version = node
            .index
            .versions
            .iter()
            .find(|version| version.version == selection.version)
            .expect("selected version exists");
        packages.push(LockedPackage {
            name: node.name.clone(),
            ident: node.ident.clone(),
            requested: node.requested.clone(),
            selected: selection.version.clone(),
            hash: selection.hash.clone(),
            ident_key: node.index.ident_key.clone(),
            ident_fingerprint: node.index.ident_fingerprint.clone(),
            state: version.state.clone(),
        });
    }
    packages.sort_by(|a, b| a.name.cmp(&b.name));

    let checkpoint = client::fetch_checkpoint(&repo_url, &paths)?;
    let repo_fingerprint = nodes
        .values()
        .next()
        .map(|node| node.index.server_fingerprint.clone())
        .unwrap_or_default();

    Ok(Lock {
        project_hash: crate::audit::project_hash(manifest),
        repo_fingerprint,
        checkpoint_size: checkpoint.size,
        checkpoint_root: checkpoint.root_hash,
        packages,
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
fn is_superset(exports: &BTreeMap<String, String>, required: &BTreeMap<String, String>) -> bool {
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
) -> Result<Vec<(String, BTreeMap<String, String>, Option<String>)>, String> {
    if let Some(edges) = cache.get(hash) {
        return Ok(edges.clone());
    }
    let blob = client::fetch_blob(repo_url, hash)?;
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
fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
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
        (Some(left), Some(right)) => left.cmp(&right),
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

fn lock_path(project_dir: &Path) -> PathBuf {
    project_dir.join("mfb.lock")
}

/// Write `mfb.lock` with a byte-stable formatting so re-resolving an unchanged
/// project reproduces the file exactly.
pub(crate) fn write_lock(project_dir: &Path, lock: &Lock) -> Result<(), String> {
    let path = lock_path(project_dir);
    fs::write(&path, render_lock(lock))
        .map_err(|err| format!("failed to write '{}': {err}", path.display()))
}

fn render_lock(lock: &Lock) -> String {
    let mut out = String::new();
    out.push_str("{\n");
    out.push_str(&format!("  \"lockfileVersion\": {LOCKFILE_VERSION},\n"));
    out.push_str(&format!(
        "  \"projectHash\": {},\n",
        json_string(&lock.project_hash)
    ));
    out.push_str(&format!(
        "  \"repoFingerprint\": {},\n",
        json_string(&lock.repo_fingerprint)
    ));
    out.push_str("  \"checkpoint\": {\n");
    out.push_str(&format!("    \"size\": {},\n", lock.checkpoint_size));
    out.push_str(&format!(
        "    \"rootHash\": {}\n",
        json_string(&lock.checkpoint_root)
    ));
    out.push_str("  },\n");
    out.push_str("  \"packages\": [");
    for (index, package) in lock.packages.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str("\n    {\n");
        out.push_str(&format!(
            "      \"name\": {},\n",
            json_string(&package.name)
        ));
        out.push_str(&format!(
            "      \"ident\": {},\n",
            json_string(&package.ident)
        ));
        out.push_str(&format!(
            "      \"requested\": {},\n",
            json_string(&package.requested)
        ));
        out.push_str(&format!(
            "      \"selected\": {},\n",
            json_string(&package.selected)
        ));
        out.push_str(&format!(
            "      \"hash\": {},\n",
            json_string(&package.hash)
        ));
        out.push_str(&format!(
            "      \"identKey\": {},\n",
            json_string(&package.ident_key)
        ));
        out.push_str(&format!(
            "      \"identFingerprint\": {},\n",
            json_string(&package.ident_fingerprint)
        ));
        out.push_str(&format!(
            "      \"state\": {}\n",
            json_string(&package.state)
        ));
        out.push_str("    }");
    }
    if lock.packages.is_empty() {
        out.push_str("]\n");
    } else {
        out.push_str("\n  ]\n");
    }
    out.push_str("}\n");
    out
}

/// Read an existing `mfb.lock`, if present.
pub(crate) fn read_lock(project_dir: &Path) -> Result<Option<Lock>, String> {
    let path = lock_path(project_dir);
    if !path.is_file() {
        return Ok(None);
    }
    let contents = fs::read_to_string(&path)
        .map_err(|err| format!("failed to read '{}': {err}", path.display()))?;
    let value: JsonValue = contents
        .parse()
        .map_err(|_| format!("'{}' is not valid JSON", path.display()))?;
    let object = value
        .get::<std::collections::HashMap<String, JsonValue>>()
        .ok_or_else(|| "mfb.lock must be a JSON object".to_string())?;
    let string_field = |name: &str| -> String {
        object
            .get(name)
            .and_then(|value| value.get::<String>())
            .cloned()
            .unwrap_or_default()
    };
    let checkpoint = object
        .get("checkpoint")
        .and_then(|value| value.get::<std::collections::HashMap<String, JsonValue>>());
    let checkpoint_size = checkpoint
        .and_then(|object| object.get("size"))
        .and_then(|value| value.get::<f64>())
        .map(|size| *size as i64)
        .unwrap_or(0);
    let checkpoint_root = checkpoint
        .and_then(|object| object.get("rootHash"))
        .and_then(|value| value.get::<String>())
        .cloned()
        .unwrap_or_default();
    let packages = object
        .get("packages")
        .and_then(|value| value.get::<Vec<JsonValue>>())
        .map(|packages| {
            packages
                .iter()
                .filter_map(|entry| {
                    let object = entry.get::<std::collections::HashMap<String, JsonValue>>()?;
                    let get = |name: &str| {
                        object
                            .get(name)
                            .and_then(|value| value.get::<String>())
                            .cloned()
                            .unwrap_or_default()
                    };
                    Some(LockedPackage {
                        name: get("name"),
                        ident: get("ident"),
                        requested: get("requested"),
                        selected: get("selected"),
                        hash: get("hash"),
                        ident_key: get("identKey"),
                        ident_fingerprint: get("identFingerprint"),
                        state: get("state"),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Some(Lock {
        project_hash: string_field("projectHash"),
        repo_fingerprint: string_field("repoFingerprint"),
        checkpoint_size,
        checkpoint_root,
        packages,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering;

    #[test]
    fn version_comparison_orders_releases_and_prereleases() {
        assert_eq!(compare_versions("1.0.1", "1.0.0"), Ordering::Greater);
        assert_eq!(compare_versions("1.2.0", "1.10.0"), Ordering::Less);
        assert_eq!(compare_versions("2.0.0", "2.0.0"), Ordering::Equal);
        // A release outranks the same release with a pre-release suffix.
        assert_eq!(compare_versions("1.0.0", "1.0.0-rc1"), Ordering::Greater);
        assert_eq!(
            compare_versions("1.0.0-rc2", "1.0.0-rc1"),
            Ordering::Greater
        );
        // Missing trailing components are implicit zeroes.
        assert_eq!(compare_versions("1.2", "1.2.0"), Ordering::Equal);
    }

    #[test]
    fn version_comparison_does_not_coerce_malformed_components_to_zero() {
        // `"2x"` used to parse as 0, ranking `2x.0.0` exactly like `0.0.0` — so
        // a malformed registry version could silently outrank `0.9.0`.
        assert_ne!(compare_versions("2x.0.0", "0.0.0"), Ordering::Equal);
        assert_eq!(compare_versions("2.0.0", "2x.0.0"), Ordering::Greater);
        assert_eq!(compare_versions("2x.0.0", "2.0.0"), Ordering::Less);
        // A component too large for u64 is malformed, not zero.
        assert_eq!(
            compare_versions("18446744073709551616.0.0", "1.0.0"),
            Ordering::Less
        );
        // Two malformed components order lexically, so the comparison is total.
        assert_eq!(compare_versions("2x.0.0", "2y.0.0"), Ordering::Less);
        assert_eq!(compare_versions("2x.0.0", "2x.0.0"), Ordering::Equal);
    }

    #[test]
    fn superset_requires_every_symbol_hash() {
        let mut exports = BTreeMap::new();
        exports.insert("foo".to_string(), "aa".to_string());
        exports.insert("bar".to_string(), "bb".to_string());
        let mut needs_foo = BTreeMap::new();
        needs_foo.insert("foo".to_string(), "aa".to_string());
        assert!(is_superset(&exports, &needs_foo));
        // A missing symbol is not a superset.
        let mut needs_baz = BTreeMap::new();
        needs_baz.insert("baz".to_string(), "cc".to_string());
        assert!(!is_superset(&exports, &needs_baz));
        // A differing hash is not a superset.
        let mut needs_foo_v2 = BTreeMap::new();
        needs_foo_v2.insert("foo".to_string(), "ff".to_string());
        assert!(!is_superset(&exports, &needs_foo_v2));
    }

    fn index_version(version: &str, state: &str) -> mfb_repository::server::IndexVersion {
        mfb_repository::server::IndexVersion {
            version: version.to_string(),
            hash: format!("hash-{version}"),
            published_at: 0,
            state: state.to_string(),
            abi_index: serde_json::Value::Null,
            log_entry: None,
        }
    }

    fn node_with(
        versions: Vec<mfb_repository::server::IndexVersion>,
        requirers: Vec<Requirer>,
    ) -> Node {
        let mut map = BTreeMap::new();
        for (index, requirer) in requirers.into_iter().enumerate() {
            map.insert(format!("req-{index}"), requirer);
        }
        Node {
            name: "shape".to_string(),
            ident: "ada#shape".to_string(),
            index: mfb_repository::server::IndexResponse {
                ident: "ada#shape".to_string(),
                owner: "ada".to_string(),
                ident_key: "ed25519:ik".to_string(),
                ident_fingerprint: "if".to_string(),
                name_binding_signature: String::new(),
                server_fingerprint: "sf".to_string(),
                versions,
            },
            requested: "1.0.0".to_string(),
            requirers: map,
            selected: None,
        }
    }

    fn select_node_err(node: &Node) -> String {
        match select_node(node) {
            Ok(_) => panic!("expected select_node to fail"),
            Err(message) => message,
        }
    }

    fn requirer(who: &str, required: &[(&str, &str)], pin: Option<&str>) -> Requirer {
        Requirer {
            who: who.to_string(),
            required: required
                .iter()
                .map(|(name, hash)| (name.to_string(), hash.to_string()))
                .collect(),
            pin: pin.map(str::to_string),
        }
    }

    #[test]
    fn read_manifest_reads_and_validates() {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(
            dir.path().join("project.json"),
            "{\"name\":\"app\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"sources\":[{\"root\":\"src\"}]}",
        )
        .expect("manifest");
        let (manifest, _contents) = read_manifest(dir.path()).expect("read manifest");
        assert_eq!(
            manifest
                .get("name")
                .and_then(|v| v.get::<String>())
                .map(String::as_str),
            Some("app")
        );
        // A missing manifest is a read error.
        let empty = tempfile::tempdir().expect("temp dir");
        assert!(read_manifest(empty.path())
            .unwrap_err()
            .contains("failed to read"));
    }

    #[test]
    fn install_without_a_lock_errors_before_network() {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(
            dir.path().join("project.json"),
            "{\"name\":\"app\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"sources\":[{\"root\":\"src\"}]}",
        )
        .expect("manifest");
        // No mfb.lock present -> early error, no /blob fetch.
        assert!(install(dir.path()).unwrap_err().contains("no mfb.lock"));
    }

    #[test]
    fn install_with_stale_lock_errors_before_network() {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(
            dir.path().join("project.json"),
            "{\"name\":\"app\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"sources\":[{\"root\":\"src\"}]}",
        )
        .expect("manifest");
        // A lock whose projectHash does not match the current project is stale.
        std::fs::write(
            dir.path().join("mfb.lock"),
            "{\"lockfileVersion\":1,\"projectHash\":\"stale\",\"repoFingerprint\":\"r\",\"checkpoint\":{\"size\":0,\"rootHash\":\"\"},\"packages\":[]}\n",
        )
        .expect("lock");
        assert!(install(dir.path()).unwrap_err().contains("stale"));
    }

    #[test]
    fn read_lock_absent_returns_none() {
        let dir = tempfile::tempdir().expect("temp dir");
        assert!(read_lock(dir.path()).expect("read").is_none());
    }

    #[test]
    fn read_lock_rejects_non_json() {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(dir.path().join("mfb.lock"), "not json").expect("lock");
        assert!(read_lock(dir.path()).is_err());
    }

    #[test]
    fn write_lock_writes_a_readable_lockfile() {
        let dir = tempfile::tempdir().expect("temp dir");
        let lock = Lock {
            project_hash: "p".to_string(),
            repo_fingerprint: "r".to_string(),
            checkpoint_size: 3,
            checkpoint_root: "root".to_string(),
            packages: Vec::new(),
        };
        write_lock(dir.path(), &lock).expect("write");
        assert!(dir.path().join("mfb.lock").is_file());
        // The written lock round-trips.
        let reread = read_lock(dir.path()).expect("read").expect("present");
        assert_eq!(reread.project_hash, "p");
        assert_eq!(reread.checkpoint_size, 3);
    }

    #[test]
    fn select_node_picks_highest_superset_version() {
        let node = node_with(
            vec![
                index_version("1.0.0", "available"),
                index_version("2.0.0", "available"),
                index_version("1.5.0", "deprecated"),
            ],
            vec![requirer("project", &[], None)],
        );
        let selection = select_node(&node).expect("selection");
        // Highest floating-eligible version with no ABI needs is 2.0.0.
        assert_eq!(selection.version, "2.0.0");
    }

    #[test]
    fn select_node_honors_a_pin() {
        let node = node_with(
            vec![
                index_version("1.0.0", "available"),
                index_version("2.0.0", "available"),
            ],
            vec![requirer("project", &[], Some("1.0.0"))],
        );
        let selection = select_node(&node).expect("selection");
        assert_eq!(selection.version, "1.0.0");
    }

    #[test]
    fn select_node_rejects_conflicting_pins() {
        let node = node_with(
            vec![
                index_version("1.0.0", "available"),
                index_version("2.0.0", "available"),
            ],
            vec![
                requirer("a", &[], Some("1.0.0")),
                requirer("b", &[], Some("2.0.0")),
            ],
        );
        assert!(select_node_err(&node).contains("conflicting pins"));
    }

    #[test]
    fn select_node_rejects_unpublished_pin() {
        let node = node_with(
            vec![index_version("1.0.0", "available")],
            vec![requirer("a", &[], Some("9.9.9"))],
        );
        assert!(select_node_err(&node).contains("is not published"));
    }

    #[test]
    fn select_node_rejects_blocked_pin() {
        let node = node_with(
            vec![index_version("1.0.0", "blocked")],
            vec![requirer("a", &[], Some("1.0.0"))],
        );
        assert!(select_node_err(&node).contains("cannot be selected"));
    }

    #[test]
    fn select_node_reports_diamond_conflict() {
        let node = node_with(
            vec![index_version("1.0.0", "available")],
            vec![
                requirer("a", &[("foo", "aa")], None),
                requirer("b", &[("foo", "bb")], None),
            ],
        );
        assert!(select_node_err(&node).contains("diamond conflict"));
    }

    #[test]
    fn select_node_reports_no_satisfying_version() {
        // A required symbol no version provides -> no eligible candidate.
        let node = node_with(
            vec![index_version("1.0.0", "available")],
            vec![requirer("a", &[("missing", "zz")], None)],
        );
        assert!(select_node_err(&node).contains("no install-eligible version"));
    }

    #[test]
    fn print_lock_diff_covers_add_change_keep_and_remove() {
        let package = |name: &str, ident: &str, selected: &str, state: &str| LockedPackage {
            name: name.to_string(),
            ident: ident.to_string(),
            requested: "1.0.0".to_string(),
            selected: selected.to_string(),
            hash: "h".to_string(),
            ident_key: String::new(),
            ident_fingerprint: String::new(),
            state: state.to_string(),
        };
        let previous = Lock {
            project_hash: "p".to_string(),
            repo_fingerprint: "r".to_string(),
            checkpoint_size: 0,
            checkpoint_root: String::new(),
            packages: vec![
                package("kept", "a#kept", "1.0.0", "available"),
                package("bumped", "a#bumped", "1.0.0", "available"),
                package("gone", "a#gone", "1.0.0", "available"),
            ],
        };
        let next = Lock {
            project_hash: "p".to_string(),
            repo_fingerprint: "r".to_string(),
            checkpoint_size: 0,
            checkpoint_root: String::new(),
            packages: vec![
                package("kept", "a#kept", "1.0.0", "available"),
                package("bumped", "a#bumped", "2.0.0", "available"),
                package("added", "a#added", "1.0.0", "available"),
            ],
        };
        // Exercises +/~/keep/- lines; must not panic. Also covers the None-previous path.
        print_lock_diff(Some(&previous), &next);
        print_lock_diff(None, &next);
    }

    #[test]
    fn lockfile_round_trips_byte_identically() {
        let lock = Lock {
            project_hash: "abc123".to_string(),
            repo_fingerprint: "def456".to_string(),
            checkpoint_size: 7,
            checkpoint_root: "0f0f".to_string(),
            packages: vec![
                LockedPackage {
                    name: "beta".to_string(),
                    ident: "alice#beta".to_string(),
                    requested: "1.0.0".to_string(),
                    selected: "1.0.1".to_string(),
                    hash: "aa".to_string(),
                    ident_key: "ed25519:xyz".to_string(),
                    ident_fingerprint: "ff".to_string(),
                    state: "available".to_string(),
                },
                LockedPackage {
                    name: "alpha".to_string(),
                    ident: "alice#alpha".to_string(),
                    requested: "2.0.0".to_string(),
                    selected: "2.0.0".to_string(),
                    hash: "bb".to_string(),
                    ident_key: "ed25519:uvw".to_string(),
                    ident_fingerprint: "ee".to_string(),
                    state: "deprecated".to_string(),
                },
            ],
        };
        let rendered = render_lock(&lock);
        // A rebuilt lock renders identically (deterministic resolution).
        let temp = std::env::temp_dir().join(format!("mfb-lock-test-{}", lock.project_hash));
        std::fs::create_dir_all(&temp).unwrap();
        std::fs::write(temp.join("mfb.lock"), &rendered).unwrap();
        let reread = read_lock(&temp).unwrap().unwrap();
        assert_eq!(render_lock(&reread), rendered);
        assert_eq!(reread.packages.len(), 2);
        assert_eq!(reread.checkpoint_size, 7);
        std::fs::remove_dir_all(&temp).ok();
    }
}

fn print_lock_diff(previous: Option<&Lock>, next: &Lock) {
    let old: BTreeMap<&str, &LockedPackage> = previous
        .map(|lock| {
            lock.packages
                .iter()
                .map(|p| (p.ident.as_str(), p))
                .collect()
        })
        .unwrap_or_default();
    println!("Resolution:");
    for package in &next.packages {
        match old.get(package.ident.as_str()) {
            None => println!(
                "  + {} {} ({})",
                package.name, package.selected, package.state
            ),
            Some(before) if before.selected != package.selected => println!(
                "  ~ {} {} -> {} ({})",
                package.name, before.selected, package.selected, package.state
            ),
            Some(_) => println!(
                "    {} {} ({})",
                package.name, package.selected, package.state
            ),
        }
    }
    if let Some(previous) = previous {
        for package in &previous.packages {
            if !next.packages.iter().any(|p| p.ident == package.ident) {
                println!("  - {} {}", package.name, package.selected);
            }
        }
    }
}
