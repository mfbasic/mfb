//! Resolver diagnostics, collected by detail: a duplicated re-export alias, a
//! RESOURCE doc header naming nothing or something that is not a resource, a
//! LINK parameter default naming nothing, and an import of a declared package
//! that is not installed.

use super::*;

/// Resolve an inline project whose `src/main.mfb` is `source`, its manifest
/// carrying `packages` (a JSON array), and collect every diagnostic.
fn diagnostics(source: &str, packages: &str) -> Vec<(String, String)> {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("project.json"),
        format!(
            r#"{{ "name": "scratch", "version": "0.1.0", "mfb": "1.0", "kind": "executable",
                 "sources": [{{ "root": "src", "role": "main", "include": ["**/*.mfb"] }}],
                 "packages": {packages},
                 "entry": "main", "targets": ["native"] }}"#
        ),
    )
    .unwrap();
    fs::write(root.join("src").join("main.mfb"), source).unwrap();
    let manifest = validate_project_manifest(&root.join("project.json")).unwrap();
    crate::rules::collecting_diagnostics(|| {
        let ast = crate::ast::parse_project("scratch", root, &manifest).expect("parses");
        let _ = crate::resolver::resolve_project(root, &manifest, &ast);
    })
    .1
}

const MAIN: &str =
    "IMPORT io\n\nFUNC main() AS Integer\n  io::print(\"x\")\n  RETURN 0\nEND FUNC\n";

const LINK: &str = "RESOURCE Db CLOSE BY demoLink::close\n\n\
    LINK \"demo\" AS demoLink\n  FUNC close(db AS Db) AS Nothing\n    SYMBOL \"demo_close\"\n    \
    ABI (db CPtr) AS status CInt32\n    SUCCESS_ON status = 0\n  END FUNC\nEND LINK\n";

#[test]
fn a_second_re_export_alias_of_the_same_function_is_a_duplicate() {
    let source = format!(
        "{LINK}\nEXPORT FUNC close AS demoLink::close\nEXPORT FUNC close AS demoLink::close\n\n{MAIN}"
    );
    let found = diagnostics(&source, "[]");
    assert!(
        found.contains(&(
            "SYMBOL_DUPLICATE_TOP_LEVEL".to_string(),
            "Top-level symbol `close` was already declared in src/main.mfb:11.".to_string()
        )),
        "{found:?}"
    );
}

#[test]
fn a_resource_doc_header_must_name_a_resource() {
    let doc =
        |name: &str| format!("{LINK}\nDOC\n  RESOURCE {name}\n  DESC Docs.\nEND DOC\n\n{MAIN}");
    let unresolved = diagnostics(&doc("Nope"), "[]");
    assert!(
        unresolved.contains(&(
            "DOC_UNRESOLVED".to_string(),
            "DOC header `RESOURCE Nope` does not name a RESOURCE in this package.".to_string()
        )),
        "{unresolved:?}"
    );
    let mismatched = diagnostics(&doc("main"), "[]");
    assert!(
        mismatched.contains(&(
            "DOC_NAME_MISMATCH".to_string(),
            "DOC header `RESOURCE main` does not name a RESOURCE in this package.".to_string()
        )),
        "{mismatched:?}"
    );
}

#[test]
fn a_link_parameter_default_is_resolved_where_it_is_declared() {
    let source = format!(
        "LINK \"demo\" AS demoLink\n  FUNC level(n AS Integer = nowhere) AS Integer\n    \
         SYMBOL \"demo_level\"\n    ABI (n CInt64) AS result CInt64\n  END FUNC\nEND LINK\n\n{MAIN}"
    );
    let found = diagnostics(&source, "[]");
    assert!(
        found.iter().any(|(_, detail)| detail.contains("nowhere")),
        "{found:?}"
    );
}

#[test]
fn importing_a_declared_package_that_is_not_installed_is_refused() {
    let found = diagnostics(
        &format!("IMPORT base\n{MAIN}"),
        r#"[{ "name": "base", "version": "1.0.0", "source": "file:packages/base.mfp",
              "direct": true, "requiredBy": [] }]"#,
    );
    assert!(
        found
            .iter()
            .any(|(rule, detail)| rule == "IMPORT_PACKAGE_NOT_INSTALLED"
                && detail.starts_with("Declared package `base` was not found at `")),
        "{found:?}"
    );
}
