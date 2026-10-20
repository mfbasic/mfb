use super::*;
use crate::html::escape;

// --- HTML rendering -------------------------------------------------------

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
mod tests;
