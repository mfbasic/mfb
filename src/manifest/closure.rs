//! A project's declared dependency closure (bug-628).
//!
//! A native build merges exactly the packages `project.json` declares
//! (`installed_package_files` → `merge_packages`), so a package's own
//! dependencies must be declared by every project that uses it: an undeclared
//! one is never merged, and the importer's reference to it reached lowering as
//! the unlocated `NIR call target … does not resolve`.
//!
//! The rule: `packages[]` lists the whole closure, and every entry records why it
//! is there — `direct` (the user added it) and `requiredBy` (the idents of the
//! declared packages whose import tables name it). The truth those fields mirror
//! is each package's import table, so this module reads the tables and
//!
//! - [`check`] reports every way the manifest disagrees with them (the build
//!   refuses on any, and never repairs);
//! - [`conflicts`] reports a declared package that does not provide what an
//!   importer was compiled against (the build refuses; `mfb pkg verify` explains);
//! - [`reconcile`] computes the manifest `mfb pkg` writes: missing dependencies
//!   declared, orphaned indirect ones dropped, both fields rewritten.
//!
//! What a declared package needs is read the way the build will read it: an
//! installed `packages/<name>.mfp`'s import table wins; otherwise a source
//! directory's own `project.json` (whose `packages[]` IS the import table the
//! build compiles into it — `package_dependencies`); otherwise the build cache.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

use tinyjson::JsonValue;

use crate::binary_repr;
use crate::manifest::json_edit::{
    project_json_with_closure_fields, project_json_with_package, project_json_without_packages,
};
use crate::manifest::package::{
    project_package_dependency, resolved_package_file, source_dependency, ProjectPackageDependency,
    SourceDependency,
};
use crate::manifest::parse_project_json;

/// Upper bound on [`reconcile`]'s structural passes. Each pass adds or removes one
/// entry, so a real closure converges in (entries added + entries removed) passes.
const MAX_RECONCILE_PASSES: usize = 512;

/// One dependency a declared package needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Requirement {
    pub(crate) name: String,
    pub(crate) ident: String,
    pub(crate) version: String,
    pub(crate) pin: bool,
    /// The requirer's own `source` for it, rebased onto the importing project;
    /// `None` when the requirer is a compiled `.mfp` that records no source.
    pub(crate) source: Option<String>,
    pub(crate) ident_key: String,
    /// A compiled `.mfp` of it that must be copied into the importing project's
    /// `packages/` for the declared `source` to build.
    pub(crate) install_from: Option<PathBuf>,
}

/// A way `packages[]` disagrees with the packages' import tables.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Issue {
    Undeclared {
        requirer: String,
        name: String,
    },
    NameClash {
        requirer: String,
        name: String,
        ident: String,
        declared_ident: String,
    },
    MissingDirect {
        name: String,
    },
    MissingRequiredBy {
        name: String,
        expected: Vec<String>,
    },
    RequiredBy {
        name: String,
        declared: Vec<String>,
        expected: Vec<String>,
    },
    Orphan {
        name: String,
    },
}

impl Issue {
    pub(crate) fn message(&self) -> String {
        match self {
            Issue::Undeclared { requirer, name } => format!(
                "package `{requirer}` requires `{name}`, which project.json does not declare"
            ),
            Issue::NameClash {
                requirer,
                name,
                ident,
                declared_ident,
            } => format!(
                "package `{requirer}` requires `{name}` as ident `{ident}`, but project.json \
                 declares `{name}` as ident `{declared_ident}`"
            ),
            Issue::MissingDirect { name } => {
                format!("package `{name}` has no boolean `direct` field")
            }
            Issue::MissingRequiredBy { name, expected } => format!(
                "package `{name}` has no `requiredBy` array of idents; it is required by {}",
                ident_list(expected)
            ),
            Issue::RequiredBy {
                name,
                declared,
                expected,
            } => format!(
                "package `{name}` lists requiredBy [{}], but it is required by {}",
                declared
                    .iter()
                    .map(|ident| format!("`{ident}`"))
                    .collect::<Vec<_>>()
                    .join(", "),
                ident_list(expected)
            ),
            Issue::Orphan { name } => {
                format!("package `{name}` is not `direct` and no declared package requires it")
            }
        }
    }
}

fn ident_list(idents: &[String]) -> String {
    if idents.is_empty() {
        return "no declared package".to_string();
    }
    let items: Vec<String> = idents.iter().map(|ident| format!("`{ident}`")).collect();
    format!("[{}]", items.join(", "))
}

/// A declared package that does not provide what an importer was compiled against.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Conflict {
    pub(crate) requirer: String,
    pub(crate) name: String,
    pub(crate) installed_version: String,
    pub(crate) kind: ConflictKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ConflictKind {
    /// The importer pins an exact version the installed package is not.
    Pin { pinned: String },
    /// Symbols the importer uses whose ABI hash the installed package does not
    /// export: `(symbol, the importer's hash, the installed hash if exported)`.
    Symbols(Vec<(String, String, Option<String>)>),
}

impl Conflict {
    /// The one-line summary the build prints.
    pub(crate) fn message(&self) -> String {
        match &self.kind {
            ConflictKind::Pin { pinned } => format!(
                "package `{}` pins `{}` {pinned}, but the installed `{}` is {}",
                self.requirer, self.name, self.name, self.installed_version
            ),
            ConflictKind::Symbols(symbols) => {
                let names: Vec<String> = symbols
                    .iter()
                    .map(|(name, _, _)| format!("`{name}`"))
                    .collect();
                format!(
                    "package `{}` was built against a `{}` incompatible with the installed one: \
                     its {} differs from `{}` {}",
                    self.requirer,
                    self.name,
                    names.join(", "),
                    self.name,
                    self.installed_version
                )
            }
        }
    }

    /// The detail lines `mfb pkg verify` prints under the summary.
    pub(crate) fn details(&self) -> Vec<String> {
        match &self.kind {
            ConflictKind::Pin { pinned } => vec![format!(
                "`{}` requires exactly {pinned}; installed: {}",
                self.requirer, self.installed_version
            )],
            ConflictKind::Symbols(symbols) => symbols
                .iter()
                .map(|(symbol, wanted, installed)| match installed {
                    Some(installed) => format!(
                        "`{symbol}`: `{}` needs ABI {wanted}; `{}` {} exports {installed}",
                        self.requirer, self.name, self.installed_version
                    ),
                    None => format!(
                        "`{symbol}`: `{}` needs ABI {wanted}; `{}` {} does not export it",
                        self.requirer, self.name, self.installed_version
                    ),
                })
                .collect(),
        }
    }
}

/// The declared entries of `manifest`, in file order.
pub(crate) fn declared_dependencies(
    manifest: &std::collections::HashMap<String, JsonValue>,
) -> Vec<ProjectPackageDependency> {
    manifest
        .get("packages")
        .and_then(|value| value.get::<Vec<JsonValue>>())
        .into_iter()
        .flatten()
        .filter_map(project_package_dependency)
        .collect()
}

/// Every declared entry paired with what it needs (`None`: not readable yet —
/// not installed, and no source directory to read).
struct Graph {
    entries: Vec<(ProjectPackageDependency, Option<Vec<Requirement>>)>,
}

impl Graph {
    fn read(project_dir: &Path, declared: &[ProjectPackageDependency]) -> Result<Graph, String> {
        let mut entries = Vec::new();
        for dependency in declared {
            let needs = requirements(project_dir, dependency)?.map(|needs| {
                needs
                    .into_iter()
                    .filter(|need| need.ident != dependency.ident)
                    .collect()
            });
            entries.push((dependency.clone(), needs));
        }
        Ok(Graph { entries })
    }

    /// The sorted idents of the declared packages that need `ident`.
    fn required_by(&self, ident: &str) -> Vec<String> {
        let set: BTreeSet<String> = self
            .entries
            .iter()
            .filter(|(_, needs)| needs.iter().flatten().any(|need| need.ident == ident))
            .map(|(dependency, _)| dependency.ident.clone())
            .collect();
        set.into_iter().collect()
    }

    /// Each requirement no declared entry satisfies, once per ident, with the
    /// name of the first declared package that needs it.
    fn undeclared(&self) -> Vec<(String, Requirement)> {
        let declared: BTreeSet<&str> = self
            .entries
            .iter()
            .map(|(dependency, _)| dependency.ident.as_str())
            .collect();
        let mut seen = BTreeSet::new();
        let mut out = Vec::new();
        for (dependency, needs) in &self.entries {
            for need in needs.iter().flatten() {
                if !declared.contains(need.ident.as_str()) && seen.insert(need.ident.clone()) {
                    out.push((dependency.name.clone(), need.clone()));
                }
            }
        }
        out
    }

    fn declared_with_name(&self, name: &str) -> Option<&ProjectPackageDependency> {
        self.entries
            .iter()
            .map(|(dependency, _)| dependency)
            .find(|dependency| dependency.name == name)
    }
}

/// What one declared dependency needs, read from where the build will read it.
fn requirements(
    project_dir: &Path,
    dependency: &ProjectPackageDependency,
) -> Result<Option<Vec<Requirement>>, String> {
    let installed = project_dir
        .join("packages")
        .join(format!("{}.mfp", dependency.name));
    if installed.is_file() {
        return Ok(import_table_requirements(
            project_dir,
            &installed,
            Some(&dependency.source),
        ));
    }
    if let SourceDependency::Directory(dir) =
        source_dependency(project_dir, &dependency.name, Some(&dependency.source))
    {
        if dir.join("project.json").is_file() {
            return source_manifest_requirements(project_dir, &dir).map(Some);
        }
    }
    Ok(resolved_package_file(project_dir, &dependency.name)
        .and_then(|path| import_table_requirements(project_dir, &path, None)))
}

/// A compiled package's import table. The table records no source, so a
/// dependency is located beside the requirer: a sibling `<name>.mfp` next to the
/// `.mfp` the requirer was installed from (`source`), when there is one.
///
/// `None` when the payload cannot be decoded: what it needs is unknown, and the
/// package's verification and merge report the broken file far more precisely.
fn import_table_requirements(
    project_dir: &Path,
    path: &Path,
    source: Option<&str>,
) -> Option<Vec<Requirement>> {
    let info = binary_repr::read_package_info(path).ok()?;
    let origin_dir = source.and_then(|source| installed_from_dir(project_dir, source));
    Some(
        info.imports
            .into_iter()
            .map(|import| {
                let ident = if import.package_ident.is_empty() {
                    import.package_name.clone()
                } else {
                    import.package_ident.clone()
                };
                let sibling = origin_dir
                    .as_ref()
                    .map(|(dir, _)| dir.join(format!("{}.mfp", import.package_name)))
                    .filter(|sibling| sibling.is_file());
                // Installed as `packages/<name>.mfp`, recorded the way its requirer
                // was: project-relative, or as the `file://` URL it was added from.
                let source = match (&origin_dir, &sibling) {
                    (Some((_, true)), Some(_)) => {
                        Some(format!("file:packages/{}.mfp", import.package_name))
                    }
                    (Some((dir, false)), Some(_)) => Some(format!(
                        "file://{}",
                        slash_path(&dir.join(format!("{}.mfp", import.package_name)))
                    )),
                    _ => None,
                };
                Requirement {
                    name: import.package_name,
                    ident,
                    version: import.version,
                    pin: import.pin,
                    source,
                    ident_key: String::new(),
                    install_from: sibling,
                }
            })
            .collect(),
    )
}

/// The directory an installed `.mfp` was added from, and whether its `source`
/// spelled it project-relative (`file:<rel>.mfp`) rather than as a `file://` URL.
fn installed_from_dir(project_dir: &Path, source: &str) -> Option<(PathBuf, bool)> {
    if source.starts_with("file://") {
        let path = crate::manifest::package::package_file_url_path(source).ok()?;
        return path.parent().map(|dir| (dir.to_path_buf(), false));
    }
    let relative = source.strip_prefix("file:")?;
    if !relative.ends_with(".mfp") {
        return None;
    }
    project_dir
        .join(relative)
        .parent()
        .map(|dir| (dir.to_path_buf(), true))
}

/// A source package's own `packages[]`, each `source` rebased from `dir` onto
/// `project_dir`.
fn source_manifest_requirements(
    project_dir: &Path,
    dir: &Path,
) -> Result<Vec<Requirement>, String> {
    let path = dir.join("project.json");
    let contents = std::fs::read_to_string(&path)
        .map_err(|err| format!("failed to read '{}': {err}", path.display()))?;
    let manifest = parse_project_json(&contents, &path)?;
    Ok(declared_dependencies(&manifest)
        .into_iter()
        .map(|dependency| {
            let (source, install_from) = rebase_source(project_dir, dir, &dependency);
            Requirement {
                name: dependency.name,
                ident: dependency.ident,
                version: dependency.version,
                pin: dependency.pin,
                source: Some(source),
                ident_key: dependency.ident_key,
                install_from,
            }
        })
        .collect())
}

/// Re-express `dependency`'s `source`, written relative to the project at
/// `requirer_dir`, for the project at `project_dir`, and name the compiled file
/// to install into `project_dir/packages/` when the requirer builds against one.
///
/// The requirer builds against what the build would resolve for it: its
/// installed `packages/<name>.mfp` wins over any source directory beside it
/// (`source_dependency_dirs`), so that file is what the importer installs too,
/// recorded as `file:packages/<name>.mfp` — or, for a `file://` add, the URL the
/// user added it from. A registry ident is the same wherever it is written, and
/// is installed from the registry, never copied.
fn rebase_source(
    project_dir: &Path,
    requirer_dir: &Path,
    dependency: &ProjectPackageDependency,
) -> (String, Option<PathBuf>) {
    let source = dependency.source.as_str();
    let name = &dependency.name;
    let is_registry = !source.is_empty() && !source.contains(':');
    if is_registry {
        return (source.to_string(), None);
    }
    let installed = requirer_dir.join("packages").join(format!("{name}.mfp"));
    if source.starts_with("file://") {
        let origin = crate::manifest::package::package_file_url_path(source).ok();
        let install_from = if installed.is_file() {
            Some(installed)
        } else {
            origin.filter(|origin| origin.is_file())
        };
        return (source.to_string(), install_from);
    }
    if installed.is_file() {
        return (format!("file:packages/{name}.mfp"), Some(installed));
    }
    if source.starts_with("local://") {
        return (source.to_string(), None);
    }
    match source.strip_prefix("file:") {
        Some(relative) if relative.ends_with(".mfp") => {
            let compiled = requirer_dir.join(relative);
            (
                format!("file:packages/{name}.mfp"),
                compiled.is_file().then_some(compiled),
            )
        }
        Some(relative) => (file_source(project_dir, &requirer_dir.join(relative)), None),
        None => (
            file_source(project_dir, &requirer_dir.join("packages").join(name)),
            None,
        ),
    }
}

/// `file:<path relative to project_dir>`, or an absolute spelling when no
/// relative path exists (different roots).
fn file_source(project_dir: &Path, target: &Path) -> String {
    match relative_path(project_dir, target) {
        Some(relative) => format!("file:{relative}"),
        None if target.extension().is_some_and(|ext| ext == "mfp") => {
            format!("file://{}", slash_path(&normalize(target)))
        }
        None => format!("local://{}", slash_path(&normalize(target))),
    }
}

fn slash_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Absolute and lexically normalized, with its longest existing prefix
/// canonicalized — so a path that does not exist yet still compares equal
/// component-for-component with one that does (`/tmp` vs `/private/tmp`).
fn normalize(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    };
    let mut lexical = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::ParentDir => {
                lexical.pop();
            }
            Component::CurDir => {}
            other => lexical.push(other),
        }
    }
    let mut existing = lexical.as_path();
    let mut rest = Vec::new();
    loop {
        if let Ok(canonical) = std::fs::canonicalize(existing) {
            return rest
                .iter()
                .rev()
                .fold(canonical, |path, part| path.join(part));
        }
        match (existing.parent(), existing.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_os_string());
                existing = parent;
            }
            _ => return lexical,
        }
    }
}

/// `target` relative to the directory `from`, with `/` separators.
fn relative_path(from: &Path, target: &Path) -> Option<String> {
    let from = normalize(from);
    let target = normalize(target);
    let from_parts: Vec<Component> = from.components().collect();
    let target_parts: Vec<Component> = target.components().collect();
    if from_parts.first() != target_parts.first() {
        return None;
    }
    let common = from_parts
        .iter()
        .zip(&target_parts)
        .take_while(|(a, b)| a == b)
        .count();
    let mut parts: Vec<String> = vec!["..".to_string(); from_parts.len() - common];
    parts.extend(
        target_parts[common..]
            .iter()
            .map(|part| part.as_os_str().to_string_lossy().into_owned()),
    );
    if parts.is_empty() {
        return Some(".".to_string());
    }
    Some(parts.join("/"))
}

/// Every way `manifest`'s `packages[]` disagrees with the import tables.
pub(crate) fn check(
    project_dir: &Path,
    manifest: &std::collections::HashMap<String, JsonValue>,
) -> Result<Vec<Issue>, String> {
    let graph = Graph::read(project_dir, &declared_dependencies(manifest))?;
    let mut issues = Vec::new();
    for (requirer, need) in graph.undeclared() {
        issues.push(match graph.declared_with_name(&need.name) {
            Some(clash) => Issue::NameClash {
                requirer,
                name: need.name,
                ident: need.ident,
                declared_ident: clash.ident.clone(),
            },
            None => Issue::Undeclared {
                requirer,
                name: need.name,
            },
        });
    }
    for (dependency, _) in &graph.entries {
        let expected = graph.required_by(&dependency.ident);
        let name = dependency.name.clone();
        if dependency.direct.is_none() {
            issues.push(Issue::MissingDirect { name: name.clone() });
        }
        match &dependency.required_by {
            None => issues.push(Issue::MissingRequiredBy {
                name: name.clone(),
                expected: expected.clone(),
            }),
            Some(declared) => {
                let declared_set: BTreeSet<&String> = declared.iter().collect();
                let expected_set: BTreeSet<&String> = expected.iter().collect();
                if declared_set != expected_set || declared.len() != declared_set.len() {
                    issues.push(Issue::RequiredBy {
                        name: name.clone(),
                        declared: declared.clone(),
                        expected: expected.clone(),
                    });
                }
            }
        }
        if dependency.direct == Some(false) && expected.is_empty() {
            issues.push(Issue::Orphan { name });
        }
    }
    Ok(issues)
}

/// Every declared package that does not provide what a declared importer was
/// compiled against. Reads compiled `.mfp`s only; a package not yet compiled or
/// installed is skipped (its absence is reported elsewhere).
pub(crate) fn conflicts(
    project_dir: &Path,
    manifest: &std::collections::HashMap<String, JsonValue>,
) -> Vec<Conflict> {
    let declared = declared_dependencies(manifest);
    let mut infos = BTreeMap::new();
    for dependency in &declared {
        if let Some(path) = resolved_package_file(project_dir, &dependency.name) {
            if let Ok(info) = binary_repr::read_package_info(&path) {
                infos.insert(dependency.ident.clone(), (dependency.name.clone(), info));
            }
        }
    }
    let mut conflicts = Vec::new();
    for (requirer_ident, (requirer, info)) in &infos {
        for import in &info.imports {
            let ident = if import.package_ident.is_empty() {
                &import.package_name
            } else {
                &import.package_ident
            };
            if ident == requirer_ident {
                continue;
            }
            let Some((name, target)) = infos.get(ident) else {
                continue;
            };
            let installed_version = target.manifest_version.clone();
            let pinned = import.version.trim_start_matches('=');
            if import.pin && pinned != installed_version {
                conflicts.push(Conflict {
                    requirer: requirer.clone(),
                    name: name.clone(),
                    installed_version: installed_version.clone(),
                    kind: ConflictKind::Pin {
                        pinned: pinned.to_string(),
                    },
                });
            }
            let exports: BTreeSet<(&str, &str)> = target
                .exports
                .iter()
                .map(|export| (export.name.as_str(), export.sig_hash.as_str()))
                .collect();
            let missing: Vec<(String, String, Option<String>)> = import
                .used_symbols
                .iter()
                .filter(|used| !exports.contains(&(used.name.as_str(), used.sig_hash.as_str())))
                .map(|used| {
                    let installed = target
                        .exports
                        .iter()
                        .find(|export| export.name == used.name)
                        .map(|export| export.sig_hash.clone());
                    (used.name.clone(), used.sig_hash.clone(), installed)
                })
                .collect();
            if !missing.is_empty() {
                conflicts.push(Conflict {
                    requirer: requirer.clone(),
                    name: name.clone(),
                    installed_version,
                    kind: ConflictKind::Symbols(missing),
                });
            }
        }
    }
    conflicts
}

/// The manifest `mfb pkg` writes, and what it changed.
#[derive(Debug, Default)]
pub(crate) struct Reconciled {
    pub(crate) contents: String,
    /// `(name, the declared package that needs it)` for each entry added.
    pub(crate) added: Vec<(String, String)>,
    /// Names of the orphaned indirect entries dropped.
    pub(crate) removed: Vec<String>,
    /// `(name, file)`: a compiled package to copy into `packages/<name>.mfp`.
    pub(crate) installs: Vec<(String, PathBuf)>,
    /// Needed packages this module cannot locate: `(requirer name, requirement)`.
    /// A registry ident is the caller's to fetch; anything else is an error.
    pub(crate) unresolved: Vec<(String, Requirement)>,
}

/// Close `contents`' dependency set: declare every locatable undeclared
/// dependency (`direct: false`), drop every indirect entry nothing requires,
/// then set each entry's `requiredBy` from the import tables and default a
/// missing `direct` to `true` — an entry already in the file is one the user
/// wrote. Edits are surgical; an already-correct manifest is returned unchanged.
pub(crate) fn reconcile(project_dir: &Path, contents: &str) -> Result<Reconciled, String> {
    let project_path = project_dir.join("project.json");
    let mut result = Reconciled {
        contents: contents.to_string(),
        ..Reconciled::default()
    };
    for _ in 0..MAX_RECONCILE_PASSES {
        let manifest = parse_project_json(&result.contents, &project_path)?;
        let graph = Graph::read(project_dir, &declared_dependencies(&manifest))?;

        // 1. Declare one locatable dependency per pass, then re-read.
        let mut unresolved = Vec::new();
        let mut declared_one = false;
        for (requirer, need) in graph.undeclared() {
            if let Some(clash) = graph.declared_with_name(&need.name) {
                return Err(Issue::NameClash {
                    requirer,
                    name: need.name.clone(),
                    ident: need.ident.clone(),
                    declared_ident: clash.ident.clone(),
                }
                .message());
            }
            let Some(source) = need.source.clone() else {
                unresolved.push((requirer, need));
                continue;
            };
            let dependency = ProjectPackageDependency {
                name: need.name.clone(),
                ident: need.ident.clone(),
                version: need.version.clone(),
                pin: need.pin,
                source,
                ident_key: need.ident_key.clone(),
                direct: Some(false),
                required_by: Some(Vec::new()),
            };
            result.contents = project_json_with_package(&result.contents, &manifest, &dependency)?;
            if let Some(file) = &need.install_from {
                result.installs.push((need.name.clone(), file.clone()));
            }
            result.added.push((need.name, requirer));
            declared_one = true;
            break;
        }
        if declared_one {
            continue;
        }

        // 2. Drop one orphaned indirect entry per pass, then re-read.
        if let Some((orphan, _)) = graph.entries.iter().find(|(dependency, _)| {
            dependency.direct == Some(false) && graph.required_by(&dependency.ident).is_empty()
        }) {
            result.contents =
                project_json_without_packages(&result.contents, &[orphan.ident.as_str()])?;
            result.removed.push(orphan.name.clone());
            continue;
        }

        // 3. The set is closed: write both fields on every entry.
        for (dependency, _) in &graph.entries {
            let expected = graph.required_by(&dependency.ident);
            let direct = dependency.direct.unwrap_or(true);
            if dependency.direct != Some(direct)
                || dependency.required_by.as_deref() != Some(expected.as_slice())
            {
                result.contents = project_json_with_closure_fields(
                    &result.contents,
                    &dependency.ident,
                    direct,
                    &expected,
                )?;
            }
        }
        result.unresolved = unresolved;
        return Ok(result);
    }
    Err(format!(
        "the dependency closure of '{}' did not converge after {MAX_RECONCILE_PASSES} passes",
        project_path.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    fn manifest(packages: &str) -> String {
        format!(
            "{{\n  \"name\": \"p\",\n  \"version\": \"0.1.0\",\n  \"mfb\": \"1.0\",\n  \
             \"kind\": \"package\",\n  \"packages\": [{packages}]\n}}\n"
        )
    }

    /// root/base (no deps), root/user (declares base), root/app (declares user).
    fn fixture(app_packages: &str) -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        write(&root.path().join("base/project.json"), &manifest(""));
        write(
            &root.path().join("user/project.json"),
            &manifest(
                "{\"name\": \"base\", \"version\": \"=0.1.0\", \"source\": \"file:../base\", \
                 \"direct\": true, \"requiredBy\": []}",
            ),
        );
        write(
            &root.path().join("app/project.json"),
            &manifest(app_packages),
        );
        root
    }

    fn check_app(root: &Path) -> Vec<Issue> {
        let app = root.join("app");
        let contents = fs::read_to_string(app.join("project.json")).unwrap();
        let manifest = parse_project_json(&contents, &app.join("project.json")).unwrap();
        check(&app, &manifest).unwrap()
    }

    const USER_ONLY: &str = "{\"name\": \"user\", \"version\": \"=0.1.0\", \
                             \"source\": \"file:../user\", \"direct\": true, \"requiredBy\": []}";

    #[test]
    fn an_undeclared_dependency_of_a_declared_package_is_an_issue() {
        let root = fixture(USER_ONLY);
        assert_eq!(
            check_app(root.path()),
            vec![Issue::Undeclared {
                requirer: "user".to_string(),
                name: "base".to_string()
            }]
        );
    }

    #[test]
    fn a_closed_manifest_has_no_issues() {
        let root = fixture(&format!(
            "{USER_ONLY}, {{\"name\": \"base\", \"version\": \"=0.1.0\", \
             \"source\": \"file:../base\", \"direct\": false, \"requiredBy\": [\"user\"]}}"
        ));
        assert_eq!(check_app(root.path()), Vec::<Issue>::new());
    }

    #[test]
    fn missing_stale_and_orphaned_fields_are_each_reported() {
        let root = fixture(
            "{\"name\": \"user\", \"version\": \"=0.1.0\", \"source\": \"file:../user\", \
             \"requiredBy\": []}, \
             {\"name\": \"base\", \"version\": \"=0.1.0\", \"source\": \"file:../base\", \
             \"direct\": false, \"requiredBy\": [\"nobody\"]}, \
             {\"name\": \"extra\", \"version\": \"=0.1.0\", \"source\": \"file:../base\", \
             \"direct\": false}",
        );
        assert_eq!(
            check_app(root.path()),
            vec![
                Issue::MissingDirect {
                    name: "user".to_string()
                },
                Issue::RequiredBy {
                    name: "base".to_string(),
                    declared: vec!["nobody".to_string()],
                    expected: vec!["user".to_string()],
                },
                Issue::MissingRequiredBy {
                    name: "extra".to_string(),
                    expected: vec![],
                },
                Issue::Orphan {
                    name: "extra".to_string()
                },
            ]
        );
    }

    #[test]
    fn reconcile_declares_the_closure_with_a_rebased_source() {
        let root = fixture(
            "\n    {\n      \"name\": \"user\",\n      \"version\": \"=0.1.0\",\n      \
             \"source\": \"file:../user\"\n    }\n  ",
        );
        let app = root.path().join("app");
        let contents = fs::read_to_string(app.join("project.json")).unwrap();
        let result = reconcile(&app, &contents).unwrap();
        assert_eq!(result.added, vec![("base".to_string(), "user".to_string())]);
        assert!(result.unresolved.is_empty());
        let manifest = parse_project_json(&result.contents, &app.join("project.json")).unwrap();
        let declared = declared_dependencies(&manifest);
        let user = declared.iter().find(|d| d.name == "user").unwrap();
        assert_eq!(user.direct, Some(true));
        assert_eq!(user.required_by, Some(vec![]));
        let base = declared.iter().find(|d| d.name == "base").unwrap();
        assert_eq!(base.direct, Some(false));
        assert_eq!(base.required_by, Some(vec!["user".to_string()]));
        assert_eq!(base.source, "file:../base");
        // Reconciling a closed manifest changes nothing.
        let again = reconcile(&app, &result.contents).unwrap();
        assert_eq!(again.contents, result.contents);
        assert!(again.added.is_empty() && again.removed.is_empty());
    }

    #[test]
    fn reconcile_drops_orphans_transitively_and_keeps_direct_entries() {
        // `mid` (indirect) requires `base` (indirect); nothing requires `mid`.
        let root = fixture("");
        write(
            &root.path().join("mid/project.json"),
            &manifest(
                "{\"name\": \"base\", \"version\": \"=0.1.0\", \"source\": \"file:../base\", \
                 \"direct\": true, \"requiredBy\": []}",
            ),
        );
        let app = root.path().join("app");
        let contents = manifest(
            "{\"name\": \"mid\", \"version\": \"=0.1.0\", \"source\": \"file:../mid\", \
             \"direct\": false, \"requiredBy\": []}, \
             {\"name\": \"base\", \"version\": \"=0.1.0\", \"source\": \"file:../base\", \
             \"direct\": false, \"requiredBy\": [\"mid\"]}, \
             {\"name\": \"user\", \"version\": \"=0.1.0\", \"source\": \"file:../user\", \
             \"direct\": true, \"requiredBy\": []}",
        );
        let result = reconcile(&app, &contents).unwrap();
        assert_eq!(result.removed, vec!["mid".to_string()]);
        // `base` is still required, by `user`.
        let manifest = parse_project_json(&result.contents, &app.join("project.json")).unwrap();
        let names: Vec<String> = declared_dependencies(&manifest)
            .into_iter()
            .map(|d| format!("{}:{:?}", d.name, d.required_by))
            .collect();
        assert_eq!(
            names,
            vec![
                "base:Some([\"user\"])".to_string(),
                "user:Some([])".to_string()
            ]
        );
    }

    #[test]
    fn a_requirers_source_is_rebased_onto_the_importer() {
        let root = tempfile::tempdir().unwrap();
        let app = root.path().join("app");
        let user = root.path().join("libs/user");
        fs::create_dir_all(&app).unwrap();
        fs::create_dir_all(user.join("packages")).unwrap();
        let entry = |name: &str, source: &str| ProjectPackageDependency {
            name: name.to_string(),
            ident: name.to_string(),
            version: "=0.1.0".to_string(),
            pin: false,
            source: source.to_string(),
            ident_key: String::new(),
            direct: Some(true),
            required_by: Some(Vec::new()),
        };
        // A source directory, relative to the requirer.
        assert_eq!(
            rebase_source(&app, &user, &entry("base", "file:../base")),
            ("file:../libs/base".to_string(), None)
        );
        // The conventional `packages/<name>/` directory.
        assert_eq!(
            rebase_source(&app, &user, &entry("base", "")),
            ("file:../libs/user/packages/base".to_string(), None)
        );
        // Absolute and registry sources are the same everywhere.
        assert_eq!(
            rebase_source(&app, &user, &entry("base", "local:///opt/base")),
            ("local:///opt/base".to_string(), None)
        );
        assert_eq!(
            rebase_source(&app, &user, &entry("base", "ada#base")),
            ("ada#base".to_string(), None)
        );
        // An installed .mfp wins over the source beside it, and is installed.
        let installed = user.join("packages/base.mfp");
        fs::write(&installed, b"mfp").unwrap();
        assert_eq!(
            rebase_source(&app, &user, &entry("base", "file:../base")),
            ("file:packages/base.mfp".to_string(), Some(installed))
        );
    }

    #[test]
    fn relative_paths_walk_up_to_the_common_ancestor() {
        let root = tempfile::tempdir().unwrap();
        let app = root.path().join("app");
        let base = root.path().join("libs/base");
        fs::create_dir_all(&app).unwrap();
        fs::create_dir_all(&base).unwrap();
        assert_eq!(relative_path(&app, &base).as_deref(), Some("../libs/base"));
        assert_eq!(relative_path(&app, &app).as_deref(), Some("."));
        assert_eq!(
            relative_path(&app, &app.join("packages/x.mfp")).as_deref(),
            Some("packages/x.mfp")
        );
    }
}
