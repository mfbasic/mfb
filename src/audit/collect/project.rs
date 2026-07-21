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
mod tests {
    use super::*;
    use crate::ast::{
        AbiSpec, AstFile, AstProject, LinkBlock, LinkFunction, ResourceDecl, Visibility,
    };
    use std::path::Path;
    use tinyjson::JsonValue;

    fn resource_decl(
        name: &str,
        close_fn: &str,
        sendable: bool,
        vis: Visibility,
        line: usize,
    ) -> ResourceDecl {
        ResourceDecl {
            visibility: vis,
            name: name.to_string(),
            close_fn: close_fn.to_string(),
            thread_sendable: sendable,
            line,
        }
    }

    fn link_block(alias: &str, funcs: Vec<(&str, Option<Expression>)>) -> LinkBlock {
        LinkBlock {
            library: "lib".to_string(),
            alias: alias.to_string(),
            cstructs: Vec::new(),
            functions: funcs
                .into_iter()
                .map(|(name, success_on)| LinkFunction {
                    name: name.to_string(),
                    params: Vec::new(),
                    return_type: None,
                    return_resource: false,
                    return_state_type: None,
                    symbol: name.to_string(),
                    abi: AbiSpec {
                        slots: Vec::new(),
                        return_name: String::new(),
                        return_ctype: String::new(),
                        line: 1,
                    },
                    consts: Vec::new(),
                    bind_in: Vec::new(),
                    bind_state: None,
                    buffers: Vec::new(),
                    success_on,
                    result: None,
                    free: None,
                    line: 1,
                })
                .collect(),
            line: 1,
        }
    }

    fn file(path: &str, items: Vec<Item>) -> AstFile {
        AstFile {
            path: path.to_string(),
            imports: Vec::new(),
            items,
            internal: false,
        }
    }

    #[test]
    fn native_resources_derive_close_may_fail_from_link_success_on() {
        // link alias `db` has close op `close` gated with SUCCESS_ON -> may fail;
        // `freeIt` without a gate -> may not fail.
        let link = link_block(
            "db",
            vec![("close", Some(Expression::Boolean(true))), ("freeIt", None)],
        );
        let resources = vec![
            Item::Resource(resource_decl("Db", "db.close", true, Visibility::Export, 5)),
            Item::Resource(resource_decl(
                "Cursor",
                "db.freeIt",
                false,
                Visibility::Private,
                9,
            )),
        ];
        let ast = AstProject {
            name: "pkg".to_string(),
            files: vec![file("lib.mfb", {
                let mut items = vec![Item::Link(link)];
                items.extend(resources);
                items
            })],
        };
        let out = collect_native_resources("pkg", &ast);
        assert_eq!(out.len(), 2);
        // sorted by path, line, resource_type -> Db (line 5) then Cursor (line 9)
        assert_eq!(out[0].resource_type, "Db");
        assert!(out[0].close_may_fail);
        assert!(out[0].sendable);
        assert!(out[0].exported);
        assert_eq!(out[0].package, "pkg");
        assert_eq!(out[0].close_op, "db.close");

        assert_eq!(out[1].resource_type, "Cursor");
        assert!(!out[1].close_may_fail);
        assert!(!out[1].sendable);
        assert!(!out[1].exported);
    }

    #[test]
    fn native_resource_unknown_close_fn_defaults_may_fail_false() {
        let ast = AstProject {
            name: "pkg".to_string(),
            files: vec![file(
                "lib.mfb",
                vec![Item::Resource(resource_decl(
                    "Orphan",
                    "unknown.close",
                    false,
                    Visibility::Public,
                    3,
                ))],
            )],
        };
        let out = collect_native_resources("pkg", &ast);
        assert_eq!(out.len(), 1);
        assert!(!out[0].close_may_fail);
        assert!(!out[0].exported); // Package visibility is not Export
    }

    #[test]
    fn native_links_report_every_linked_symbol_sorted() {
        // `open` is gated by SUCCESS_ON (may fail); `freeIt` is not.
        let link = link_block(
            "db",
            vec![("open", Some(Expression::Boolean(true))), ("freeIt", None)],
        );
        let ast = AstProject {
            name: "pkg".to_string(),
            files: vec![file("lib.mfb", vec![Item::Link(link)])],
        };
        let links = collect_native_links("pkg", &ast);
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].symbol, "freeIt");
        assert!(!links[0].may_fail);
        assert_eq!(links[1].symbol, "open");
        assert!(links[1].may_fail);
        assert_eq!(links[1].package, "pkg");
        // No FREE block declared, so no deallocator symbol.
        assert_eq!(links[1].close_function, "");
    }

    #[test]
    fn no_resources_yields_empty() {
        let ast = AstProject {
            name: "pkg".to_string(),
            files: vec![file("lib.mfb", Vec::new())],
        };
        assert!(collect_native_resources("pkg", &ast).is_empty());
    }

    #[test]
    fn native_resources_sorted_across_files() {
        let ast = AstProject {
            name: "pkg".to_string(),
            files: vec![
                file(
                    "z.mfb",
                    vec![Item::Resource(resource_decl(
                        "Za",
                        "x.c",
                        false,
                        Visibility::Export,
                        1,
                    ))],
                ),
                file(
                    "a.mfb",
                    vec![Item::Resource(resource_decl(
                        "Ab",
                        "x.c",
                        false,
                        Visibility::Export,
                        1,
                    ))],
                ),
            ],
        };
        let out = collect_native_resources("pkg", &ast);
        assert_eq!(out[0].path, "a.mfb");
        assert_eq!(out[1].path, "z.mfb");
    }

    #[test]
    fn project_summary_reads_manifest_fields() {
        let mut manifest: HashMap<String, JsonValue> = HashMap::new();
        manifest.insert("name".to_string(), JsonValue::String("demo".to_string()));
        manifest.insert(
            "ident".to_string(),
            JsonValue::String("demo.id".to_string()),
        );
        manifest.insert(
            "version".to_string(),
            JsonValue::String("3.0.0".to_string()),
        );
        manifest.insert("mfb".to_string(), JsonValue::String("1".to_string()));
        let ast = AstProject {
            name: "demo".to_string(),
            files: Vec::new(),
        };
        let inputs = AuditInputs {
            project_dir: Path::new("."),
            root_display: "root".to_string(),
            manifest: &manifest,
            ast: &ast,
            kind: "program".to_string(),
            entry: Some("main".to_string()),
            locked: false,
        };
        let summary = project_summary(&inputs);
        assert_eq!(summary.name, "demo");
        assert_eq!(summary.ident, "demo.id");
        assert_eq!(summary.version, "3.0.0");
        assert_eq!(summary.language_version, "1");
        assert_eq!(summary.kind, "program");
        assert_eq!(summary.entry.as_deref(), Some("main"));
        assert_eq!(summary.root, "root");
    }

    #[test]
    fn project_summary_defaults_ident_to_name_and_empties() {
        let mut manifest: HashMap<String, JsonValue> = HashMap::new();
        manifest.insert(
            "name".to_string(),
            JsonValue::String("only-name".to_string()),
        );
        let ast = AstProject {
            name: "x".to_string(),
            files: Vec::new(),
        };
        let inputs = AuditInputs {
            project_dir: Path::new("."),
            root_display: ".".to_string(),
            manifest: &manifest,
            ast: &ast,
            kind: "library".to_string(),
            entry: None,
            locked: false,
        };
        let summary = project_summary(&inputs);
        assert_eq!(summary.ident, "only-name"); // defaults to name
        assert_eq!(summary.version, "");
        assert_eq!(summary.language_version, "");
        assert!(summary.entry.is_none());
    }
}
