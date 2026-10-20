use super::*;

/// Collect native `LINK` resource *type* declarations for the audit report
/// (plan-link-update.md §13). Each reports its declaring package, close op,
/// whether close may fail (derived from the close wrapper's `SUCCESS_ON` gate),
/// and thread sendability. Native pointer values are never exposed.
pub(super) fn collect_native_resources(
    package: &str,
    ast: &ast::AstProject,
) -> Vec<NativeResourceEntry> {
    use std::collections::HashMap;
    let mut close_may_fail: HashMap<String, bool> = HashMap::new();
    for file in &ast.files {
        for item in &file.items {
            if let Item::Link(link) = item {
                for function in &link.functions {
                    close_may_fail.insert(
                        format!("{}.{}", link.alias, function.name),
                        function.success_on.is_some(),
                    );
                }
            }
        }
    }
    let mut out = Vec::new();
    for file in &ast.files {
        // Report only the project's own source (bug-279); the `close_may_fail`
        // table above still scans every file because it is a lookup.
        if file.internal {
            continue;
        }
        for item in &file.items {
            if let Item::Resource(resource) = item {
                out.push(NativeResourceEntry {
                    package: package.to_string(),
                    resource_type: resource.name.clone(),
                    close_op: resource.close_fn.clone(),
                    close_may_fail: close_may_fail
                        .get(&resource.close_fn)
                        .copied()
                        .unwrap_or(false),
                    sendable: resource.thread_sendable,
                    exported: matches!(resource.visibility, ast::Visibility::Export),
                    path: file.path.clone(),
                    line: resource.line,
                });
            }
        }
    }
    out.sort_by(|a, b| {
        a.path
            .cmp(&b.path)
            .then(a.line.cmp(&b.line))
            .then(a.resource_type.cmp(&b.resource_type))
    });
    out
}

/// Collect the native symbols a project binds through `LINK` blocks. `may_fail`
/// mirrors the wrapper's `SUCCESS_ON` gate; `close_function` names the `FREE`
/// deallocator that releases a caller-owned native return, and is empty when the
/// wrapper owns nothing.
pub(super) fn collect_native_links(package: &str, ast: &ast::AstProject) -> Vec<NativeLinkEntry> {
    let mut out = Vec::new();
    for file in &ast.files {
        // Compiler-injected package source is not the project's own LINK surface
        // (bug-279).
        if file.internal {
            continue;
        }
        for item in &file.items {
            let Item::Link(link) = item else {
                continue;
            };
            for function in &link.functions {
                out.push(NativeLinkEntry {
                    package: package.to_string(),
                    symbol: function.symbol.clone(),
                    close_function: function
                        .free
                        .as_ref()
                        .map(|free| free.symbol.clone())
                        .unwrap_or_default(),
                    may_fail: function.success_on.is_some(),
                });
            }
        }
    }
    out.sort_by(|a, b| a.symbol.cmp(&b.symbol));
    out
}

pub(super) fn project_summary(inputs: &AuditInputs) -> ProjectSummary {
    let manifest = inputs.manifest;
    let name = manifest_string(manifest, "name").unwrap_or_default();
    let ident = manifest_string(manifest, "ident").unwrap_or_else(|| name.clone());
    let version = manifest_string(manifest, "version").unwrap_or_default();
    let language_version = manifest_string(manifest, "mfb").unwrap_or_default();
    ProjectSummary {
        name,
        ident,
        version,
        kind: inputs.kind.clone(),
        entry: inputs.entry.clone(),
        root: inputs.root_display.clone(),
        language_version,
    }
}

/// The manifest's `libraries` locators (plan-46) and `resources` entries
/// (plan-55) — bug-283 A3.
///
/// `mfb audit` reported `LINK` symbols but never which file each logical library
/// actually binds to, so a project pointing a benign-looking `LINK "sqlite3"` at
/// a vendored `./vendor/evil.dylib` audited identically to one using the system
/// library. The auditability spec requires surfacing linked native libraries;
/// these two sections are where a build reaches outside the source tree.
pub(super) fn collect_libraries(
    manifest: &HashMap<String, JsonValue>,
) -> (Vec<LibraryEntry>, Vec<ResourceFileEntry>) {
    let mut libraries = Vec::new();
    // `project_libraries` returns a BTreeMap, so logical names are already
    // ordered; locators keep their manifest order within a name.
    for (logical, locators) in crate::manifest::libraries::project_libraries(manifest) {
        for locator in locators {
            libraries.push(LibraryEntry {
                logical: logical.clone(),
                os: locator.os.clone(),
                arch: locator.arch.clone(),
                libc: locator.libc.map(|libc| libc.as_str().to_string()),
                lib_type: locator.lib_type.as_str().to_string(),
                source: locator.source.clone(),
            });
        }
    }

    let mut resource_files = Vec::new();
    if let Some(entries) = manifest
        .get("resources")
        .and_then(|value| value.get::<Vec<JsonValue>>())
    {
        for entry in entries {
            let Some(object) = entry.get::<HashMap<String, JsonValue>>() else {
                continue;
            };
            let field = |key: &str| {
                object
                    .get(key)
                    .and_then(|value| value.get::<String>())
                    .cloned()
                    .unwrap_or_default()
            };
            let src = field("src");
            if src.is_empty() {
                continue;
            }
            let dst = field("dst");
            resource_files.push(ResourceFileEntry { src, dst });
        }
    }
    resource_files.sort_by(|a, b| a.src.cmp(&b.src).then(a.dst.cmp(&b.dst)));

    (libraries, resource_files)
}

#[cfg(test)]
mod tests;
