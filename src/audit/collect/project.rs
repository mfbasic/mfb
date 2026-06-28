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
