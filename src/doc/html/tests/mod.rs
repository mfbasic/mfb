use super::*;
use crate::binary_repr::{DeclDocEntry, PackageDocEntry, PackageDocs};

fn decl(kind: &str, name: &str, group: &str, internal: bool) -> DeclDocEntry {
    DeclDocEntry {
        kind: kind.to_string(),
        name: name.to_string(),
        signature: format!("FUNC {name}()"),
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

fn parse(src: &str) -> AstProject {
    let path = std::path::Path::new("doc_test.mfb");
    let file = crate::ast::parse_source(path, "doc_test.mfb", src).expect("parse source");
    AstProject {
        name: "docpkg".to_string(),
        files: vec![file],
    }
}

#[test]
fn kind_helpers_cover_every_kind() {
    assert_eq!(kind_label("type"), "Type");
    assert_eq!(kind_label("union"), "Union");
    assert_eq!(kind_label("enum"), "Enum");
    assert_eq!(kind_label("func"), "Function");
    assert_eq!(badge_class("type"), "type");
    assert_eq!(badge_class("union"), "union");
    assert_eq!(badge_class("enum"), "enum");
    assert_eq!(badge_class("func"), "function");
    assert_eq!(member_label("type"), Some("Fields"));
    assert_eq!(member_label("union"), Some("Variants"));
    assert_eq!(member_label("enum"), Some("Members"));
    assert_eq!(member_label("func"), None);
    assert_eq!(group_title("type", "Ignored"), "Types");
    assert_eq!(group_title("func", "Utilities"), "Utilities");
    assert_eq!(group_title("func", ""), "Functions");
}

#[test]
fn anchor_deduplicates_and_slugifies() {
    let mut used = HashSet::new();
    assert_eq!(anchor("Foo Bar!", &mut used), "foo-bar-");
    assert_eq!(anchor("Foo Bar!", &mut used), "foo-bar--2");
    assert_eq!(anchor("Foo Bar!", &mut used), "foo-bar--3");
}

/// bug-299 D3: the page's `<section id="intro">` is emitted directly by the
/// renderer, not assigned by `anchor`, so it was never in the used-set. A
/// declaration named `intro` therefore slugified to the same id and produced a
/// duplicate HTML id -- its sidebar link scrolled to the page introduction
/// instead of the declaration.
#[test]
fn a_declaration_named_intro_does_not_collide_with_the_page_intro() {
    let mut used = reserved_anchors();
    assert_eq!(
        anchor("intro", &mut used),
        "intro-2",
        "the page-level intro id must already be reserved"
    );
    // Case and punctuation slugify onto the same base, so they collide too.
    assert_eq!(anchor("Intro", &mut used), "intro-3");
    // An unrelated name is unaffected by the seeding.
    assert_eq!(anchor("open", &mut used), "open");
}

#[test]
fn escape_and_inline_handle_all_specials() {
    assert_eq!(escape("&<>\"x"), "&amp;&lt;&gt;&quot;x");
    // Balanced code span, then leading/trailing escaped text.
    assert_eq!(inline("a `b<c` d"), "a <code>b&lt;c</code> d");
    // Unterminated backtick: literal backtick restored, rest escaped.
    assert_eq!(inline("open `code"), "open `code");
}

#[test]
fn split_subtitle_without_leading_desc_keeps_prose() {
    // First prose block is a callout, not Desc -> no subtitle taken.
    let mut prose = vec![
        Prose {
            kind: DocProseKind::Info,
            text: "note".to_string(),
        },
        Prose {
            kind: DocProseKind::Desc,
            text: "body".to_string(),
        },
    ];
    let (subtitle, rest) = split_subtitle(&mut prose);
    assert!(subtitle.is_empty());
    assert_eq!(rest.len(), 2);
}

#[test]
fn from_package_no_package_uses_fallback_and_empty_render() {
    let docs = PackageDocs::default();
    let page = from_package(docs, "fallbackName");
    assert_eq!(page.package_name, "fallbackName");
    assert!(page.subtitle.is_empty());
    assert!(page.intro.is_empty());
    let html = render_html(&page);
    assert!(html.contains("No documentation is available."));
    assert!(html.contains("fallbackName — Documentation"));
}

#[test]
fn from_package_full_page_renders_every_element() {
    let package = PackageDocEntry {
        name: "mypkg".to_string(),
        desc: vec![
            (0, "Subtitle line.".to_string()),
            (1, "warn body".to_string()),
            (2, "info body".to_string()),
            (3, "sec body".to_string()),
        ],
        deprecated: Some("use other".to_string()),
    };
    let mut func = decl("func", "doThing", "Utilities", false);
    func.desc = vec![(0, "Does a `thing`.".to_string())];
    func.args = vec![("x".to_string(), "the x".to_string())];
    func.ret = "the result".to_string();
    func.errors = vec![("ErrBad".to_string(), "bad input".to_string())];
    func.example = "doThing(1)".to_string();
    func.deprecated = Some("gone soon".to_string());

    let mut ty = decl("type", "Widget", "", false);
    ty.props = vec![("width".to_string(), "in px".to_string())];

    let mut internal_fn = decl("func", "helper", "Utilities", true);
    internal_fn.deprecated = Some(String::new());

    let docs = PackageDocs {
        package: Some(package),
        decls: vec![func, ty, internal_fn],
    };
    let page = from_package(docs, "ignored");
    assert_eq!(page.package_name, "mypkg");
    assert_eq!(page.subtitle, "Subtitle line.");
    assert_eq!(page.intro.len(), 3);
    assert_eq!(page.package_deprecated.as_deref(), Some("use other"));
    // public groups: Utilities (func) + Types (type)
    assert_eq!(page.public.len(), 2);
    assert_eq!(page.internal.len(), 1);

    let html = render_html(&page);
    // Package deprecation callout.
    assert!(html.contains("Deprecated. use other"));
    // Callouts of all three kinds.
    assert!(html.contains("callout warning"));
    assert!(html.contains("callout info"));
    assert!(html.contains("callout danger"));
    // Decl-level deprecation (non-empty and empty).
    assert!(html.contains("Deprecated. gone soon"));
    assert!(html.contains("This declaration is deprecated."));
    // Tables: Parameters, Fields, Errors, Returns, Example, code span.
    assert!(html.contains("<h4>Parameters</h4>"));
    assert!(html.contains("<h4>Fields</h4>"));
    assert!(html.contains("<h4>Errors</h4>"));
    assert!(html.contains("<h4>Returns</h4>"));
    assert!(html.contains("class=\"error-code\""));
    assert!(html.contains("example-label"));
    assert!(html.contains("<code>thing</code>"));
    // Internal section header and sidebar Internal group.
    assert!(html.contains("Internal — not part of the public API"));
    assert!(html.contains(">Internal<"));
    assert!(html.contains(">Overview<"));
}

#[test]
fn render_empty_html_is_valid() {
    let html = render_empty_html("bare");
    assert!(html.contains("bare — Documentation"));
    assert!(html.contains("No documentation is available."));
}

#[test]
fn subtitle_without_intro_still_emits_intro_anchor() {
    let package = PackageDocEntry {
        name: "p".to_string(),
        desc: vec![(0, "Only a subtitle.".to_string())],
        deprecated: None,
    };
    let page = from_package(
        PackageDocs {
            package: Some(package),
            decls: Vec::new(),
        },
        "p",
    );
    let html = render_html(&page);
    assert!(html.contains("<section id=\"intro\"></section>"));
    assert!(html.contains("class=\"subtitle\""));
}

#[test]
fn from_source_func_type_union_enum_and_package() {
    let src = "\
DOC
  PACKAGE
  DESC The package summary.
  INFO be careful
END DOC
DOC
  FUNC greet
  DESC Greets someone.
  ARG who the person
  RET a greeting
  ERROR ErrX went wrong
  GROUP Greetings
  EXAMPLE
    greet(\"a\")
  END EXAMPLE
END DOC
EXPORT FUNC greet(who AS String) AS String
  RETURN who
END FUNC
DOC
  FUNC shout
  DESC Shouts.
END DOC
FUNC shout(msg AS String) AS Nothing
  io::print(msg)
END FUNC
DOC
  TYPE Point
  DESC A point.
  PROP x the x
END DOC
EXPORT TYPE Point
  x AS Integer
END TYPE
DOC
  UNION Shape
END DOC
EXPORT UNION Shape
  Circle
END UNION
DOC
  ENUM Color
END DOC
EXPORT ENUM Color
  Red
END ENUM
";
    let ast = parse(src);
    let page = from_source(&ast);
    assert_eq!(page.subtitle, "The package summary.");
    assert_eq!(page.intro.len(), 1);
    // greet is exported -> public; shout is not exported -> internal.
    let public_names: Vec<&str> = page
        .public
        .iter()
        .flat_map(|g| g.decls.iter().map(|d| d.name.as_str()))
        .collect();
    assert!(public_names.contains(&"greet"));
    assert!(public_names.contains(&"Point"));
    assert!(public_names.contains(&"Shape"));
    assert!(public_names.contains(&"Color"));
    let internal_names: Vec<&str> = page
        .internal
        .iter()
        .flat_map(|g| g.decls.iter().map(|d| d.name.as_str()))
        .collect();
    assert!(internal_names.contains(&"shout"));

    let html = render_html(&page);
    assert!(html.contains("greet"));
    assert!(html.contains("went wrong"));
}

#[test]
fn from_source_overload_matched_by_params() {
    let src = "\
DOC
  FUNC f(Integer)
  DESC The integer overload.
END DOC
EXPORT FUNC f(n AS Integer) AS Integer
  RETURN n
END FUNC
EXPORT FUNC f(s AS String) AS String
  RETURN s
END FUNC
";
    let ast = parse(src);
    let page = from_source(&ast);
    // Only one documented decl, and its signature is the Integer overload.
    let decls: Vec<&DocDecl> = page.public.iter().flat_map(|g| &g.decls).collect();
    assert_eq!(decls.len(), 1);
    assert!(decls[0].signature.contains("Integer"));
}

#[test]
fn from_source_internal_attribute_and_missing_target() {
    let src = "\
DOC INTERNAL
  FUNC hidden
  DESC Hidden helper.
END DOC
EXPORT FUNC hidden() AS Integer
  RETURN 0
END FUNC
DOC
  FUNC ghost
  DESC No such function.
END DOC
";
    let ast = parse(src);
    let page = from_source(&ast);
    // hidden is exported but marked INTERNAL -> internal section.
    let internal_names: Vec<&str> = page
        .internal
        .iter()
        .flat_map(|g| g.decls.iter().map(|d| d.name.as_str()))
        .collect();
    assert!(internal_names.contains(&"hidden"));
    // ghost has no matching declaration -> skipped entirely.
    let all_names: Vec<&str> = page
        .public
        .iter()
        .chain(page.internal.iter())
        .flat_map(|g| g.decls.iter().map(|d| d.name.as_str()))
        .collect();
    assert!(!all_names.contains(&"ghost"));
}

#[test]
fn from_source_package_deprecated_no_prose() {
    let src = "\
DOC
  PACKAGE
  DEPRECATED do not use
END DOC
";
    let ast = parse(src);
    let page = from_source(&ast);
    assert_eq!(page.package_deprecated.as_deref(), Some("do not use"));
    assert!(page.subtitle.is_empty());
    let html = render_html(&page);
    assert!(html.contains("Deprecated. do not use"));
}

#[test]
fn from_source_skips_internal_files_and_second_package_block() {
    // Two package DOC blocks: only the first supplies prose; the second is
    // skipped (the `pkg_prose.is_empty()` guard is false). An internal
    // file's items are ignored entirely (the two `file.internal` continues).
    let src = "\
DOC
  PACKAGE
  DESC First summary.
END DOC
DOC
  PACKAGE
  DESC Second summary should be ignored.
END DOC
DOC
  FUNC visible
  DESC A visible function.
END DOC
EXPORT FUNC visible() AS Integer
  RETURN 0
END FUNC
";
    let mut ast = parse(src);
    let mut hidden = crate::ast::parse_source(
        std::path::Path::new("hidden.mfb"),
        "hidden.mfb",
        "EXPORT FUNC secret() AS Integer\n  RETURN 1\nEND FUNC\n",
    )
    .expect("parse hidden");
    hidden.internal = true;
    ast.files.push(hidden);

    let page = from_source(&ast);
    assert_eq!(page.subtitle, "First summary.");
    let names: Vec<&str> = page
        .public
        .iter()
        .chain(page.internal.iter())
        .flat_map(|g| g.decls.iter().map(|d| d.name.as_str()))
        .collect();
    assert!(names.contains(&"visible"));
    // The internal file's `secret` never surfaces.
    assert!(!names.contains(&"secret"));
}

#[test]
fn source_decl_meta_returns_none_for_package_header() {
    // Directly exercise the Package arm of source_decl_meta, which
    // from_source never reaches because it `continue`s on Package first.
    let src = "\
DOC
  PACKAGE
  DESC hi
END DOC
";
    let ast = parse(src);
    let doc = ast.files[0].items.iter().find_map(|item| match item {
        Item::Doc(doc) if doc.header_kind == DocHeaderKind::Package => Some(doc),
        _ => None,
    });
    let doc = doc.expect("a package doc block");
    let funcs: HashMap<&str, Vec<&Function>> = HashMap::new();
    let type_meta: HashMap<&str, (String, bool, &'static str)> = HashMap::new();
    let resource_meta: HashMap<&str, (String, bool, &'static str)> = HashMap::new();
    assert_eq!(
        source_decl_meta(doc, &funcs, &type_meta, &resource_meta),
        None
    );
}

#[test]
fn package_deprecated_empty_message_uses_default_text() {
    // Empty DEPRECATED message -> the "This package is deprecated." branch.
    let page = DocPage {
        package_name: "p".to_string(),
        subtitle: String::new(),
        intro: Vec::new(),
        package_deprecated: Some(String::new()),
        public: Vec::new(),
        internal: Vec::new(),
    };
    let html = render_html(&page);
    assert!(html.contains("This package is deprecated."));
}
