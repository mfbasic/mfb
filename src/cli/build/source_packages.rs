//! Compiling dependencies declared by SOURCE DIRECTORY (bug-480 Defect A).
//!
//! A `packages` entry may name a directory of MFBASIC sources rather than an
//! installed `.mfp` (`13_modules-and-packages.md`, `07_cli-reference.md`):
//!
//! ```json
//! { "name": "tiny", "version": "=0.1.0", "source": "file:packages/tiny" }
//! ```
//!
//! The resolver found such a dependency but nothing ever compiled it, so none of
//! its exported functions acquired a type and every call into it evaluated to
//! `Unknown`. The fix is one step, not a second interface path: build the
//! dependency into a real `.mfp` in this build's package cache
//! ([`source_package_cache_dir`]) and let the existing `.mfp` machinery —
//! signatures, type defs, resource closers, monomorph overloads, the shape pass,
//! `merge_packages` — read it unchanged.
//!
//! Nothing here is written to `mfb.lock` and nothing is installed into
//! `packages/`: a source dependency's compiled interface is a build
//! intermediate, re-derived from source on every build, which is exactly why it
//! is the standing remedy for a committed `.mfp` going stale.

use super::*;

/// How deep a chain of source dependencies may nest before the build refuses.
///
/// The in-progress set below already makes a *cycle* impossible, so this is the
/// second bound: a pathological but acyclic chain still costs one full compile
/// per link, and a build that has descended this far is a mistake worth naming
/// rather than waiting out.
const MAX_SOURCE_DEPENDENCY_DEPTH: usize = 32;

thread_local! {
    /// The projects whose source dependencies are currently being compiled, in
    /// entry order. A dependency that reappears here depends transitively on
    /// itself, so building it would not terminate.
    ///
    /// Thread-local rather than a parameter because [`build_project`] is the
    /// recursion point and its signature is the CLI's; a build is
    /// single-threaded from `main` down.
    static IN_PROGRESS: std::cell::RefCell<Vec<PathBuf>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// A guard that keeps `project_dir` in the in-progress set for its lifetime, so
/// the entry is removed even when a nested build returns `Err`.
struct InProgress;

impl InProgress {
    fn enter(project_dir: PathBuf) -> InProgress {
        IN_PROGRESS.with(|stack| stack.borrow_mut().push(project_dir));
        InProgress
    }
}

impl Drop for InProgress {
    fn drop(&mut self) {
        IN_PROGRESS.with(|stack| {
            stack.borrow_mut().pop();
        });
    }
}

/// Canonicalize for identity comparison, falling back to the path as written
/// when the directory does not exist (it is about to be reported as missing).
fn identity(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Compile every dependency this project declares by source directory into
/// `build/packages/<name>.mfp`, so the rest of the build resolves it exactly
/// like an installed `.mfp`.
///
/// Runs before `verify_and_report_packages` — that report classifies whatever
/// the shared resolver finds, and for a source dependency that is the artifact
/// written here.
///
/// The cache is cleared first, so a stale entry for a dependency that has since
/// been removed, renamed, or converted to an installed `.mfp` can never be
/// resolved against.
///
/// Takes the caller's already-validated manifest rather than re-reading it:
/// `validate_project_manifest` PRINTS its findings, so a second call would
/// duplicate every manifest warning in the build output.
pub(super) fn build_source_dependencies(
    options: &BuildOptions,
    manifest: &HashMap<String, JsonValue>,
) -> Result<(), ()> {
    let cache = crate::manifest::package::source_package_cache_dir(&options.location);
    if let Err(err) = std::fs::remove_dir_all(&cache) {
        if err.kind() != std::io::ErrorKind::NotFound {
            eprintln!("error: failed to clear '{}': {err}", cache.display());
            return Err(());
        }
    }

    let dependencies = source_dependency_dirs(&options.location, manifest);
    if dependencies.is_empty() {
        return Ok(());
    }

    let here = identity(&options.location);
    let (depth, cycle) = IN_PROGRESS.with(|stack| {
        let stack = stack.borrow();
        (stack.len(), stack.iter().any(|entry| *entry == here))
    });
    if cycle || depth >= MAX_SOURCE_DEPENDENCY_DEPTH {
        report_dependency_cycle(
            &options.location.join("project.json"),
            &options.location,
            cycle,
        );
        return Err(());
    }
    let _guard = InProgress::enter(here);

    if let Err(err) = std::fs::create_dir_all(&cache) {
        eprintln!("error: failed to create '{}': {err}", cache.display());
        return Err(());
    }

    for (_, dir) in dependencies {
        build_source_dependency(options, &dir, &cache)?;
    }
    Ok(())
}

/// The `(name, directory)` of every dependency that must be compiled from
/// source: declared by source directory, not already installed as a
/// `packages/<name>.mfp`, and holding a readable `project.json`.
///
/// A dependency with an installed `.mfp` is skipped whatever its `source` says —
/// the compiled form is what `resolved_package_file` will pick, so recompiling
/// the sources beside it would produce an artifact nothing reads.
fn source_dependency_dirs(
    project_dir: &Path,
    manifest: &HashMap<String, JsonValue>,
) -> Vec<(String, PathBuf)> {
    let Some(packages) = manifest
        .get("packages")
        .and_then(|value| value.get::<Vec<JsonValue>>())
    else {
        return Vec::new();
    };
    packages
        .iter()
        .filter_map(crate::manifest::package::project_package_dependency)
        .filter_map(|dependency| {
            if crate::manifest::package::validate_package_name(&dependency.name).is_err() {
                return None;
            }
            if project_dir
                .join("packages")
                .join(format!("{}.mfp", dependency.name))
                .is_file()
            {
                return None;
            }
            let source = Some(dependency.source.as_str());
            match crate::manifest::package::source_dependency(project_dir, &dependency.name, source)
            {
                crate::manifest::package::SourceDependency::Directory(dir)
                    if dir.join("project.json").is_file() =>
                {
                    Some((dependency.name, dir))
                }
                // A directory that does not exist, a non-absolute `local://`
                // path, and a `.mfp`/registry source all belong to the resolver,
                // which reports them against the `IMPORT` line that needs them.
                // Not every declared dependency is imported, so refusing here
                // would fail builds the resolver passes.
                _ => None,
            }
        })
        .collect()
}

/// Compile one source dependency, writing its `.mfp` straight into the
/// importer's package cache.
///
/// The nested build inherits the importer's target, optimization level and
/// verbosity: the dependency's code is linked into the importer's binary, so it
/// must be compiled for the same machine with the same dial. It never inherits
/// `mode`, `--app`, or `--sign`: those describe the *executable* being produced,
/// and a dependency is always an ordinary unsigned package build.
///
/// The artifact is named from the DEPENDENCY's own manifest, so a directory
/// whose `project.json` disagrees with the importing entry writes a file the
/// importer never looks for. That mismatch is the resolver's to report, against
/// the `IMPORT` line that needs it (`IMPORT_PACKAGE_NAME_MISMATCH`); leaving the
/// cache without the entry is the whole of this function's part in it.
fn build_source_dependency(options: &BuildOptions, dir: &Path, cache: &Path) -> Result<(), ()> {
    build_project(&BuildOptions {
        location: dir.to_path_buf(),
        outputs: Vec::new(),
        package_output_dir: Some(cache.to_path_buf()),
        target: options.target.clone(),
        sign_owner: None,
        app_mode: false,
        app_debug: false,
        opt: options.opt,
        allow_unsigned: options.allow_unsigned,
        mode: crate::testing::CompileMode::Build,
        verbosity: options.verbosity,
    })
}

/// Empty `build/` of everything a previous build left there (plan-55-A §4.2),
/// keeping only the source-dependency package cache.
///
/// The cache is not build OUTPUT: `build_source_dependencies` filled it at the
/// top of this same build, and the `Vec<PathBuf>` the executable path is about
/// to hand to `write_executable` points into it. It is emptied and refilled once
/// per build by its owner, so nothing stale can survive there either.
pub(super) fn clear_build_dir(build_dir: &Path) -> std::io::Result<()> {
    let entries = match std::fs::read_dir(build_dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err),
    };
    let cache = std::path::Path::new(crate::manifest::package::SOURCE_PACKAGE_CACHE_DIR);
    for entry in entries {
        let entry = entry?;
        if std::path::Path::new(&entry.file_name()) == cache {
            continue;
        }
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            std::fs::remove_dir_all(&path)?;
        } else {
            std::fs::remove_file(&path)?;
        }
    }
    Ok(())
}

/// A dependency graph that contains its own importer, or nests past
/// [`MAX_SOURCE_DEPENDENCY_DEPTH`].
///
/// Reported as `IMPORT_PACKAGE_MANIFEST_INVALID` located at the offending
/// project's `packages` field: the manifest that closes the loop is the one the
/// developer has to edit, and it is the manifest — not any source line — that is
/// wrong.
fn report_dependency_cycle(project_path: &Path, project_dir: &Path, cycle: bool) {
    let contents = std::fs::read_to_string(project_path).unwrap_or_default();
    let (line, column) = crate::manifest::field_position(&contents, "packages");
    let detail = if cycle {
        format!(
            "source package `{}` depends on itself through its `packages` entries; a package may not be part of its own dependency graph.",
            project_dir.display()
        )
    } else {
        format!(
            "source package dependencies nest more than {MAX_SOURCE_DEPENDENCY_DEPTH} levels deep at `{}`; flatten the graph.",
            project_dir.display()
        )
    };
    rules::show_diagnostic(
        "IMPORT_PACKAGE_MANIFEST_INVALID",
        &detail,
        project_path,
        line,
        column,
        column + "\"packages\"".len(),
    );
}
