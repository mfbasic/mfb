//! The renderable documentation page model (plan-126-D Phase 2; moved from
//! `src/doc/mod.rs`).
//!
//! # One model, several renderers
//!
//! A [`DocPage`] is presentation-independent: groups, anchors, subtitle and
//! prose, with no markup. The compiler's standalone `mfb doc` HTML and the
//! registry's Docs tab (plan-126-F) are two renderers over this one model, so a
//! package's documentation groups, orders and anchors the same way wherever it
//! is shown. Moving only the *model* — not `src/doc/html.rs` — is deliberate:
//! the compiler's renderer emits an inline `<style>` that the registry's CSP
//! blocks, so the shareable part is the data, not the markup.
//!
//! # Two entry points, one here
//!
//! [`from_package`] builds a page from a compiled package's section 17 and lives
//! here. `from_source`, which builds one from parsed source, stays in the
//! compiler because it needs the AST. It calls the shared helpers below, which is
//! why they are `pub` — a deliberate widening of this crate's API so the two
//! entry points share one anchor/grouping implementation rather than drifting.
//!
//! **Not every helper that moved is shared.** `prose_from_codes` is used only by
//! `from_package` (`from_source` builds prose straight from AST kinds), so it
//! stays private. The plan that moved this module counted "nine shared helpers";
//! measured, eight are shared (plan-126-D Corrections).

use crate::docs::{DocProseKind, PackageDocs};
use std::collections::HashSet;

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

pub fn kind_label(kind: &str) -> &'static str {
    match kind {
        "sub" => "Subroutine",
        "type" => "Type",
        "union" => "Union",
        "enum" => "Enum",
        "resource" => "Resource",
        _ => "Function",
    }
}

pub fn badge_class(kind: &str) -> &'static str {
    match kind {
        "sub" => "function",
        "type" => "type",
        "union" => "union",
        "enum" => "enum",
        "resource" => "resource",
        _ => "function",
    }
}

pub fn member_label(kind: &str) -> Option<&'static str> {
    match kind {
        "type" => Some("Fields"),
        "union" => Some("Variants"),
        "enum" => Some("Members"),
        _ => None,
    }
}

/// The content group a declaration belongs to: callables use their `GROUP`
/// (falling back to "Functions"); type-like kinds collect under "Types".
pub fn group_title(kind: &str, group: &str) -> String {
    match kind {
        "type" | "union" | "enum" | "resource" => "Types".to_string(),
        _ if !group.is_empty() => group.to_string(),
        _ => "Functions".to_string(),
    }
}

/// Wire prose codes → [`Prose`]. Private: only [`from_package`] reads prose as
/// wire codes; `from_source` already has AST kinds.
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
pub fn assemble_groups(items: Vec<(DocDecl, String, bool)>) -> (Vec<DocGroup>, Vec<DocGroup>) {
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

/// The page-level introduction section's HTML id. It is emitted directly by the
/// renderer rather than assigned by [`anchor`], so it must be reserved before any
/// declaration anchor is handed out (bug-299 D3) -- otherwise a declaration
/// literally named `intro` slugifies to the same id, and its sidebar link scrolls
/// to the page introduction instead of the declaration. Same collision class as
/// bug-93.1, and the same fix: seed the used-set.
pub const PAGE_INTRO_ANCHOR: &str = "intro";

/// A fresh anchor set with every renderer-owned id already reserved.
pub fn reserved_anchors() -> HashSet<String> {
    let mut used = HashSet::new();
    used.insert(PAGE_INTRO_ANCHOR.to_string());
    used
}

/// Slugify a declaration name into a unique anchor id.
///
/// (This line used to sit orphaned above `PAGE_INTRO_ANCHOR` in
/// `src/doc/mod.rs`, documenting the constant instead of this function — a doc
/// comment stranded when the constant was inserted between them. Restored to
/// the item it describes in plan-126-D.)
pub fn anchor(name: &str, used: &mut HashSet<String>) -> String {
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

/// Split the first description paragraph off as the page subtitle.
pub fn split_subtitle(prose: &mut Vec<Prose>) -> (String, Vec<Prose>) {
    if prose.first().is_some_and(|p| p.kind == DocProseKind::Desc) {
        let first = prose.remove(0);
        (first.text, std::mem::take(prose))
    } else {
        (String::new(), std::mem::take(prose))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docs::{DeclDocEntry, PackageDocEntry};

    fn decl(kind: &str, name: &str, group: &str, internal: bool) -> DeclDocEntry {
        DeclDocEntry {
            kind: kind.to_string(),
            name: name.to_string(),
            signature: format!("EXPORT {} {name}", kind.to_ascii_uppercase()),
            group: group.to_string(),
            desc: Vec::new(),
            args: Vec::new(),
            props: Vec::new(),
            ret: String::new(),
            errors: Vec::new(),
            example: String::new(),
            internal,
            deprecated: None,
        }
    }

    /// With no package entry, `from_package` uses the caller's fallback name and
    /// produces an empty page — the model half of the compiler's
    /// `from_package_no_package_uses_fallback_and_empty_render`, which also
    /// renders and therefore stays with the renderer in `src/doc/html.rs`.
    #[test]
    fn with_no_package_entry_the_fallback_name_is_used() {
        let page = from_package(PackageDocs::default(), "fallbackName");
        assert_eq!(page.package_name, "fallbackName");
        assert_eq!(page.subtitle, "");
        assert!(page.intro.is_empty());
        assert!(page.package_deprecated.is_none());
        assert!(page.public.is_empty());
        assert!(page.internal.is_empty());
    }

    /// The first `DESC` becomes the subtitle; the rest is intro; callout codes
    /// decode to their kinds; declarations group by title in first-appearance
    /// order and split into public and internal.
    #[test]
    fn from_package_builds_subtitle_intro_and_ordered_public_internal_groups() {
        let docs = PackageDocs {
            package: Some(PackageDocEntry {
                name: "mathx".to_string(),
                desc: vec![
                    (DocProseKind::Desc.code(), "The subtitle.".to_string()),
                    (DocProseKind::Warn.code(), "A warning.".to_string()),
                    (DocProseKind::Desc.code(), "More intro.".to_string()),
                ],
                deprecated: Some("use mathy".to_string()),
            }),
            decls: vec![
                decl("func", "addUp", "Math", false),
                decl("type", "Point", "", false),
                decl("func", "helper", "", true),
                decl("sub", "logIt", "Math", false),
            ],
        };
        let page = from_package(docs, "ignored");

        assert_eq!(page.package_name, "mathx");
        assert_eq!(page.subtitle, "The subtitle.");
        assert_eq!(page.intro.len(), 2);
        assert_eq!(page.intro[0].kind, DocProseKind::Warn);
        assert_eq!(page.intro[1].kind, DocProseKind::Desc);
        assert_eq!(page.package_deprecated.as_deref(), Some("use mathy"));

        // Public groups in first-appearance order: "Math" (addUp, logIt), then
        // "Types" (Point). The internal helper lands in its own list.
        let titles: Vec<&str> = page.public.iter().map(|g| g.title.as_str()).collect();
        assert_eq!(titles, ["Math", "Types"]);
        let math: Vec<&str> = page.public[0]
            .decls
            .iter()
            .map(|d| d.name.as_str())
            .collect();
        assert_eq!(math, ["addUp", "logIt"]);
        assert_eq!(page.internal.len(), 1);
        assert_eq!(page.internal[0].title, "Functions");
        assert_eq!(page.internal[0].decls[0].name, "helper");

        let point = &page.public[1].decls[0];
        assert_eq!(point.kind_label, "Type");
        assert_eq!(point.badge_class, "type");
        assert_eq!(point.member_label, Some("Fields"));
    }

    /// bug-299 D3: a declaration literally named `intro` must not take the
    /// renderer-owned page-intro id, and colliding names get distinct suffixes.
    #[test]
    fn anchors_reserve_the_page_intro_and_deduplicate() {
        let docs = PackageDocs {
            package: None,
            decls: vec![
                decl("func", "intro", "", false),
                decl("func", "add up", "", false),
                decl("func", "add-up", "", false),
            ],
        };
        let page = from_package(docs, "p");
        let anchors: Vec<&str> = page.public[0]
            .decls
            .iter()
            .map(|d| d.anchor.as_str())
            .collect();
        assert_eq!(anchors, ["intro-2", "add-up", "add-up-2"]);
        assert!(!anchors.contains(&PAGE_INTRO_ANCHOR));
    }
}
