//! Documentation model and HTML renderer shared by `mfb doc` (from source) and
//! `mfb pkg doc` (from a compiled `.mfp` doc section). See plan-09-doc.md §7.
//!
//! Declarations are organized into groups (`GROUP` lines for callables, plus a
//! kind-derived "Types" group), rendered as a sidebar-navigated, card-per-symbol
//! page with info/warning/security callouts.

use crate::ast::{AstProject, DocBlock, DocHeaderKind, DocProseKind, Function, Item, Visibility};
use crate::binary_repr::PackageDocs;
use std::collections::{HashMap, HashSet};

/// A renderable documentation page.
pub struct DocPage {
    pub package_name: String,
    /// First package description paragraph, shown as the page subtitle.
    pub subtitle: String,
    /// Remaining package prose (paragraphs/callouts).
    pub intro: Vec<Prose>,
    pub package_deprecated: Option<String>,
    pub public: Vec<DocGroup>,
    pub internal: Vec<DocGroup>,
}

/// A named group of declarations (a sidebar section and a content heading).
pub struct DocGroup {
    pub title: String,
    pub decls: Vec<DocDecl>,
}

/// One prose block: an ordinary paragraph or a callout.
pub struct Prose {
    pub kind: DocProseKind,
    pub text: String,
}

/// One documented declaration (plan-09-doc.md §7.2).
pub struct DocDecl {
    pub anchor: String,
    pub kind_label: &'static str,
    pub badge_class: &'static str,
    /// Heading for the members table (`Fields`/`Variants`/`Members`), or `None`.
    pub member_label: Option<&'static str>,
    pub name: String,
    pub signature: String,
    pub desc: Vec<Prose>,
    pub args: Vec<(String, String)>,
    pub props: Vec<(String, String)>,
    pub ret: String,
    pub errors: Vec<(String, String)>,
    pub example: String,
    pub deprecated: Option<String>,
}

fn kind_label(kind: &str) -> &'static str {
    match kind {
        "sub" => "Subroutine",
        "type" => "Type",
        "union" => "Union",
        "enum" => "Enum",
        "resource" => "Resource",
        _ => "Function",
    }
}

fn badge_class(kind: &str) -> &'static str {
    match kind {
        "sub" => "function",
        "type" => "type",
        "union" => "union",
        "enum" => "enum",
        "resource" => "resource",
        _ => "function",
    }
}

fn member_label(kind: &str) -> Option<&'static str> {
    match kind {
        "type" => Some("Fields"),
        "union" => Some("Variants"),
        "enum" => Some("Members"),
        _ => None,
    }
}

/// The content group a declaration belongs to: callables use their `GROUP`
/// (falling back to "Functions"); type-like kinds collect under "Types".
fn group_title(kind: &str, group: &str) -> String {
    match kind {
        "type" | "union" | "enum" | "resource" => "Types".to_string(),
        _ if !group.is_empty() => group.to_string(),
        _ => "Functions".to_string(),
    }
}

fn prose_from_codes(codes: &[(u8, String)]) -> Vec<Prose> {
    codes
        .iter()
        .map(|(code, text)| Prose {
            kind: DocProseKind::from_code(*code),
            text: text.clone(),
        })
        .collect()
}

/// Assemble grouped public/internal sections from a flat, source-ordered list of
/// `(decl, group_title, internal)`. Group order follows first appearance.
fn assemble_groups(items: Vec<(DocDecl, String, bool)>) -> (Vec<DocGroup>, Vec<DocGroup>) {
    let mut public: Vec<DocGroup> = Vec::new();
    let mut internal: Vec<DocGroup> = Vec::new();
    for (decl, title, is_internal) in items {
        let groups = if is_internal {
            &mut internal
        } else {
            &mut public
        };
        match groups.iter_mut().find(|g| g.title == title) {
            Some(group) => group.decls.push(decl),
            None => groups.push(DocGroup {
                title,
                decls: vec![decl],
            }),
        }
    }
    (public, internal)
}

/// Slugify a declaration name into a unique anchor id.
/// The page-level introduction section's HTML id. It is emitted directly by the
/// renderer rather than assigned by [`anchor`], so it must be reserved before any
/// declaration anchor is handed out (bug-299 D3) -- otherwise a declaration
/// literally named `intro` slugifies to the same id, and its sidebar link scrolls
/// to the page introduction instead of the declaration. Same collision class as
/// bug-93.1, and the same fix: seed the used-set.
const PAGE_INTRO_ANCHOR: &str = "intro";

/// A fresh anchor set with every renderer-owned id already reserved.
fn reserved_anchors() -> HashSet<String> {
    let mut used = HashSet::new();
    used.insert(PAGE_INTRO_ANCHOR.to_string());
    used
}

fn anchor(name: &str, used: &mut HashSet<String>) -> String {
    let base: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let mut candidate = base.clone();
    let mut n = 2;
    while used.contains(&candidate) {
        candidate = format!("{base}-{n}");
        n += 1;
    }
    used.insert(candidate.clone());
    candidate
}

/// Build a page from a compiled package's doc section (exported declarations
/// only, plan-09-doc.md §3).
pub fn from_package(docs: PackageDocs, fallback_name: &str) -> DocPage {
    let (package_name, mut pkg_prose, package_deprecated) = match docs.package {
        Some(package) => (
            package.name,
            prose_from_codes(&package.desc),
            package.deprecated,
        ),
        None => (fallback_name.to_string(), Vec::new(), None),
    };
    let (subtitle, intro) = split_subtitle(&mut pkg_prose);

    let mut used = reserved_anchors();
    let items = docs
        .decls
        .into_iter()
        .map(|decl| {
            let title = group_title(&decl.kind, &decl.group);
            let entry = DocDecl {
                anchor: anchor(&decl.name, &mut used),
                kind_label: kind_label(&decl.kind),
                badge_class: badge_class(&decl.kind),
                member_label: member_label(&decl.kind),
                name: decl.name,
                signature: decl.signature,
                desc: prose_from_codes(&decl.desc),
                args: decl.args,
                props: decl.props,
                ret: decl.ret,
                errors: decl.errors,
                example: decl.example,
                deprecated: decl.deprecated,
            };
            (entry, title, decl.internal)
        })
        .collect();
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
                Some(wanted) => matching.find(|f| param_types(f) == normalize(wanted))?,
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

fn param_types(function: &Function) -> Vec<String> {
    function
        .params
        .iter()
        .map(|p| crate::ast::normalize_ws(p.type_name.as_deref().unwrap_or("")))
        .collect()
}

fn normalize(types: &[String]) -> Vec<String> {
    types.iter().map(|t| crate::ast::normalize_ws(t)).collect()
}

/// Split the first description paragraph off as the page subtitle.
fn split_subtitle(prose: &mut Vec<Prose>) -> (String, Vec<Prose>) {
    if prose.first().is_some_and(|p| p.kind == DocProseKind::Desc) {
        let first = prose.remove(0);
        (first.text, std::mem::take(prose))
    } else {
        (String::new(), std::mem::take(prose))
    }
}

mod html;

pub use html::{render_empty_html, render_html};
