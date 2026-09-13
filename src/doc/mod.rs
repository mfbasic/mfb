//! Documentation from source, and the HTML renderer, for `mfb doc` and
//! `mfb pkg doc`. See plan-09-doc.md §7.
//!
//! The page *model* and the compiled-package entry point `from_package` live in
//! `mfb_wire::docpage` (plan-126-D) and are re-exported here; `from_source`
//! stays because it needs the AST, and `html` stays because its inline `<style>`
//! is incompatible with the registry's CSP.
//!
//! Declarations are organized into groups (`GROUP` lines for callables, plus a
//! kind-derived "Types" group), rendered as a sidebar-navigated, card-per-symbol
//! page with info/warning/security callouts.

use crate::ast::{AstProject, DocBlock, DocHeaderKind, DocProseKind, Function, Item, Visibility};
use std::collections::HashMap;
// `HashSet` is referenced only by `src/doc/html.rs`'s *test* module, which reaches
// it through `use super::*`. The helpers that used it in production moved to
// `mfb_wire::docpage` (plan-126-D), so an unconditional import is unused in the
// non-test build. Gated rather than removed so `html.rs` stays untouched.
#[cfg(test)]
use std::collections::HashSet;

// plan-126-D Phase 2: the page model -- `DocPage`, `DocGroup`, `DocDecl`,
// `Prose` -- the shared naming/grouping/anchor helpers, and `from_package` moved
// to `mfb_wire::docpage`, so the registry's Docs tab (plan-126-F) renders the
// same model this crate's `mfb doc` HTML does. Re-exported by glob so
// `crate::doc::{DocPage, from_package}` (used by `src/cli`) and everything
// `src/doc/html.rs` reaches through `use super::*` resolve unchanged -- that file
// is untouched by the move.
//
// The moved items were DELETED here, not left beside this glob: a local item
// silently shadows a glob import rather than colliding with it.
pub use mfb_wire::docpage::*;

/// Build a page directly from parsed source. Includes non-exported declarations
/// (implicitly internal, plan-09-doc.md §2.9).
pub fn from_source(ast: &AstProject) -> DocPage {
    let mut funcs: HashMap<&str, Vec<&Function>> = HashMap::new();
    let mut type_meta: HashMap<&str, (String, bool, &'static str)> = HashMap::new();
    let mut resource_meta: HashMap<&str, (String, bool, &'static str)> = HashMap::new();
    for file in &ast.files {
        if file.internal {
            continue;
        }
        for item in &file.items {
            match item {
                Item::Function(function) => {
                    funcs
                        .entry(function.name.as_str())
                        .or_default()
                        .push(function);
                }
                Item::Type(type_decl) => {
                    let kind = match type_decl.kind {
                        crate::ast::TypeDeclKind::Type => "type",
                        crate::ast::TypeDeclKind::Union => "union",
                        crate::ast::TypeDeclKind::Enum => "enum",
                    };
                    type_meta.entry(type_decl.name.as_str()).or_insert((
                        type_decl.signature_line(),
                        type_decl.visibility == Visibility::Export,
                        kind,
                    ));
                }
                Item::Resource(resource) => {
                    resource_meta.entry(resource.name.as_str()).or_insert((
                        resource.signature_line(),
                        resource.visibility == Visibility::Export,
                        "resource",
                    ));
                }
                _ => {}
            }
        }
    }

    let package_name = ast.name.clone();
    let mut pkg_prose: Vec<Prose> = Vec::new();
    let mut package_deprecated = None;
    let mut items: Vec<(DocDecl, String, bool)> = Vec::new();
    let mut used = reserved_anchors();

    for file in &ast.files {
        if file.internal {
            continue;
        }
        for item in &file.items {
            let Item::Doc(doc) = item else {
                continue;
            };
            if doc.header_kind == DocHeaderKind::Package {
                if pkg_prose.is_empty() && package_deprecated.is_none() {
                    pkg_prose = doc
                        .desc
                        .iter()
                        .map(|p| Prose {
                            kind: p.kind,
                            text: p.text.clone(),
                        })
                        .collect();
                    package_deprecated = doc.deprecated.first().map(|(m, _)| m.clone());
                }
                continue;
            }
            let Some((kind_str, signature, exported)) =
                source_decl_meta(doc, &funcs, &type_meta, &resource_meta)
            else {
                continue;
            };
            let is_internal =
                !exported || doc.attrs.iter().any(|a| a.eq_ignore_ascii_case("INTERNAL"));
            let group = doc
                .groups
                .first()
                .map(|(n, _)| n.clone())
                .unwrap_or_default();
            let title = group_title(kind_str, &group);
            let entry = DocDecl {
                anchor: anchor(&doc.header_name, &mut used),
                kind_label: kind_label(kind_str),
                badge_class: badge_class(kind_str),
                member_label: member_label(kind_str),
                name: doc.header_name.clone(),
                signature,
                desc: doc
                    .desc
                    .iter()
                    .map(|p| Prose {
                        kind: p.kind,
                        text: p.text.clone(),
                    })
                    .collect(),
                args: doc
                    .args
                    .iter()
                    .map(|a| (a.name.clone(), a.desc.clone()))
                    .collect(),
                props: doc
                    .props
                    .iter()
                    .map(|p| (p.name.clone(), p.desc.clone()))
                    .collect(),
                ret: doc.rets.first().map(|(t, _)| t.clone()).unwrap_or_default(),
                errors: doc
                    .errors
                    .iter()
                    .map(|e| (e.code.clone(), e.desc.clone()))
                    .collect(),
                example: doc
                    .examples
                    .first()
                    .map(|(t, _)| t.clone())
                    .unwrap_or_default(),
                deprecated: doc.deprecated.first().map(|(m, _)| m.clone()),
            };
            items.push((entry, title, is_internal));
        }
    }

    let (subtitle, intro) = split_subtitle(&mut pkg_prose);
    let (public, internal) = assemble_groups(items);
    DocPage {
        package_name,
        subtitle,
        intro,
        package_deprecated,
        public,
        internal,
    }
}

/// Resolve a source DOC block to `(kind, signature, exported)`, picking the
/// overload named by the header's parameter types when present.
fn source_decl_meta(
    doc: &DocBlock,
    funcs: &HashMap<&str, Vec<&Function>>,
    type_meta: &HashMap<&str, (String, bool, &'static str)>,
    resource_meta: &HashMap<&str, (String, bool, &'static str)>,
) -> Option<(&'static str, String, bool)> {
    match doc.header_kind {
        DocHeaderKind::Func | DocHeaderKind::Sub => {
            let want_sub = doc.header_kind == DocHeaderKind::Sub;
            let list = funcs.get(doc.header_name.as_str())?;
            let mut matching = list
                .iter()
                .copied()
                .filter(|f| matches!(f.kind, crate::ast::FunctionKind::Sub) == want_sub);
            let function = match &doc.header_params {
                Some(wanted) => matching
                    .find(|f| crate::ast::param_types(f) == crate::ast::normalize_types(wanted))?,
                None => matching.next()?,
            };
            let kind = if want_sub { "sub" } else { "func" };
            Some((
                kind,
                function.signature_line(),
                function.visibility == Visibility::Export,
            ))
        }
        DocHeaderKind::Type | DocHeaderKind::Union | DocHeaderKind::Enum => {
            let (signature, exported, kind) = type_meta.get(doc.header_name.as_str())?;
            Some((kind, signature.clone(), *exported))
        }
        DocHeaderKind::Resource => {
            let (signature, exported, kind) = resource_meta.get(doc.header_name.as_str())?;
            Some((kind, signature.clone(), *exported))
        }
        DocHeaderKind::Package => None,
    }
}

mod html;

pub use html::{render_empty_html, render_html};

#[cfg(test)]
mod tests {
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
}
