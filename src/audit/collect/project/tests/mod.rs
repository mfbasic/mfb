use super::*;
use crate::ast::{
    AbiSpec, AstFile, AstProject, FreeSpec, LinkBlock, LinkFunction, ResourceDecl, Visibility,
};
use std::path::Path;
use tinyjson::JsonValue;

fn internal_file(path: &str, items: Vec<Item>) -> AstFile {
    AstFile {
        path: path.to_string(),
        imports: Vec::new(),
        items,
        internal: true,
    }
}

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
                result_length: None,
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
fn internal_files_are_excluded_from_resources_and_links() {
    // An `internal: true` file (compiler-injected package source) holds the
    // LINK block and a RESOURCE. Its resource must be excluded (bug-279), yet
    // the `close_may_fail` lookup still scans it so the derivation resolves.
    let link = link_block("db", vec![("close", Some(Expression::Boolean(true)))]);
    let internal = internal_file(
        "builtins/db.mfb",
        vec![
            Item::Link(link),
            Item::Resource(resource_decl(
                "Hidden",
                "db.close",
                true,
                Visibility::Export,
                2,
            )),
        ],
    );
    // A real project file with its own resource pointing at the internal LINK.
    let own = file(
        "main.mfb",
        vec![Item::Resource(resource_decl(
            "Db",
            "db.close",
            true,
            Visibility::Export,
            7,
        ))],
    );
    let ast = AstProject {
        name: "pkg".to_string(),
        files: vec![internal, own],
    };
    let resources = collect_native_resources("pkg", &ast);
    assert_eq!(resources.len(), 1);
    assert_eq!(resources[0].resource_type, "Db");
    // close_may_fail still resolves from the internal file's LINK table.
    assert!(resources[0].close_may_fail);

    // The internal file's LINK symbols are absent from the link report.
    let links = collect_native_links("pkg", &ast);
    assert!(links.is_empty());
}

#[test]
fn native_link_free_symbol_populates_close_function() {
    let mut link = link_block("db", vec![("prepare", None)]);
    link.functions[0].free = Some(FreeSpec {
        slot: "return".to_string(),
        symbol: "sqlite3_free".to_string(),
        param_name: "ptr".to_string(),
        param_ctype: "CPtr".to_string(),
        return_ctype: "CVoid".to_string(),
        line: 1,
    });
    let ast = AstProject {
        name: "pkg".to_string(),
        files: vec![file("lib.mfb", vec![Item::Link(link)])],
    };
    let links = collect_native_links("pkg", &ast);
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].close_function, "sqlite3_free");
}

#[test]
fn collect_libraries_reads_locators_and_resource_files() {
    let manifest_json: JsonValue = r#"{
            "libraries": {
                "sqlite3": [
                    {"os": "macos", "source": "./vendor/evil.dylib"},
                    {"os": "linux", "arch": "x86_64", "libc": "gnu", "type": "system", "source": "sqlite3"}
                ]
            },
            "resources": [
                {"src": "z.txt", "dst": "out/z.txt"},
                {"dst": "no-src.txt"},
                "not-an-object",
                {"src": "a.txt", "dst": "out/a.txt"}
            ]
        }"#
    .parse()
    .unwrap();
    let manifest = manifest_json
        .get::<HashMap<String, JsonValue>>()
        .unwrap()
        .clone();

    let (libraries, resources) = collect_libraries(&manifest);

    // Two locators under the single logical "sqlite3", in manifest order.
    assert_eq!(libraries.len(), 2);
    assert_eq!(libraries[0].logical, "sqlite3");
    assert_eq!(libraries[0].os, "macos");
    assert_eq!(libraries[0].source, "./vendor/evil.dylib");
    // Absent `type` defaults to vendor.
    assert_eq!(libraries[0].lib_type, "vendor");
    assert_eq!(libraries[1].os, "linux");
    assert_eq!(libraries[1].arch.as_deref(), Some("x86_64"));
    assert_eq!(libraries[1].lib_type, "system");

    // The missing-src and non-object entries are skipped; the two valid ones
    // are sorted by src ("a.txt" before "z.txt").
    assert_eq!(resources.len(), 2);
    assert_eq!(resources[0].src, "a.txt");
    assert_eq!(resources[0].dst, "out/a.txt");
    assert_eq!(resources[1].src, "z.txt");
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
