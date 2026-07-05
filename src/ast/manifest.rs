use super::*;

/// Synthetic path for the compiler-owned prelude file injected into every
/// project. It is excluded from `-ast` serialization so it does not perturb
/// golden AST output, but the resolver, monomorphizer, and type checker all see
/// its declarations as ordinary always-in-scope types.
pub const BUILTIN_PRELUDE_PATH: &str = "<builtin prelude>";

/// Builds the compiler-owned prelude: the always-in-scope generic record
/// templates `Pair OF A, B` and `Partition OF T` (plan-01-functions.md §4). They
/// are ordinary generic records — constructible, field-accessible, copyable, and
/// thread-sendable when their members are — handled by the existing template
/// machinery rather than special-cased like `MapEntry`.
fn builtin_prelude_file() -> AstFile {
    fn field(name: &str, type_name: &str) -> TypeField {
        TypeField {
            visibility: None,
            name: name.to_string(),
            type_name: type_name.to_string(),
            line: 0,
        }
    }
    fn template(name: &str, params: &[&str], fields: Vec<TypeField>) -> Item {
        Item::Type(TypeDecl {
            kind: TypeDeclKind::Type,
            visibility: Visibility::Export,
            name: name.to_string(),
            template_params: params.iter().map(|param| param.to_string()).collect(),
            fields,
            includes: Vec::new(),
            variants: Vec::new(),
            members: Vec::new(),
            line: 0,
        })
    }

    AstFile {
        path: BUILTIN_PRELUDE_PATH.to_string(),
        imports: Vec::new(),
        // Public prelude types (`Pair`, `Partition`) — not internal-name material.
        internal: false,
        items: vec![
            template(
                "Pair",
                &["A", "B"],
                vec![field("first", "A"), field("second", "B")],
            ),
            template(
                "Partition",
                &["T"],
                vec![
                    field("matched", "List OF T"),
                    field("unmatched", "List OF T"),
                ],
            ),
        ],
    }
}

pub fn parse_project(
    project_name: &str,
    project_dir: &Path,
    manifest: &HashMap<String, JsonValue>,
) -> Result<AstProject, ()> {
    let mut files = Vec::new();
    let canonical_project_dir = fs::canonicalize(project_dir).map_err(|err| {
        rules::show_diagnostic(
            "MFB_SOURCE_READ_FAILED",
            &format!(
                "Could not resolve project directory `{}`: {err}",
                project_dir.display()
            ),
            &project_dir.join("project.json"),
            1,
            1,
            1,
        );
    })?;

    for source_file in collect_selected_source_files(project_dir, &canonical_project_dir, manifest)?
    {
        files.push(parse_file(
            project_dir,
            &source_file.actual_path,
            &source_file.display_path,
        )?);
    }

    // Append the compiler-owned prelude last so the user's first source file
    // stays `files[0]` — the monomorphizer emits generated instantiations into
    // the first file. The prelude is still globally in scope and is filtered out
    // of `-ast` output by `AstProject::to_json`.
    files.push(builtin_prelude_file());

    let project = AstProject {
        name: project_name.to_string(),
        files,
    };
    // Inject the built-in `collections` package source when the project imports
    // it; its sentinel file is likewise filtered out of `-ast` output.
    crate::builtins::collections::augmented_project(project)
}

/// Enumerate the `.mfb` source files selected by the project manifest, for tools
/// that operate on raw source text (such as `mfb fmt`) rather than the parsed
/// AST. Returns the on-disk paths in a stable, sorted order.
/// Append the compiler-owned prelude to an already-parsed project and run the
/// built-in `collections` augmentation, mirroring the tail of [`parse_project`].
/// Test-only: lets `crate::testutil` build a project directly from source text
/// without touching the filesystem.
#[cfg(test)]
pub fn augment_with_prelude(mut project: AstProject) -> AstProject {
    project.files.push(builtin_prelude_file());
    crate::builtins::collections::augmented_project(project)
        .expect("collections augmentation should not fail for test sources")
}

pub fn selected_source_paths(
    project_dir: &Path,
    manifest: &HashMap<String, JsonValue>,
) -> Result<Vec<PathBuf>, ()> {
    let canonical_project_dir = fs::canonicalize(project_dir).map_err(|err| {
        rules::show_diagnostic(
            "MFB_SOURCE_READ_FAILED",
            &format!(
                "Could not resolve project directory `{}`: {err}",
                project_dir.display()
            ),
            &project_dir.join("project.json"),
            1,
            1,
            1,
        );
    })?;
    let files = collect_selected_source_files(project_dir, &canonical_project_dir, manifest)?;
    Ok(files.into_iter().map(|file| file.actual_path).collect())
}

pub fn write_ast(project_dir: &Path, ast: &AstProject) -> Result<PathBuf, String> {
    let ast_path = project_dir.join(format!("{}.ast", ast.name));
    fs::write(&ast_path, ast.to_json())
        .map_err(|err| format!("failed to write '{}': {err}", ast_path.display()))?;
    Ok(ast_path)
}

pub fn parse_source(path: &Path, relative_path: &str, contents: &str) -> Result<AstFile, ()> {
    parse_source_with(path, relative_path, contents, false)
}

/// Parse a compiler-injected built-in package file. Lexed in internal mode so
/// its `__`-prefixed private names become untypeable internal symbols, and
/// tagged `internal` for downstream provenance.
pub fn parse_source_internal(
    path: &Path,
    relative_path: &str,
    contents: &str,
) -> Result<AstFile, ()> {
    parse_source_with(path, relative_path, contents, true)
}

fn parse_source_with(
    path: &Path,
    relative_path: &str,
    contents: &str,
    internal: bool,
) -> Result<AstFile, ()> {
    let tokens = if internal {
        lexer::lex_with(path, contents, true)?
    } else {
        lexer::lex(path, contents)?
    };
    let ast_file = FileParser::new(path, tokens).parse()?;
    Ok(AstFile {
        path: relative_path.replace('\\', "/"),
        imports: ast_file.imports,
        items: ast_file.items,
        internal,
    })
}

fn parse_file(project_dir: &Path, actual_path: &Path, display_path: &Path) -> Result<AstFile, ()> {
    let contents = fs::read_to_string(actual_path).map_err(|err| {
        rules::show_diagnostic(
            "MFB_SOURCE_READ_FAILED",
            &err.to_string(),
            display_path,
            1,
            1,
            1,
        );
    })?;
    let relative_path = display_path
        .strip_prefix(project_dir)
        .unwrap_or(display_path)
        .to_string_lossy()
        .replace('\\', "/");
    parse_source(display_path, &relative_path, &contents)
}

#[derive(Clone, Debug)]
struct SourceEntry {
    root: String,
    include: Vec<String>,
    exclude: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct SelectedSource {
    pub(super) actual_path: PathBuf,
    pub(super) display_path: PathBuf,
}

fn source_entries(manifest: &HashMap<String, JsonValue>) -> Vec<SourceEntry> {
    manifest
        .get("sources")
        .and_then(|value| value.get::<Vec<JsonValue>>())
        .into_iter()
        .flatten()
        .filter_map(|source| source.get::<HashMap<String, JsonValue>>())
        .filter_map(|source| {
            let root = source.get("root")?.get::<String>()?.clone();
            let include = source
                .get("include")
                .and_then(|value| value.get::<Vec<JsonValue>>())
                .map(|patterns| {
                    patterns
                        .iter()
                        .filter_map(|pattern| pattern.get::<String>().cloned())
                        .collect()
                })
                .unwrap_or_else(|| vec!["**/*.mfb".to_string()]);
            let exclude = source
                .get("exclude")
                .and_then(|value| value.get::<Vec<JsonValue>>())
                .map(|patterns| {
                    patterns
                        .iter()
                        .filter_map(|pattern| pattern.get::<String>().cloned())
                        .collect()
                })
                .unwrap_or_default();
            Some(SourceEntry {
                root,
                include,
                exclude,
            })
        })
        .collect()
}

pub(super) fn collect_selected_source_files(
    project_dir: &Path,
    canonical_project_dir: &Path,
    manifest: &HashMap<String, JsonValue>,
) -> Result<Vec<SelectedSource>, ()> {
    let mut selected = Vec::new();
    let mut selected_roots = HashMap::new();

    for source_entry in source_entries(manifest) {
        let root = project_dir.join(&source_entry.root);
        if !root.exists() {
            rules::show_diagnostic(
                "MFB_SOURCE_ROOT_MISSING",
                &format!("Source root `{}` does not exist.", root.display()),
                &root,
                1,
                1,
                1,
            );
            return Err(());
        }

        let canonical_root = fs::canonicalize(&root).map_err(|err| {
            rules::show_diagnostic(
                "MFB_SOURCE_READ_FAILED",
                &format!("Could not resolve source root `{}`: {err}", root.display()),
                &root,
                1,
                1,
                1,
            );
        })?;
        if !path_within_project(&canonical_root, canonical_project_dir) {
            rules::show_diagnostic(
                "MFB_SOURCE_OUTSIDE_PROJECT",
                &format!(
                    "Source root `{}` resolves outside project directory `{}`.",
                    root.display(),
                    project_dir.display()
                ),
                &root,
                1,
                1,
                1,
            );
            return Err(());
        }

        let mut source_files = Vec::new();
        if root.is_file() {
            if root.extension().and_then(|ext| ext.to_str()) == Some("mfb") {
                source_files.push(SelectedSource {
                    actual_path: canonical_root,
                    display_path: root.clone(),
                });
            }
        } else {
            let mut visited_dirs = HashSet::new();
            collect_mfb_files(
                project_dir,
                &root,
                &root,
                canonical_project_dir,
                &source_entry,
                &mut visited_dirs,
                &mut source_files,
            )
            .map_err(|err| {
                if err.kind() != std::io::ErrorKind::PermissionDenied {
                    rules::show_diagnostic(
                        "MFB_SOURCE_READ_FAILED",
                        &format!("Could not read source root `{}`: {err}", root.display()),
                        &root,
                        1,
                        1,
                        1,
                    );
                }
            })?;
        }

        source_files.sort_by(|left, right| left.display_path.cmp(&right.display_path));

        if source_files.is_empty() {
            rules::show_diagnostic(
                "MFB_SOURCE_EMPTY",
                &format!(
                    "Source root `{}` contains no selected .mfb files.",
                    root.display()
                ),
                &root,
                1,
                1,
                1,
            );
            return Err(());
        }

        for source_file in source_files {
            if let Some(previous_root) = selected_roots.get(&source_file.actual_path) {
                rules::show_diagnostic(
                    "MFB_SOURCE_OVERLAP",
                    &format!(
                        "Source file `{}` is selected by both `{}` and `{}`.",
                        normalized_relative_path(project_dir, &source_file.display_path),
                        previous_root,
                        source_entry.root
                    ),
                    &source_file.display_path,
                    1,
                    1,
                    1,
                );
                return Err(());
            }
            selected_roots.insert(source_file.actual_path.clone(), source_entry.root.clone());
            selected.push(source_file);
        }
    }

    selected.sort_by(|left, right| left.display_path.cmp(&right.display_path));
    Ok(selected)
}

fn collect_mfb_files(
    project_dir: &Path,
    logical_root: &Path,
    current: &Path,
    canonical_project_dir: &Path,
    source_entry: &SourceEntry,
    visited_dirs: &mut HashSet<PathBuf>,
    files: &mut Vec<SelectedSource>,
) -> Result<(), std::io::Error> {
    let canonical_current = fs::canonicalize(current)?;
    // coverage:off — unreachable defensive re-check. Every directory entry is
    // validated against the project boundary at the call site below before being
    // recursed into, so a `current` that resolves outside the project is always
    // rejected there first; this pre-recursion guard never fires first.
    if !path_within_project(&canonical_current, canonical_project_dir) {
        rules::show_diagnostic(
            "MFB_SOURCE_OUTSIDE_PROJECT",
            &format!(
                "Source path `{}` resolves outside project directory `{}`.",
                current.display(),
                canonical_project_dir.display()
            ),
            current,
            1,
            1,
            1,
        );
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "source path resolves outside project",
        ));
    }
    // coverage:on
    if !visited_dirs.insert(canonical_current) {
        return Ok(());
    }

    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let canonical_path = fs::canonicalize(&path)?;
        if !path_within_project(&canonical_path, canonical_project_dir) {
            rules::show_diagnostic(
                "MFB_SOURCE_OUTSIDE_PROJECT",
                &format!(
                    "Source path `{}` resolves outside project directory `{}`.",
                    path.display(),
                    canonical_project_dir.display()
                ),
                &path,
                1,
                1,
                1,
            );
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "source path resolves outside project",
            ));
        }

        if path.is_dir() {
            collect_mfb_files(
                project_dir,
                logical_root,
                &path,
                canonical_project_dir,
                source_entry,
                visited_dirs,
                files,
            )?;
            continue;
        }

        if path.extension().and_then(|ext| ext.to_str()) != Some("mfb") {
            continue;
        }

        let relative_path = normalized_relative_path(logical_root, &path);
        if matches_source_patterns(&relative_path, &source_entry.include, &source_entry.exclude) {
            files.push(SelectedSource {
                actual_path: canonical_path,
                display_path: path,
            });
        }
    }

    Ok(())
}

fn path_within_project(path: &Path, canonical_project_dir: &Path) -> bool {
    path == canonical_project_dir || path.starts_with(canonical_project_dir)
}

fn normalized_relative_path(base: &Path, path: &Path) -> String {
    path.strip_prefix(base)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn matches_source_patterns(path: &str, include: &[String], exclude: &[String]) -> bool {
    include.iter().any(|pattern| glob_matches(pattern, path))
        && !exclude.iter().any(|pattern| glob_matches(pattern, path))
}

pub(super) fn glob_matches(pattern: &str, path: &str) -> bool {
    let normalized_pattern = pattern.replace('\\', "/");
    let normalized_path = path.replace('\\', "/");
    let pattern_segments: Vec<&str> = normalized_pattern.split('/').collect();
    let path_segments: Vec<&str> = normalized_path.split('/').collect();
    glob_match_segments(&pattern_segments, &path_segments)
}

fn glob_match_segments(pattern: &[&str], path: &[&str]) -> bool {
    match pattern.split_first() {
        None => path.is_empty(),
        Some((&"**", remaining)) => {
            glob_match_segments(remaining, path)
                || (!path.is_empty() && glob_match_segments(pattern, &path[1..]))
        }
        Some((segment, remaining)) => {
            !path.is_empty()
                && glob_match_component(segment, path[0])
                && glob_match_segments(remaining, &path[1..])
        }
    }
}

fn glob_match_component(pattern: &str, value: &str) -> bool {
    let pattern = pattern.as_bytes();
    let value = value.as_bytes();
    let mut pattern_index = 0;
    let mut value_index = 0;
    let mut star_index = None;
    let mut retry_value = 0;

    while value_index < value.len() {
        if pattern_index < pattern.len()
            && (pattern[pattern_index] == b'?' || pattern[pattern_index] == value[value_index])
        {
            pattern_index += 1;
            value_index += 1;
        } else if pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
            star_index = Some(pattern_index);
            pattern_index += 1;
            retry_value = value_index;
        } else if let Some(star) = star_index {
            pattern_index = star + 1;
            retry_value += 1;
            value_index = retry_value;
        } else {
            return false;
        }
    }

    while pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
        pattern_index += 1;
    }

    pattern_index == pattern.len()
}
