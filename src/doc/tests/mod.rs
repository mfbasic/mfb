use super::*;
use crate::binary_repr::{DeclDocEntry, PackageDocs};

/// **plan-126-D Phase 2 — proves the anchor helpers are shared, not
/// duplicated.**
///
/// `from_source` stays in this crate (it needs the AST) and `from_package`
/// moved to `mfb_wire::docpage`, so this is the one place both are reachable
/// — the test cannot live in `mfb_wire`, which has no parser. Hand the same
/// declaration names, in the same order, to both entry points and require
/// the same anchors.
///
/// `intro` is deliberate: it collides with the renderer-owned page-intro id
/// (bug-299 D3), so it exercises `reserved_anchors` *and* `anchor`'s
/// de-duplication. The literal expectation is asserted as well as the
/// equality, so a regression that changed both paths identically still
/// fails.
#[test]
fn from_source_and_from_package_assign_identical_anchors() {
    let src = "\
DOC
  FUNC intro
  DESC First.
END DOC
EXPORT FUNC intro() AS Integer
  RETURN 1
END FUNC
DOC
  FUNC add_up
  DESC Second.
END DOC
EXPORT FUNC add_up() AS Integer
  RETURN 2
END FUNC
";
    let path = std::path::Path::new("anchor_parity.mfb");
    let file = crate::ast::parse_source(path, "anchor_parity.mfb", src).expect("parse source");
    let ast = AstProject {
        name: "parity".to_string(),
        files: vec![file],
    };
    let from_src = from_source(&ast);

    let decl = |name: &str| DeclDocEntry {
        kind: "func".to_string(),
        name: name.to_string(),
        signature: String::new(),
        group: String::new(),
        desc: Vec::new(),
        args: Vec::new(),
        props: Vec::new(),
        ret: String::new(),
        errors: Vec::new(),
        example: String::new(),
        internal: false,
        deprecated: None,
    };
    let from_pkg = from_package(
        PackageDocs {
            package: None,
            decls: vec![decl("intro"), decl("add_up")],
        },
        "parity",
    );

    let anchors = |page: &DocPage| -> Vec<String> {
        page.public
            .iter()
            .flat_map(|group| group.decls.iter().map(|d| d.anchor.clone()))
            .collect()
    };
    let source_anchors = anchors(&from_src);
    let package_anchors = anchors(&from_pkg);

    assert_eq!(
        source_anchors, package_anchors,
        "the two entry points must share one anchor implementation"
    );
    assert_eq!(package_anchors, ["intro-2", "add-up"]);
}

/// A documented resource is listed with the RESOURCE line as its signature,
/// public when exported.
#[test]
fn a_documented_resource_is_listed_by_its_declaration() {
    let src = "\
IMPORT io
DOC
  RESOURCE Handle
  DESC An open handle.
END DOC
EXPORT RESOURCE Handle CLOSE BY io::print
";
    let path = std::path::Path::new("resource_doc.mfb");
    let file = crate::ast::parse_source(path, "resource_doc.mfb", src).expect("parse source");
    let ast = AstProject {
        name: "res".to_string(),
        files: vec![file],
    };
    let page = from_source(&ast);
    let decls: Vec<&DocDecl> = page.public.iter().flat_map(|group| &group.decls).collect();
    assert_eq!(
        decls.len(),
        1,
        "{:?}",
        decls.iter().map(|d| &d.name).collect::<Vec<_>>()
    );
    assert_eq!(decls[0].name, "Handle");
    assert!(
        decls[0].signature.contains("RESOURCE Handle"),
        "{}",
        decls[0].signature
    );
}
