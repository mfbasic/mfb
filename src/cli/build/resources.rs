use super::*;

/// The fixed (glob-free) leading directory of a resource `src` glob (plan-55-A
/// §4.3): the longest run of leading path components that contain no glob
/// metacharacter (`*`, `?`, `[`, `]`), excluding the final component (which is the
/// file or pattern to match). The result is the directory `copy_resources` walks
/// and the prefix it strips to form each match's destination-relative path.
///
/// | `src` | fixed prefix |
/// | --- | --- |
/// | `data/**/*.ogg` | `data` |
/// | `data/*.ogg` | `data` |
/// | `assets/logo.png` | `assets` |
/// | `*.ogg` | `` (project root) |
pub(super) fn resource_src_fixed_prefix(src: &str) -> String {
    let normalized = src.replace('\\', "/");
    let components: Vec<&str> = normalized.split('/').collect();
    let has_meta = |component: &str| component.contains(['*', '?', '[', ']']);
    let mut prefix: Vec<&str> = Vec::new();
    for component in &components {
        if has_meta(component) {
            break;
        }
        prefix.push(component);
    }
    // Every component was literal: the last is the file itself, so the walked
    // directory is everything before it.
    if prefix.len() == components.len() {
        prefix.pop();
    }
    prefix.join("/")
}

/// Recursively collect every regular file under `dir` into `out`.
pub(super) fn collect_files_recursive(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_files_recursive(&entry.path(), out)?;
        } else if file_type.is_file() {
            out.push(entry.path());
        }
    }
    Ok(())
}

/// Copy every file matching each resource entry's `src` glob into
/// `<resource_dir>/<dst>/…` (plan-55-A §4.3), preserving structure below the
/// glob's fixed prefix. Runs after `write_executable`, next to the vendor copy.
///
/// For each entry the fixed-prefix directory (`resource_src_fixed_prefix`) is
/// walked; every regular file whose project-relative path matches the `src` glob
/// is copied to `resource_dir/<dst>/<path-with-prefix-stripped>`. An empty match
/// set — or a `src` whose fixed-prefix directory does not exist — is a silent
/// no-op, not an error (a glob may legitimately match nothing on a checkout).
pub(super) fn copy_resources(
    project_root: &Path,
    entries: &[crate::manifest::ResourceEntry],
    resource_dir: &Path,
) -> Result<(), String> {
    for entry in entries {
        let prefix = resource_src_fixed_prefix(&entry.src);
        let walk_root = if prefix.is_empty() {
            project_root.to_path_buf()
        } else {
            project_root.join(&prefix)
        };
        // A glob whose fixed-prefix directory is absent copies nothing (§4.3).
        if !walk_root.is_dir() {
            continue;
        }
        // bug-298 defense in depth: manifest validation rejects an escaping `src`,
        // but that check is textual and this is the step that actually reads
        // files. Canonicalize and require containment, so a symlink inside the
        // project that points outside it cannot widen the read set either.
        let canonical_root = project_root.canonicalize().map_err(|err| {
            format!(
                "failed to resolve project root '{}': {err}",
                project_root.display()
            )
        })?;
        let canonical_walk = walk_root.canonicalize().map_err(|err| {
            format!(
                "failed to resolve resource source '{}': {err}",
                walk_root.display()
            )
        })?;
        if !canonical_walk.starts_with(&canonical_root) {
            return Err(format!(
                "resource source '{}' resolves to '{}', which is outside the project root '{}'",
                entry.src,
                canonical_walk.display(),
                canonical_root.display()
            ));
        }
        let mut files = Vec::new();
        collect_files_recursive(&walk_root, &mut files).map_err(|err| {
            format!(
                "failed to scan resources under '{}': {err}",
                walk_root.display()
            )
        })?;
        for file in files {
            let rel = file
                .strip_prefix(project_root)
                .unwrap_or(&file)
                .to_string_lossy()
                .replace('\\', "/");
            if !crate::ast::manifest::glob_matches(&entry.src, &rel) {
                continue;
            }
            // Destination-relative path: the match minus the fixed prefix (§4.3).
            let dest_relative = if prefix.is_empty() {
                rel.as_str()
            } else {
                rel.strip_prefix(&prefix)
                    .and_then(|rest| rest.strip_prefix('/'))
                    .unwrap_or(rel.as_str())
            };
            let to = resource_dir.join(&entry.dst).join(dest_relative);
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|err| format!("failed to create '{}': {err}", parent.display()))?;
            }
            std::fs::copy(&file, &to).map_err(|err| {
                format!(
                    "failed to copy resource '{}' to '{}': {err}",
                    file.display(),
                    to.display()
                )
            })?;
        }
    }
    Ok(())
}
