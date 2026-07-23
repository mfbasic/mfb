use super::*;

// --- HTML rendering -------------------------------------------------------

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Render inline text: backtick spans become `<code>`, everything else escaped
/// (plan-09-doc.md §2.3 — no other inline markup).
fn inline(text: &str) -> String {
    let mut out = String::new();
    let mut in_code = false;
    let mut buffer = String::new();
    for ch in text.chars() {
        if ch == '`' {
            if in_code {
                out.push_str("<code>");
                out.push_str(&escape(&buffer));
                out.push_str("</code>");
            } else {
                out.push_str(&escape(&buffer));
            }
            buffer.clear();
            in_code = !in_code;
        } else {
            buffer.push(ch);
        }
    }
    if in_code {
        out.push('`');
    }
    out.push_str(&escape(&buffer));
    out
}

fn callout(out: &mut String, class: &str, icon: &str, text: &str) {
    out.push_str(&format!(
        "        <div class=\"callout {class}\">\n          <span class=\"callout-icon\">{icon}</span>\n          <span>{}</span>\n        </div>\n",
        inline(text)
    ));
}

fn render_prose(out: &mut String, prose: &[Prose]) {
    for block in prose {
        match block.kind {
            DocProseKind::Desc => {
                out.push_str(&format!("        <p>{}</p>\n", inline(&block.text)));
            }
            DocProseKind::Warn => callout(out, "warning", "⚠️", &block.text),
            DocProseKind::Info => callout(out, "info", "ℹ️", &block.text),
            DocProseKind::Sec => callout(out, "danger", "🛡️", &block.text),
        }
    }
}

fn render_table(
    out: &mut String,
    heading: &str,
    name_col: &str,
    rows: &[(String, String)],
    code_col: bool,
) {
    if rows.is_empty() {
        return;
    }
    out.push_str(&format!(
        "        <h4>{}</h4>\n        <table>\n",
        escape(heading)
    ));
    out.push_str(&format!(
        "          <tr><th>{}</th><th>Description</th></tr>\n",
        escape(name_col)
    ));
    for (name, desc) in rows {
        let name_cell = if code_col {
            format!("<span class=\"error-code\">{}</span>", escape(name))
        } else {
            format!("<code>{}</code>", escape(name))
        };
        out.push_str(&format!(
            "          <tr><td>{name_cell}</td><td>{}</td></tr>\n",
            inline(desc)
        ));
    }
    out.push_str("        </table>\n");
}

fn render_decl(out: &mut String, decl: &DocDecl) {
    out.push_str(&format!(
        "      <section id=\"{}\" class=\"section\">\n",
        decl.anchor
    ));
    out.push_str("        <div class=\"section-header\">\n");
    out.push_str(&format!(
        "          <h3><code>{}</code></h3>\n",
        escape(&decl.name)
    ));
    out.push_str(&format!(
        "          <span class=\"badge {}\">{}</span>\n",
        decl.badge_class, decl.kind_label
    ));
    out.push_str("        </div>\n");
    if !decl.signature.is_empty() {
        out.push_str(&format!(
            "        <div class=\"signature\"><pre><code>{}</code></pre></div>\n",
            escape(&decl.signature)
        ));
    }
    if let Some(message) = &decl.deprecated {
        let text = if message.is_empty() {
            "This declaration is deprecated.".to_string()
        } else {
            format!("Deprecated. {message}")
        };
        callout(out, "warning", "⚠️", &text);
    }
    render_prose(out, &decl.desc);
    render_table(out, "Parameters", "Name", &decl.args, false);
    if let Some(label) = decl.member_label {
        render_table(out, label, "Name", &decl.props, false);
    }
    if !decl.ret.is_empty() {
        out.push_str(&format!(
            "        <h4>Returns</h4>\n        <p>{}</p>\n",
            inline(&decl.ret)
        ));
    }
    render_table(out, "Errors", "Code", &decl.errors, true);
    if !decl.example.is_empty() {
        out.push_str("        <div class=\"example\">\n          <div class=\"example-label\">Example</div>\n");
        out.push_str(&format!(
            "          <pre><code>{}</code></pre>\n        </div>\n",
            escape(&decl.example)
        ));
    }
    out.push_str("      </section>\n");
}

fn render_sidebar_groups(out: &mut String, groups: &[DocGroup]) {
    for group in groups {
        out.push_str("      <div class=\"nav-section\">\n");
        out.push_str(&format!(
            "        <div class=\"nav-section-title\">{}</div>\n",
            escape(&group.title)
        ));
        for decl in &group.decls {
            out.push_str(&format!(
                "        <a href=\"#{}\" class=\"nav-item\">{}</a>\n",
                decl.anchor,
                escape(&decl.name)
            ));
        }
        out.push_str("      </div>\n");
    }
}

fn render_content_groups(out: &mut String, groups: &[DocGroup]) {
    for group in groups {
        out.push_str(&format!("      <h2>{}</h2>\n", escape(&group.title)));
        for decl in &group.decls {
            render_decl(out, decl);
        }
    }
}

/// Render a documentation page to a single self-contained HTML document
/// (plan-09-doc.md §7).
pub fn render_html(page: &DocPage) -> String {
    let name = escape(&page.package_name);
    let mut out = String::new();
    out.push_str("<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n  <meta charset=\"utf-8\">\n");
    out.push_str(&format!("  <title>{name} — Documentation</title>\n"));
    out.push_str(&format!(
        "  <style>{STYLE}</style>\n</head>\n<body>\n  <div class=\"container\">\n"
    ));

    // Sidebar.
    out.push_str("    <nav class=\"sidebar\">\n");
    out.push_str(&format!(
        "      <div class=\"sidebar-header\">\n        <h2>{name}</h2>\n      </div>\n"
    ));
    if !page.subtitle.is_empty() || !page.intro.is_empty() {
        out.push_str("      <div class=\"nav-section\">\n        <div class=\"nav-section-title\">Overview</div>\n");
        out.push_str(
            &format!("        <a href=\"#{PAGE_INTRO_ANCHOR}\" class=\"nav-item\">Introduction</a>\n      </div>\n"),
        );
    }
    render_sidebar_groups(&mut out, &page.public);
    if !page.internal.is_empty() {
        out.push_str("      <div class=\"nav-section\">\n        <div class=\"nav-section-title\">Internal</div>\n      </div>\n");
        render_sidebar_groups(&mut out, &page.internal);
    }
    out.push_str("    </nav>\n");

    // Main content.
    out.push_str("    <main class=\"main\">\n");
    out.push_str(&format!("      <h1>{name}</h1>\n"));
    if !page.subtitle.is_empty() {
        out.push_str(&format!(
            "      <p class=\"subtitle\">{}</p>\n",
            inline(&page.subtitle)
        ));
    }
    if let Some(message) = &page.package_deprecated {
        let text = if message.is_empty() {
            "This package is deprecated.".to_string()
        } else {
            format!("Deprecated. {message}")
        };
        callout(&mut out, "warning", "⚠️", &text);
    }
    if !page.intro.is_empty() {
        out.push_str(&format!("      <section id=\"{PAGE_INTRO_ANCHOR}\">\n"));
        render_prose(&mut out, &page.intro);
        out.push_str("      </section>\n");
    } else if !page.subtitle.is_empty() {
        out.push_str(&format!(
            "      <section id=\"{PAGE_INTRO_ANCHOR}\"></section>\n"
        ));
    }

    if page.public.is_empty() && page.internal.is_empty() {
        out.push_str("      <p>No documentation is available.</p>\n");
    }
    render_content_groups(&mut out, &page.public);
    if !page.internal.is_empty() {
        out.push_str("      <h2>Internal — not part of the public API</h2>\n");
        render_content_groups(&mut out, &page.internal);
    }

    out.push_str("    </main>\n  </div>\n</body>\n</html>\n");
    out
}

/// Render the minimal "no documentation" page used when a compiled package has
/// no doc section (plan-09-doc.md §6.2).
pub fn render_empty_html(name: &str) -> String {
    let page = DocPage {
        package_name: name.to_string(),
        subtitle: String::new(),
        intro: Vec::new(),
        package_deprecated: None,
        public: Vec::new(),
        internal: Vec::new(),
    };
    render_html(&page)
}

const STYLE: &str = "\
:root{--bg:#fff;--surface:#f8f9fa;--border:#e2e8f0;--text:#1a202c;--text-muted:#64748b;--accent:#3b82f6;--accent-light:#eff6ff;--danger:#ef4444;--code-bg:#f1f5f9;--sidebar-width:260px}\
*{box-sizing:border-box}\
body{font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,sans-serif;margin:0;padding:0;line-height:1.6;color:var(--text);background:var(--bg)}\
.container{display:flex;min-height:100vh}\
.sidebar{width:var(--sidebar-width);background:var(--surface);border-right:1px solid var(--border);position:sticky;top:0;height:100vh;overflow-y:auto;padding:1.5rem 0;flex-shrink:0}\
.sidebar-header{padding:0 1.5rem 1rem;border-bottom:1px solid var(--border);margin-bottom:1rem}\
.sidebar-header h2{margin:0;font-size:.875rem;text-transform:uppercase;letter-spacing:.05em;color:var(--text-muted)}\
.nav-section{margin-bottom:1.5rem}\
.nav-section-title{padding:.5rem 1.5rem;font-size:.75rem;font-weight:600;text-transform:uppercase;letter-spacing:.05em;color:var(--text-muted)}\
.nav-item{display:block;padding:.5rem 1.5rem;color:var(--text);text-decoration:none;font-size:.875rem;border-left:3px solid transparent;transition:all .15s}\
.nav-item:hover{background:var(--accent-light);color:var(--accent)}\
.main{flex:1;max-width:900px;padding:2rem 3rem}\
h1{font-size:2.25rem;font-weight:700;margin:0 0 .5rem;letter-spacing:-.025em}\
.subtitle{font-size:1.125rem;color:var(--text-muted);margin:0 0 2rem}\
h2{font-size:1.5rem;font-weight:600;margin:3rem 0 1.5rem;padding-bottom:.5rem;border-bottom:2px solid var(--border)}\
h3{font-size:1.125rem;font-weight:600;margin:0;display:flex;align-items:center;gap:.5rem}\
h4{font-size:.875rem;font-weight:600;text-transform:uppercase;letter-spacing:.025em;color:var(--text-muted);margin:1.5rem 0 .75rem}\
p{margin:0 0 1rem;color:var(--text)}\
code{font-family:'SF Mono',Monaco,Consolas,monospace;font-size:.875em;background:var(--code-bg);padding:.15em .4em;border-radius:4px}\
pre{background:var(--code-bg);padding:1rem 1.25rem;border-radius:8px;overflow-x:auto;margin:.75rem 0;border:1px solid var(--border)}\
pre code{background:none;padding:0;font-size:.8125rem;line-height:1.7}\
.signature{background:var(--accent-light);border-left:4px solid var(--accent);padding:.75rem 1rem;margin:.75rem 0;border-radius:0 6px 6px 0}\
.signature pre{background:none;border:none;padding:0;margin:0}\
.badge{display:inline-flex;align-items:center;font-size:.6875rem;font-weight:600;text-transform:uppercase;letter-spacing:.05em;padding:.25em .6em;border-radius:9999px;background:var(--surface);color:var(--text-muted);border:1px solid var(--border)}\
.badge.union{background:#f3e8ff;color:#7c3aed;border-color:#ddd6fe}\
.badge.function{background:#dbeafe;color:#1d4ed8;border-color:#bfdbfe}\
.badge.type{background:#dcfce7;color:#15803d;border-color:#bbf7d0}\
.badge.enum{background:#ffedd5;color:#c2410c;border-color:#fed7aa}\
.badge.resource{background:#ccfbf1;color:#0f766e;border-color:#99f6e4}\
table{width:100%;border-collapse:collapse;margin:.75rem 0;font-size:.875rem}\
th,td{padding:.75rem 1rem;text-align:left;vertical-align:top;border-bottom:1px solid var(--border)}\
th{font-weight:600;background:var(--surface);white-space:nowrap}\
tr:last-child td{border-bottom:none}\
.section{background:var(--bg);border:1px solid var(--border);border-radius:12px;padding:1.5rem;margin-bottom:1.5rem;scroll-margin-top:2rem}\
.section-header{display:flex;align-items:baseline;gap:.75rem;margin-bottom:1rem;flex-wrap:wrap}\
.callout{padding:.875rem 1rem;border-radius:8px;margin:1rem 0;font-size:.875rem;display:flex;gap:.75rem;align-items:flex-start}\
.callout-icon{flex-shrink:0;font-size:1rem}\
.callout.info{background:var(--accent-light);border:1px solid #bfdbfe}\
.callout.warning{background:#fffbeb;border:1px solid #fde68a}\
.callout.danger{background:#fef2f2;border:1px solid #fecaca}\
.error-code{font-family:monospace;font-size:.8125rem;color:var(--danger);font-weight:600}\
.example{background:#f8fafc;border:1px solid var(--border);border-radius:8px;padding:1rem;margin:.75rem 0}\
.example-label{font-size:.75rem;font-weight:600;text-transform:uppercase;letter-spacing:.05em;color:var(--text-muted);margin-bottom:.5rem}\
.example pre{background:none;border:none;padding:0;margin:0}\
@media (max-width:900px){.container{flex-direction:column}.sidebar{width:100%;height:auto;position:relative;border-right:none;border-bottom:1px solid var(--border)}.main{padding:1.5rem}}\
html{scroll-behavior:smooth}";

#[cfg(test)]
mod tests {
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
        assert_eq!(kind_label("sub"), "Subroutine");
        assert_eq!(kind_label("type"), "Type");
        assert_eq!(kind_label("union"), "Union");
        assert_eq!(kind_label("enum"), "Enum");
        assert_eq!(kind_label("func"), "Function");
        assert_eq!(badge_class("sub"), "function");
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

        let mut internal_fn = decl("sub", "helper", "Utilities", true);
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
  SUB shout
  DESC Shouts.
END DOC
SUB shout(msg AS String)
  io::print(msg)
END SUB
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
}
