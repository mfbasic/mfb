//! The anonymous, read-only web interface (plan-61-C).
//!
//! Server-rendered HTML over the plan-61-B JSON. No JavaScript is required for
//! any function on this site, no cookie is ever set or read, and every route is
//! a `GET`. Those are not incidental: the registry has **zero CSRF surface**
//! today because every authenticated route takes its session token from a JSON
//! body, and adding same-origin HTML must not change that.
//!
//! # XSS is the entire security story
//!
//! Everything the package page renders is publisher-controlled — `author`,
//! `url`, `logical`, `source`, `ident`, and later `description`. `url` is
//! capped at 2048 bytes and otherwise unvalidated, and it goes into an `href`,
//! which makes `javascript:` the obvious vector.
//!
//! Three independent layers, so no single mistake is sufficient:
//!
//! 1. **Auto-escaping templates.** `maud`'s `html!` escapes every interpolated
//!    value; the bypass (`PreEscaped`) is explicit and greppable. This inverts
//!    the failure mode of hand-rolled `format!`, where every interpolation is a
//!    place to forget one call.
//! 2. **[`safe_href`], a scheme allowlist.** Only `http` and `https` become
//!    links; everything else renders as inert text.
//! 3. **A strict `Content-Security-Policy`** on every HTML response, applied by
//!    [`html_response`] so a new page cannot forget it. Because no page needs
//!    script, `default-src 'none'` denies it outright — even a total escaping
//!    failure cannot execute.

use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use maud::{html, Markup, DOCTYPE};

/// The policy applied to every HTML response.
///
/// There is deliberately **no `script-src` permitting anything**:
/// `default-src 'none'` denies script outright, which is what makes this a real
/// defense-in-depth layer rather than a decoration. Do not add an inline
/// `<script>` or `'unsafe-inline'` later — if a feature appears to need one, it
/// does not belong on this site.
///
/// `style-src 'self'` (not `'unsafe-inline'`) is why the stylesheet is a real
/// served route rather than a `<style>` block.
pub const CONTENT_SECURITY_POLICY: &str = "default-src 'none'; style-src 'self'; img-src 'self'; \
     form-action 'self'; base-uri 'none'; frame-ancestors 'none'";

/// The shared stylesheet, compiled into the binary.
///
/// `include_str!` rather than a `ServeDir`: the project's stated principle is a
/// single self-contained binary, and an asset directory would make the server's
/// correctness depend on files next to it on disk.
pub const STYLESHEET: &str = include_str!("style.css");

/// Build an HTML response carrying the CSP.
///
/// Every HTML route goes through here. The header is attached by the builder
/// rather than by each handler precisely so that adding a page cannot omit it —
/// a per-handler convention is one someone eventually forgets.
pub fn html_response(status: StatusCode, markup: Markup) -> Response {
    (
        status,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CONTENT_SECURITY_POLICY, CONTENT_SECURITY_POLICY),
            // The pages carry no credential and nothing user-specific, but a
            // referrer leaking which package someone browsed is still avoidable.
            (header::REFERRER_POLICY, "no-referrer"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        markup.into_string(),
    )
        .into_response()
}

/// Whether a publisher-supplied URL may become an `href`.
///
/// Returns `Some(url)` only for `http://` and `https://`. Everything else is
/// `None`, and the caller renders the URL as inert text instead of a link.
///
/// The match is on the **parsed scheme**, never on a substring. A `contains`
/// or `starts_with` check is defeated by every one of the cases in this
/// function's test table:
///
/// - case variation — `JaVaScRiPt:`
/// - embedded control characters and whitespace — `java\tscript:`,
///   `java\nscript:`, which browsers strip before resolving the scheme
/// - leading whitespace or control bytes before the scheme — ` javascript:`,
///   `\x01javascript:`
/// - a missing scheme entirely — `//evil.example` is protocol-relative and
///   resolves to `https://evil.example`, so it must not pass as "relative"
///
/// Note the parse is deliberately done on a **sanitized copy**: leading and
/// interior control/whitespace bytes are removed *before* the scheme is read,
/// so this function sees what a browser would resolve, not the raw bytes.
pub fn safe_href(url: &str) -> Option<String> {
    // Browsers strip ASCII whitespace and C0 control characters when resolving
    // a URL's scheme, so strip them here too before deciding. Anything else
    // would let `java\tscript:` past a check that a browser then honours.
    let sanitized: String = url
        .chars()
        .filter(|c| !c.is_ascii_whitespace() && !c.is_control())
        .collect();

    let (scheme, rest) = sanitized.split_once(':')?;
    // A scheme is ASCII-alphanumeric plus `+-.`; anything else means the colon
    // was not a scheme separator at all (a bare `//evil.example` has no colon
    // and already returned None above).
    if scheme.is_empty()
        || !scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    {
        return None;
    }
    if !matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https") {
        return None;
    }
    // Require an authority: `http:evil` is not a usable absolute URL.
    if !rest.starts_with("//") || rest.len() <= 2 {
        return None;
    }
    // Return the original string, not the sanitized one — the sanitized copy
    // exists to *decide*, and `maud` escapes whatever is rendered. Returning a
    // rewritten URL would silently change where a legitimate link points.
    Some(url.to_string())
}

/// Render a URL as a link when its scheme is allowed, and as inert text when it
/// is not. The single place `safe_href`'s decision becomes markup, so no page
/// can render an `href` without going through the allowlist.
pub fn external_link(url: &str) -> Markup {
    match safe_href(url) {
        // `rel` blunts tabnabbing and referrer leakage on links this registry
        // does not control.
        Some(href) => html! {
            a href=(href) rel="noopener noreferrer nofollow" { (url) }
        },
        None => html! {
            span."url-inert" title="link omitted: scheme is not http or https" { (url) }
        },
    }
}

/// The shell every page shares.
///
/// Markup structure, class names and copy come from `planning/plan-61/`, which
/// §3.1 makes normative for appearance. The routes do not: every mockup `href`
/// and form `action` is repointed at the real route table here.
pub fn page(title: &str, registry_id: &str, body: Markup) -> Markup {
    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { (title) }
                link rel="stylesheet" href="/style.css";
            }
            body {
                header."masthead" {
                    div."masthead__inner" {
                        a."brand" href="/" { "mfb-repo" }
                        span."masthead__tag" {
                            "Public, anonymous, read-only registry for the MFBASIC language."
                        }
                        // A plain GET form: search works with JavaScript
                        // entirely disabled, which is a hard requirement, not a
                        // progressive-enhancement nicety.
                        form."site-search" method="get" action="/search.html" role="search" {
                            label."vh" for="q" { "Search packages" }
                            input type="search" id="q" name="q" placeholder="owner#package"
                                  autocomplete="off" spellcheck="false";
                            button type="submit" { "Search" }
                        }
                    }
                }
                main { (body) }
                footer."foot" {
                    div."foot__inner" {
                        span."mono" { "mfb-repo" }
                        span { "Registry ID " span."mono" { (registry_id) } }
                        span."foot__note" {
                            "Anonymous & read-only. Content is publisher-supplied \
                             and unverified by the registry."
                        }
                    }
                }
            }
        }
    }
}

/// `GET /` — the landing page.
///
/// The fingerprint block is the most load-bearing copy on the site, so §4's
/// constraints are enforced here rather than left to the template:
///
/// - It shows the **root** fingerprint (from the signed-metadata root), not the
///   `/ident` server fingerprint. They are different values from different
///   keys, and `mfb repo trust` consumes the root one — printing the server
///   fingerprint above that command would read as a verified copy-paste and
///   then simply fail.
/// - It is worded as *"compare this against an out-of-band source"*, never as a
///   claim that the registry is verified. An attacker serving a forged copy of
///   this page serves their own fingerprint and this same reassuring paragraph,
///   so the page authenticates nothing about itself. Getting this wrong turns a
///   convenience into security theater that actively misleads.
pub fn landing(registry_id: &str, root_fingerprint: Option<&str>) -> Markup {
    let body = html! {
        div."wrap" {
            section."hero" {
                h1 { "mfb-repo" }
                p."lede" {
                    "A package registry for the MFBASIC language. Every package, every \
                     version, and every state change is public and permanently visible — \
                     including versions that have been deprecated or yanked. Nothing here \
                     can log in, and nothing here can be changed from a browser."
                }
                form."hero-search" method="get" action="/search.html" role="search" {
                    label."vh" for="q2" { "Search packages" }
                    input type="search" id="q2" name="q"
                          placeholder="Search by owner#package or keyword"
                          autocomplete="off" spellcheck="false";
                    button type="submit" { "Search" }
                }
            }

            @if let Some(fingerprint) = root_fingerprint {
                section."fingerprint" aria-labelledby="fp-title" {
                    div."fingerprint__head" {
                        p."eyebrow" { "Compare this — do not trust it on sight" }
                        h2 id="fp-title" { "Root fingerprint" }
                    }
                    div."fingerprint__body" {
                        p {
                            "The value below is the root of the signed-metadata chain "
                            strong { "this server is presenting to you right now" }
                            ". This page cannot prove that it is the real mfb-repo. An \
                             impostor serving a forged copy of this site would show you a \
                             fingerprint too — its own — and this same reassuring \
                             paragraph. So do not act on it here."
                        }
                        code."fingerprint__value" { (fingerprint) }
                        p {
                            "Obtain the fingerprint you " em { "expect" } " from a source \
                             that does not pass through this server — your organization’s \
                             records, a colleague, the project’s signed release notes — and \
                             compare it character by character with the value above. Only \
                             if the two match exactly, pin it locally:"
                        }
                        code."cmd" { "mfb repo trust " (registry_id) "  " (fingerprint) }
                        p."fingerprint__warnoff" {
                            "If the two values differ, stop. You are not talking to the \
                             registry you think you are."
                        }
                    }
                }
            }

            h2."section-title" { "Using the registry" }
            div."prose" {
                p {
                    "Search for a package to see its versions, native target matrix, and \
                     publish history. Each package also exposes a transparency "
                    strong { "Audit" }
                    " tab: an append-only log with inclusion proofs, release-state \
                     transitions, and the registry’s identity-key rotation chain, so an \
                     independent monitor can check whether this registry is showing the \
                     same history to everyone."
                }
                p."muted" {
                    "There are no accounts and no sign-in. Every action is a plain link or \
                     a GET request; nothing on this site mutates state."
                }
            }
        }
    };
    page("mfb-repo — MFBASIC package registry", registry_id, body)
}

/// One rendered search hit.
pub struct SearchRow {
    pub ident: String,
    pub owner: String,
    /// The newest **active** release (plan-126-A). `None` for a package with no
    /// published version, and for one whose every version is yanked or blocked.
    pub latest_version: Option<String>,
    /// The release state of `latest_version`, badged beside the version chip.
    /// This field did not exist before plan-126-A, which is why a yanked
    /// headline on this page carried no marker of any kind: there was nothing
    /// for the renderer to mark it with.
    pub latest_state: Option<String>,
    pub description: Option<String>,
    pub published_at: Option<i64>,
}

/// `GET /search.html?q=` — all three states of one route.
///
/// A query that matches nothing is **HTTP 200 with a "no results" page**, not a
/// 404: the request succeeded and the answer is "none", which is different from
/// "that page does not exist". An empty query renders the form and no results,
/// never the whole table.
pub fn search_page(registry_id: &str, query: &str, results: &[SearchRow]) -> Markup {
    let body = html! {
        div."wrap" {
            @if query.trim().is_empty() {
                div."empty empty--center" role="note" {
                    h2 { "Search the registry" }
                    p {
                        "Enter an owner, a package name, or a whole identifier of the \
                         form " span."mono" { "owner#package" } " — the "
                        span."mono" { "#" } " is literal."
                    }
                    p."muted" {
                        "Nothing is listed until you search: this registry does not \
                         enumerate its whole contents to anonymous callers."
                    }
                }
            } @else {
                p."summary" {
                    strong { (results.len()) }
                    " results for "
                    span."query-echo" { (query) }
                }

                @if results.is_empty() {
                    div."empty empty--center" role="note" {
                        h2 { "No packages match this query." }
                        p {
                            "Nothing in the registry matches "
                            span."query-echo" { (query) } "."
                        }
                        p."muted" {
                            "Check spelling, or try a shorter term. Identifiers have the \
                             form " span."mono" { "owner#package" } " — the "
                            span."mono" { "#" } " is literal."
                        }
                        p { a href="/search.html" { "Start a new search" } }
                    }
                } @else {
                    ul."results" {
                        @for row in results {
                            li."result" {
                                div."result__top" {
                                    a."result__ident" href=(package_path(&row.ident)) {
                                        (row.ident)
                                    }
                                    // plan-126-A: the version chip names the
                                    // newest *active* release and carries its
                                    // state. A package whose every release is
                                    // withdrawn says so — it keeps its row in
                                    // the results (it exists, and the query
                                    // found it) but shows no version chip to
                                    // misread as current.
                                    @match &row.latest_version {
                                        Some(version) => {
                                            span."result__ver" { "v" (version) }
                                            @if let Some(release_state) = &row.latest_state {
                                                span class={
                                                    "state state--" (state_modifier(release_state))
                                                } { (release_state) }
                                            }
                                        }
                                        None => {
                                            span."result__ver result__ver--none" {
                                                "no active release"
                                            }
                                        }
                                    }
                                    @if let Some(at) = row.published_at {
                                        span."result__meta" {
                                            "published " (format_date(at))
                                        }
                                    }
                                }
                                p."result__owner" { "owner: " (row.owner) }
                                p."result__desc" {
                                    @match &row.description {
                                        Some(text) => (text),
                                        // plan-61-E fills these in; until then
                                        // this is the common state.
                                        None => span."nil" { "No description provided." },
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    };
    page(
        &format!(
            "Search — {}",
            if query.is_empty() { "mfb-repo" } else { query }
        ),
        registry_id,
        body,
    )
}

/// A long hex/key value: an abbreviated summary that expands to the full value.
/// `<details>` is native HTML, so this works with JavaScript disabled.
fn hex_value(value: &str) -> Markup {
    let short: String = value.chars().take(20).collect();
    html! {
        details."hex" {
            summary { span."hx" { (short) @if value.chars().count() > 20 { "…" } } }
            code."hex-full" { (value) }
        }
    }
}

/// Which package view a page is (plan-126-F).
///
/// An enum rather than the `audit: bool` it replaced: with three tabs a bool
/// cannot name the third, and two bools could claim two current tabs at once.
/// Each variant marks exactly one tab current by construction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackageTab {
    Overview,
    Docs,
    Audit,
}

/// The tab strip shared by the package views. Each "tab" is a separate URL, not
/// a script toggle — the site has no script.
///
/// `aria-current="page"` is emitted on the current tab **only**: maud's optional
/// attribute syntax omits the attribute entirely when the value is `None`,
/// rather than rendering an empty one.
fn package_tabs(ident: &str, current: PackageTab) -> Markup {
    let base = package_path(ident);
    let mark = |tab: PackageTab| (current == tab).then_some("page");
    html! {
        nav."tabs" aria-label="Package views" {
            a."tab" href=(base) aria-current=[mark(PackageTab::Overview)] { "Overview" }
            a."tab" href={ (base) "/docs" } aria-current=[mark(PackageTab::Docs)] { "Docs" }
            a."tab" href={ (base) "/audit" } aria-current=[mark(PackageTab::Audit)] { "Audit" }
        }
    }
}

/// What the Docs tab renders (plan-126-F).
pub struct DocsView {
    pub ident: String,
    /// The latest active release — the same plan-126-A selection that picked
    /// the documentation, so the version named is the version documented.
    /// `None` when the package has no active release.
    pub version: Option<String>,
    /// The documentation of that release, or `None` when its author included
    /// none.
    pub page: Option<mfb_wire::docpage::DocPage>,
}

/// `GET /p/:ident/docs` — the Docs tab (plan-126-F).
///
/// **Every string on this page is publisher-controlled**: package prose,
/// subtitles, signatures, examples, parameter descriptions. The defence is the
/// same as everywhere on this site — maud escapes every interpolated value, and
/// this function never reaches for maud's escaping bypass. The compiler's `src/doc/html.rs`
/// renderer is deliberately **not** reused: it emits an inline `<style>` the CSP
/// blocks, and embedding its HTML would need exactly that bypass. The shared
/// thing is the *model* (`mfb_wire::docpage`), not the markup.
///
/// A package whose release carries no documentation still gets this tab, with an
/// explicit statement: a missing tab would make "no documentation" and "the docs
/// failed to load" indistinguishable.
pub fn docs_page(registry_id: &str, view: &DocsView) -> Markup {
    let raw = format!("{}/docs", package_json_path(&view.ident));
    let body = html! {
        div."wrap" {
            div."pkg-head" {
                h1."pkg-head__ident" { (view.ident) }
                div."pkg-head__row" {
                    @if let Some(version) = &view.version {
                        span."pkg-latest" { "v" (version) }
                    }
                    span."muted" { "documentation" }
                }
                @if let Some(page) = &view.page {
                    @if !page.subtitle.is_empty() {
                        p."pkg-desc" { (doc_inline(&page.subtitle)) }
                    }
                }
            }

            (package_tabs(&view.ident, PackageTab::Docs))

            p { a."raw-link" href=(raw) { (raw) " — raw JSON" } }

            @match &view.page {
                // The sidebar layout: the index is the first grid column and
                // stays in view while the declarations scroll beside it. Below
                // the stylesheet's breakpoint (or with no CSS) the index stacks
                // above the content, exactly as before.
                Some(page) => div."doc-layout" {
                    @if !page.public.is_empty() {
                        (doc_index(&page.public))
                    }
                    div."doc-main" {
                        @if let Some(message) = &page.package_deprecated {
                            div."callout callout--warning" role="note" {
                                strong."callout__label" { "Deprecated." }
                                @if message.is_empty() {
                                    "This package is deprecated."
                                } @else {
                                    (doc_inline(message))
                                }
                            }
                        }
                        @if !page.intro.is_empty() {
                            div."doc-intro" {
                                (doc_prose(&page.intro))
                            }
                        }
                        // Only the public groups. A declaration its author marked
                        // `INTERNAL` is not part of the package's API — a consumer
                        // cannot call it — so the registry does not present it
                        // (plan-126-F Open Decision; `GET /packages/:ident/docs`
                        // omits them identically).
                        @if page.public.is_empty() {
                            p."muted" {
                                "This release documents the package itself but none of its \
                                 public declarations."
                            }
                        } @else {
                            @for (index, group) in page.public.iter().enumerate() {
                                h2."section-title doc-group" id=(doc_group_id(index)) { (group.title) }
                                @for decl in &group.decls {
                                    (doc_decl(decl))
                                }
                            }
                        }
                    }
                },
                None => {
                    div."empty" role="note" {
                        h2 { "No documentation in this release" }
                        p {
                            "The package's author did not include documentation in "
                            @match &view.version {
                                Some(version) => { "release " span."mono" { "v" (version) } },
                                None => { "any active release" },
                            }
                            "."
                        }
                        p."muted" {
                            "For the MFBASIC language itself, run "
                            span."mono" { "mfb man" }
                            "."
                        }
                    }
                },
            }
        }
    };
    page(&format!("{} — docs", view.ident), registry_id, body)
}

/// Render prose blocks: a `DESC` is a paragraph, and `WARN` / `INFO` / `SEC` are
/// callouts — the same four-way mapping the compiler's renderer uses
/// (`src/doc/html.rs`, `render_prose`), expressed with the site's semantic
/// colour tokens instead of that renderer's palette.
fn doc_prose(prose: &[mfb_wire::docpage::Prose]) -> Markup {
    use mfb_wire::docs::DocProseKind;
    html! {
        @for block in prose {
            @match block.kind {
                DocProseKind::Desc => {
                    p { (doc_inline(&block.text)) }
                },
                DocProseKind::Warn => {
                    div."callout callout--warning" role="note" {
                        strong."callout__label" { "Warning." }
                        (doc_inline(&block.text))
                    }
                },
                DocProseKind::Info => {
                    div."callout callout--info" role="note" {
                        strong."callout__label" { "Note." }
                        (doc_inline(&block.text))
                    }
                },
                DocProseKind::Sec => {
                    div."callout callout--danger" role="note" {
                        strong."callout__label" { "Security." }
                        (doc_inline(&block.text))
                    }
                },
            }
        }
    }
}

/// Inline doc text: a backtick span becomes `<code>`, and nothing else is markup
/// (plan-09-doc.md §2.3) — the rule the compiler's renderer applies
/// (`src/doc/html.rs`, `inline`), including leaving an unclosed backtick as a
/// literal character. Every piece is interpolated, so maud escapes the code span
/// and the text around it alike; splitting on a backtick never produces markup.
fn doc_inline(text: &str) -> Markup {
    let mut pieces: Vec<(bool, &str)> = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find('`') {
        let Some(close) = rest[open + 1..].find('`') else {
            break;
        };
        pieces.push((false, &rest[..open]));
        pieces.push((true, &rest[open + 1..open + 1 + close]));
        rest = &rest[open + 1 + close + 1..];
    }
    pieces.push((false, rest));
    html! {
        @for (is_code, piece) in pieces {
            @if is_code {
                code { (piece) }
            } @else {
                (piece)
            }
        }
    }
}

/// The HTML element id for a declaration: the model's anchor under a `doc-`
/// prefix. The model's anchors are unique among declarations and avoid only the
/// compiler page's own ids; this page's shell carries others (`q` on the search
/// box in every page header), so a declaration named `q` would otherwise
/// duplicate an id and its index link would jump to the search box. No shell id
/// starts `doc-`. `GET /packages/:ident/docs` reports this same id.
pub fn doc_element_id(anchor: &str) -> String {
    format!("doc-{anchor}")
}

/// The HTML element id for the heading of the `index`th public group. Group
/// titles are publisher text and not unique, so the id is positional; it
/// contains `_`, which [`doc_element_id`] never produces, so it cannot collide
/// with a declaration.
fn doc_group_id(index: usize) -> String {
    format!("doc_group_{index}")
}

/// A no-script index of the declaration groups, folded with the checkbox + label
/// pattern the Overview's target rows use. Rendered **checked** (open), so the
/// index is visible by default and without CSS. On a wide screen the stylesheet
/// pins it as a sidebar beside the declarations. Each group title links to its
/// heading. The checkbox id contains `_`, which [`doc_element_id`] never
/// produces, so it cannot collide with a declaration.
fn doc_index(groups: &[mfb_wire::docpage::DocGroup]) -> Markup {
    let count: usize = groups.iter().map(|group| group.decls.len()).sum();
    html! {
        nav."doc-index" aria-label="Declarations" {
            input."fold-cb" type="checkbox" id="doc_index_fold" checked;
            label."tgt-toggle doc-index__toggle" for="doc_index_fold"
                title="Show/hide the declaration index" {
                span."chev" {}
                "Contents"
                span."tgt-count" { (count) }
            }
            div."doc-index__groups" {
                @for (index, group) in groups.iter().enumerate() {
                    div."doc-index__group" {
                        p."eyebrow" {
                            a href={ "#" (doc_group_id(index)) } { (group.title) }
                        }
                        ul {
                            @for decl in &group.decls {
                                li {
                                    a."mono" href={ "#" (doc_element_id(&decl.anchor)) } {
                                        (decl.name)
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// One declaration: the same parts, in the same order, as the compiler's
/// `render_decl` (`src/doc/html.rs`) — heading and kind badge, signature,
/// deprecation, description, parameters, members, returns, errors, example.
/// The signature and example look like code and are the fields most tempting to
/// emit raw; they are interpolated like everything else.
fn doc_decl(decl: &mfb_wire::docpage::DocDecl) -> Markup {
    let id = doc_element_id(&decl.anchor);
    html! {
        section."decl" id=(id) {
            div."decl__head" {
                h3."decl__name" {
                    a."mono" href={ "#" (id) } { (decl.name) }
                }
                span class={ "doc-badge doc-badge--" (decl.badge_class) } { (decl.kind_label) }
            }
            @if !decl.signature.is_empty() {
                pre."decl__sig" { code { (decl.signature) } }
            }
            @if let Some(message) = &decl.deprecated {
                div."callout callout--warning" role="note" {
                    strong."callout__label" { "Deprecated." }
                    @if message.is_empty() {
                        "This declaration is deprecated."
                    } @else {
                        (doc_inline(message))
                    }
                }
            }
            (doc_prose(&decl.desc))
            (doc_table("Parameters", "name", &decl.args))
            @if let Some(label) = decl.member_label {
                (doc_table(label, "name", &decl.props))
            }
            @if !decl.ret.is_empty() {
                h4."decl__h" { "Returns" }
                p { (doc_inline(&decl.ret)) }
            }
            (doc_table("Errors", "code", &decl.errors))
            @if !decl.example.is_empty() {
                div."decl__example" {
                    p."eyebrow" { "Example" }
                    pre { code { (decl.example) } }
                }
            }
        }
    }
}

/// A two-column name/description table; renders nothing for no rows.
fn doc_table(heading: &str, name_column: &str, rows: &[(String, String)]) -> Markup {
    html! {
        @if !rows.is_empty() {
            h4."decl__h" { (heading) }
            table."grid doc-table" {
                thead {
                    tr { th { (name_column) } th { "description" } }
                }
                tbody {
                    @for (name, desc) in rows {
                        tr {
                            td."mono" data-label=(name_column) { (name) }
                            td data-label="description" { (doc_inline(desc)) }
                        }
                    }
                }
            }
        }
    }
}

/// One rendered native target row.
pub struct TargetRow {
    pub os: String,
    pub arch: Option<String>,
    pub libc: Option<String>,
    pub lib_type: String,
    pub logical: String,
    pub source: String,
    pub blob_hash: Option<String>,
}

/// One rendered version row.
pub struct VersionRow {
    pub version: String,
    pub hash: String,
    pub published_at: i64,
    pub state: String,
    pub abi_symbols: usize,
    pub log_index: Option<i64>,
    pub targets: Vec<TargetRow>,
}

/// Everything `GET /p/:ident` renders.
pub struct PackageView {
    pub ident: String,
    pub owner: String,
    pub ident_key: String,
    pub ident_fingerprint: String,
    pub server_fingerprint: String,
    pub author: Option<String>,
    pub url: Option<String>,
    pub description: Option<String>,
    /// The newest **active** release (plan-126-A), not simply the newest row.
    /// `None` means every published version is yanked, blocked or
    /// legal-tombstoned — rendered as an explicit statement, not an omission.
    pub latest_version: Option<String>,
    /// The release state of `latest_version`, badged beside it so a
    /// `deprecated` headline does not read as current. `None` exactly when
    /// `latest_version` is.
    pub latest_state: Option<String>,
    pub versions: Vec<VersionRow>,
}

/// `GET /p/:ident` — the package page (plan-61-C Phase 3).
///
/// Renders the most publisher-controlled data on the site. Every interpolated
/// value goes through maud's auto-escaping, and `url` additionally through
/// [`external_link`]'s scheme allowlist.
///
/// **Every version is listed, with its state visible.** A yanked version is
/// distinguished visually but never hidden: a version that vanished from this
/// list would itself be the evidence of tampering this page exists to expose.
pub fn package_page(registry_id: &str, view: &PackageView) -> Markup {
    let body = html! {
        div."wrap" {
            div."pkg-head" {
                h1."pkg-head__ident" { (view.ident) }
                div."pkg-head__row" {
                    // plan-126-A: the headline version is the newest *active*
                    // release, and its state is badged beside it — a
                    // `deprecated` headline must not read as current. When
                    // there is no active release the absence is *stated*: an
                    // omitted chip and a package with no releases at all would
                    // otherwise render identically, and the reader could not
                    // tell "nothing published" from "everything withdrawn".
                    @match &view.latest_version {
                        Some(latest) => {
                            span."pkg-latest" { "latest v" (latest) }
                            @if let Some(release_state) = &view.latest_state {
                                span class={ "state state--" (state_modifier(release_state)) } {
                                    (release_state)
                                }
                            }
                        }
                        None => {
                            @if !view.versions.is_empty() {
                                span."pkg-latest pkg-latest--none" {
                                    "no active release"
                                }
                                span."muted" {
                                    "every published version is yanked or blocked"
                                }
                            }
                        }
                    }
                    span."muted" { "owner " span."mono" { (view.owner) } }
                }
                p."pkg-desc" {
                    @match &view.description {
                        Some(text) => (text),
                        None => span."nil" { "No description provided." },
                    }
                }

                dl."dl" {
                    dt { "author" }
                    dd."mono" {
                        @match &view.author {
                            Some(author) => (author),
                            None => span."nil" { "—" },
                        }
                    }

                    dt { "url" }
                    dd {
                        @match &view.url {
                            Some(url) => {
                                (external_link(url))
                                @if safe_href(url).is_none() {
                                    span."url-note" {
                                        "link withheld — scheme not in the http/https allowlist"
                                    }
                                }
                            }
                            None => span."nil" { "—" },
                        }
                    }

                    dt { "ident key" }
                    dd { (hex_value(&view.ident_key)) }
                    dt { "ident fp" }
                    dd { (hex_value(&view.ident_fingerprint)) }
                    dt { "server fp" }
                    dd { (hex_value(&view.server_fingerprint)) }
                }
            }

            (package_tabs(&view.ident, PackageTab::Overview))

            h2."section-title" {
                "Versions " span."muted" { "(" (view.versions.len()) ")" }
            }
            p."table-caption" {
                "Every published version is listed, including deprecated and yanked \
                 releases. Nothing is hidden or collapsed — a version that has \
                 disappeared from this list would itself be evidence of tampering."
            }

            table."versions" {
                thead {
                    tr {
                        th { "version" } th { "state" } th { "published" }
                        th { "ABI" } th { "hash" } th { "log" }
                    }
                }
                @for (index, version) in view.versions.iter().enumerate() {
                    // The latest release's targets are the ones a reader is
                    // almost always after, so they stay expanded. Every older
                    // release folds behind a toggle — the rows are still in the
                    // document (and still in view-source), just collapsed, so
                    // nothing is hidden from an auditor.
                    @let folded = view.latest_version.as_deref() != Some(version.version.as_str());
                    @let fold_id = format!("tgt-{index}");
                    tbody."v" {
                        tr."v-main" {
                            td."v-ver" data-label="version" {
                                (version.version)
                                @if folded && !version.targets.is_empty() {
                                    input type="checkbox" id=(fold_id) ."fold-cb";
                                    label."tgt-toggle" for=(fold_id)
                                        title="Show/hide native targets" {
                                        span."chev" {}
                                        span."tgt-count" { (version.targets.len()) }
                                    }
                                }
                            }
                            td data-label="state" {
                                span class={ "state state--" (state_modifier(&version.state)) } {
                                    (version.state)
                                }
                            }
                            td."v-date" data-label="published" {
                                (format_date(version.published_at))
                            }
                            td."v-abi num" data-label="ABI index" { (version.abi_symbols) }
                            td."v-hash" data-label="hash" { (hex_value(&version.hash)) }
                            td."v-log" data-label="log entry" {
                                @match version.log_index {
                                    Some(index) => a href={ (package_path(&view.ident)) "/audit" } {
                                        "#" (index)
                                    },
                                    None => span."nil" { "—" },
                                }
                            }
                        }
                        @if !version.targets.is_empty() {
                            tr."v-targets"."v-fold"[folded] {
                                td colspan="6" {
                                    div."targets-wrap" {
                                        p."targets-cap" {
                                            "Native targets — " (version.targets.len())
                                            " for v" (version.version)
                                        }
                                        table."targets" {
                                            thead {
                                                tr {
                                                    th { "os" } th { "arch" } th { "libc" }
                                                    th { "type" } th { "logical" }
                                                    th { "source" } th { "blobHash" }
                                                }
                                            }
                                            tbody {
                                                @for target in &version.targets {
                                                    tr {
                                                        td data-label="os" { (target.os) }
                                                        // NULL arch is the
                                                        // any-arch wildcard, and
                                                        // reads as "any" — never
                                                        // as a blank cell, which
                                                        // would look like
                                                        // missing data.
                                                        td data-label="arch" {
                                                            @match &target.arch {
                                                                Some(arch) => (arch),
                                                                None => span."nil" { "any" },
                                                            }
                                                        }
                                                        td data-label="libc" {
                                                            @match &target.libc {
                                                                Some(libc) => (libc),
                                                                None => span."nil" { "—" },
                                                            }
                                                        }
                                                        td data-label="type" {
                                                            span."libtype" { (target.lib_type) }
                                                        }
                                                        td data-label="logical" { (target.logical) }
                                                        td data-label="source" { (target.source) }
                                                        td data-label="blobHash" {
                                                            @match &target.blob_hash {
                                                                Some(hash) => (hex_value(hash)),
                                                                None => span."nil" { "—" },
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    };
    page(&format!("{} — mfb-repo", view.ident), registry_id, body)
}

/// The CSS modifier for a release state. Unknown states fall back to a neutral
/// modifier rather than being interpolated into the class list unchecked.
fn state_modifier(state: &str) -> &'static str {
    match state {
        "available" => "available",
        "deprecated" => "deprecated",
        "yanked" => "yanked",
        "blocked" => "blocked",
        _ => "other",
    }
}

/// One rendered publish log entry.
pub struct AuditPublish {
    pub version: String,
    pub index: i64,
    pub leaf_hash: String,
    pub proof: Vec<String>,
}

/// Everything `GET /p/:ident/audit` renders.
pub struct AuditView {
    pub ident: String,
    pub checkpoint_size: i64,
    pub checkpoint_root: String,
    pub checkpoint_signature: String,
    pub publishes: Vec<AuditPublish>,
    pub state_changes: Vec<(String, String, i64)>,
    pub ident_chain: Vec<(String, String, String, i64)>,
}

/// `GET /p/:ident/audit` — the transparency tab (plan-61-C Phase 3, §4).
///
/// Renders the inclusion **proof path** inline and links the raw JSON endpoint,
/// so a third-party monitor can script against it. The copy is deliberate: this
/// is evidence for the reader to check, never an assurance from the registry
/// that anything is correct.
pub fn audit_page(registry_id: &str, view: &AuditView) -> Markup {
    let raw = format!("{}/audit", package_json_path(&view.ident));
    let body = html! {
        div."wrap" {
            div."pkg-head" {
                h1."pkg-head__ident" { (view.ident) }
                div."pkg-head__row" {
                    span."muted" { "transparency log & identity history" }
                }
            }

            (package_tabs(&view.ident, PackageTab::Audit))

            div."prose" {
                p {
                    "This is an append-only transparency log for "
                    span."mono" { (view.ident) }
                    ". It records an inclusion proof for every publish, every \
                     release-state transition, and every rotation of the registry’s \
                     identity key. It is evidence for you to check — not an assurance \
                     that anything is correct. To detect a registry that shows \
                     different histories to different people, fetch this log from more \
                     than one vantage point and compare the checkpoint root hashes, or \
                     script against the raw endpoint:"
                }
                p { a."raw-link" href=(raw) { (raw) " — raw JSON" } }
            }

            h2."section-title" { "Log checkpoint" }
            p."table-caption" {
                "A signed statement of the log’s size and Merkle root at a point in \
                 time. Two observers who see the same root at the same size are being \
                 shown the same log."
            }
            div."checkpoint" {
                dl."dl" {
                    dt { "log size" }
                    dd."mono num" { (view.checkpoint_size) " entries" }
                    dt { "root hash" }
                    dd { (hex_value(&view.checkpoint_root)) }
                    dt { "signature" }
                    dd { (hex_value(&view.checkpoint_signature)) }
                }
            }

            h2."section-title" { "Publishes" }
            p."table-caption" {
                "Each publish is a leaf in the log. The leaf hash, the sibling path \
                 below it, and the checkpoint above together form an inclusion proof \
                 that this exact release was recorded."
            }
            table."grid" {
                thead {
                    tr {
                        th { "version" } th { "leaf index" } th { "leaf hash" }
                        th { "inclusion proof" }
                    }
                }
                tbody {
                    @for entry in &view.publishes {
                        tr id={ "leaf-" (entry.index) } {
                            td."mono" data-label="version" { (entry.version) }
                            td."mono num" data-label="leaf index" { (entry.index) }
                            td data-label="leaf hash" { (hex_value(&entry.leaf_hash)) }
                            td data-label="inclusion proof" {
                                @if entry.proof.is_empty() {
                                    span."nil" { "— (sole leaf)" }
                                } @else {
                                    ol."proof-path" {
                                        @for hop in &entry.proof {
                                            li { (hex_value(hop)) }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            h2."section-title" { "State changes" }
            p."table-caption" {
                "Every release-state transition this package has undergone. A yank is \
                 recorded here permanently; it does not remove the version."
            }
            @if view.state_changes.is_empty() {
                p."muted" { "No state changes recorded." }
            } @else {
                table."grid" {
                    thead { tr { th { "version" } th { "state" } th { "at" } } }
                    tbody {
                        @for (version, state, at) in &view.state_changes {
                            tr {
                                td."mono" data-label="version" { (version) }
                                td data-label="state" {
                                    span class={ "state state--" (state_modifier(state)) } {
                                        (state)
                                    }
                                }
                                td."mono" data-label="at" { (format_date(*at)) }
                            }
                        }
                    }
                }
            }

            h2."section-title" { "Identity key rotations" }
            p."table-caption" {
                "Each rotation is signed by the key being replaced, so the chain can be \
                 followed from any earlier pin. An unexplained re-anchor with no chain \
                 link is what a compromised or seized registry looks like."
            }
            @if view.ident_chain.is_empty() {
                p."muted" { "No rotations recorded — the owner still holds its original ident key." }
            } @else {
                table."grid" {
                    thead {
                        tr { th { "old key" } th { "new key" } th { "signature" } th { "at" } }
                    }
                    tbody {
                        @for (old_key, new_key, signature, issued) in &view.ident_chain {
                            tr {
                                td data-label="old key" { (hex_value(old_key)) }
                                td data-label="new key" { (hex_value(new_key)) }
                                td data-label="signature" { (hex_value(signature)) }
                                td."mono" data-label="at" { (format_date(*issued)) }
                            }
                        }
                    }
                }
            }
        }
    };
    page(
        &format!("{} — audit — mfb-repo", view.ident),
        registry_id,
        body,
    )
}

/// The JSON API path for a package, for the audit tab's raw-JSON link.
fn package_json_path(ident: &str) -> String {
    format!(
        "/packages/{}",
        ident.replace('%', "%25").replace('#', "%23")
    )
}

/// A minimal page for an error or notice, so every non-200 HTML response is
/// still a real page carrying the CSP rather than a bare status.
pub fn message_page(registry_id: &str, heading: &str, detail: &str) -> Markup {
    let body = html! {
        div."wrap" {
            div."empty empty--center" role="note" {
                h2 { (heading) }
                p { (detail) }
                p { a href="/" { "Back to the registry" } }
            }
        }
    };
    page(heading, registry_id, body)
}

/// The site path for a package page, percent-encoding the `#` so it does not
/// become a URL fragment. Only `#` and `%` need encoding — idents are already
/// restricted to a safe charset by `validate_ident`.
pub fn package_path(ident: &str) -> String {
    format!("/p/{}", ident.replace('%', "%25").replace('#', "%23"))
}

/// A Unix timestamp as `YYYY-MM-DD`, UTC.
///
/// Hand-rolled from the civil-from-days algorithm rather than pulling in a date
/// crate for one format: the registry stores seconds and the pages show days.
pub fn format_date(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    // Howard Hinnant's civil_from_days, shifted to a March-based year so the
    // leap day lands at the end.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// plan-61-C Phase 1 — the point of the phase.
    ///
    /// Each rejected case defeats a *different* naive implementation:
    /// `starts_with("javascript:")` misses the case-varied and whitespace-padded
    /// forms; `contains("javascript")` misses `data:` and `vbscript:`; a check
    /// that treats "no scheme" as safe misses protocol-relative `//evil`.
    #[test]
    fn safe_href_rejects_every_non_http_scheme() {
        for hostile in [
            "javascript:alert(1)",
            "JaVaScRiPt:alert(1)",
            "JAVASCRIPT:alert(1)",
            "java\tscript:alert(1)",
            "java\nscript:alert(1)",
            "java\r\nscript:alert(1)",
            " javascript:alert(1)",
            "\u{1}javascript:alert(1)",
            "\u{0}javascript:alert(1)",
            "data:text/html,<script>alert(1)</script>",
            "DATA:text/html;base64,PHNjcmlwdD4=",
            "vbscript:msgbox(1)",
            "file:///etc/passwd",
            "//evil.example",
            "//evil.example/path",
            "\\\\evil.example",
            "about:blank",
            "blob:https://x.example/uuid",
            "ftp://x.example",
            "mailto:someone@example.invalid",
            "http:evil",   // scheme with no authority
            "https:/evil", // one slash is not an authority
            "",
            ":",
            "://evil.example",
            "relative/path",
            "#fragment",
        ] {
            assert_eq!(
                safe_href(hostile),
                None,
                "{hostile:?} must not become an href",
            );
        }
    }

    #[test]
    fn safe_href_accepts_ordinary_http_and_https_urls() {
        for benign in [
            "http://x.example",
            "https://x.example",
            "https://x.example/a?b=c#d",
            "HTTPS://X.EXAMPLE/a",
            "https://x.example:8443/path",
            "https://user@x.example/path",
        ] {
            assert_eq!(
                safe_href(benign).as_deref(),
                Some(benign),
                "{benign:?} must be linkable, and returned unrewritten",
            );
        }
    }

    /// A rejected URL still has to be *visible* — the reader should see what
    /// the publisher claimed, just not be able to click it. Silently dropping
    /// it would hide the hostile value from the person best placed to notice.
    #[test]
    fn a_rejected_url_renders_as_inert_escaped_text_not_a_link() {
        let rendered = external_link("javascript:alert(1)").into_string();
        assert!(!rendered.contains("<a "), "{rendered}");
        assert!(!rendered.contains("href"), "{rendered}");
        assert!(rendered.contains("javascript:alert(1)"), "{rendered}");

        // And an accepted one is a link with the anti-tabnabbing rel.
        let rendered = external_link("https://x.example/a").into_string();
        assert!(
            rendered.contains("href=\"https://x.example/a\""),
            "{rendered}"
        );
        assert!(
            rendered.contains("noopener noreferrer nofollow"),
            "{rendered}"
        );
    }

    /// plan-61-C — the mockup folds the native-target block for every release
    /// except the latest. Folding is CSS-only (checkbox + `:has`), so the rows
    /// stay in the document: collapsed, never omitted.
    #[test]
    fn only_the_latest_release_shows_its_native_targets_expanded() {
        fn version(v: &str) -> VersionRow {
            VersionRow {
                version: v.to_string(),
                hash: "sha256:00".to_string(),
                published_at: 0,
                state: "available".to_string(),
                abi_symbols: 1,
                log_index: Some(1),
                targets: vec![TargetRow {
                    os: "linux".to_string(),
                    arch: Some("x86_64".to_string()),
                    libc: Some("glibc".to_string()),
                    lib_type: "system".to_string(),
                    logical: format!("m-{v}.so"),
                    source: "src:m".to_string(),
                    blob_hash: None,
                }],
            }
        }

        let view = PackageView {
            ident: "acme.matrix".to_string(),
            owner: "acme".to_string(),
            ident_key: "00".to_string(),
            ident_fingerprint: "00".to_string(),
            server_fingerprint: "00".to_string(),
            author: None,
            url: None,
            description: None,
            latest_version: Some("4.2.0".to_string()),
            latest_state: Some("available".to_string()),
            versions: vec![version("4.2.0"), version("3.6.0")],
        };
        let rendered = package_page("reg", &view).into_string();

        // Exactly one fold toggle — for 3.6.0, not for the latest 4.2.0.
        assert_eq!(rendered.matches("fold-cb").count(), 1, "{rendered}");
        assert_eq!(rendered.matches("tgt-toggle").count(), 1, "{rendered}");
        assert_eq!(
            rendered.matches("v-targets v-fold").count(),
            1,
            "{rendered}"
        );
        assert!(rendered.contains(r#"class="v-targets""#), "{rendered}");

        // The label's `for` must resolve to the checkbox, or the toggle is dead.
        assert!(rendered.contains(r#"id="tgt-1""#), "{rendered}");
        assert!(rendered.contains(r#"for="tgt-1""#), "{rendered}");

        // Both versions' target rows are still present in the markup.
        assert!(rendered.contains("m-4.2.0.so"), "{rendered}");
        assert!(rendered.contains("m-3.6.0.so"), "{rendered}");
    }

    // === plan-126-A: the Overview header's headline release =================

    /// A `PackageView` whose newest *listed* version is yanked. The header must
    /// name the older active release, **and** the versions table must still
    /// contain the yanked one — the second half is what proves the selection
    /// filter did not leak into the transparency listing.
    #[test]
    fn the_header_names_the_newest_active_release_while_the_table_keeps_the_yanked_one() {
        let view = PackageView {
            latest_version: Some("1.5.0".to_string()),
            latest_state: Some("available".to_string()),
            versions: vec![
                header_test_version("2.0.0", "yanked"),
                header_test_version("1.5.0", "available"),
            ],
            ..header_test_view()
        };
        let rendered = package_page("reg", &view).into_string();

        assert!(rendered.contains("latest v1.5.0"), "{rendered}");
        assert!(!rendered.contains("latest v2.0.0"), "{rendered}");
        // The yanked release is still listed, with its state visible.
        assert!(rendered.contains("state--yanked"), "{rendered}");
        assert_eq!(rendered.matches("2.0.0").count() >= 1, true, "{rendered}");
    }

    /// A `deprecated` headline is badged, so it cannot read as current. This is
    /// the case the bare version chip could never express.
    #[test]
    fn a_deprecated_headline_release_carries_its_state_badge() {
        let view = PackageView {
            latest_version: Some("2.0.0".to_string()),
            latest_state: Some("deprecated".to_string()),
            versions: vec![header_test_version("2.0.0", "deprecated")],
            ..header_test_view()
        };
        let rendered = package_page("reg", &view).into_string();
        assert!(rendered.contains("latest v2.0.0"), "{rendered}");
        assert!(rendered.contains("state--deprecated"), "{rendered}");
    }

    /// Versions exist but none is active. The header states that, rather than
    /// silently omitting the chip — otherwise "everything withdrawn" and
    /// "nothing published" render identically.
    #[test]
    fn no_active_release_is_stated_not_omitted() {
        let view = PackageView {
            latest_version: None,
            latest_state: None,
            versions: vec![
                header_test_version("2.0.0", "blocked"),
                header_test_version("1.5.0", "yanked"),
            ],
            ..header_test_view()
        };
        let rendered = package_page("reg", &view).into_string();

        assert!(rendered.contains("no active release"), "{rendered}");
        assert!(
            rendered.contains("every published version is yanked or blocked"),
            "{rendered}"
        );
        assert!(!rendered.contains("latest v"), "{rendered}");
        // Both versions are still in the table.
        assert!(rendered.contains("state--blocked"), "{rendered}");
        assert!(rendered.contains("state--yanked"), "{rendered}");
    }

    /// A package identity with no published version at all takes neither
    /// branch: there is nothing to state the absence *of*.
    #[test]
    fn a_package_with_no_versions_renders_no_release_copy_at_all() {
        let view = PackageView {
            latest_version: None,
            latest_state: None,
            versions: Vec::new(),
            ..header_test_view()
        };
        let rendered = package_page("reg", &view).into_string();
        assert!(!rendered.contains("latest v"), "{rendered}");
        assert!(!rendered.contains("no active release"), "{rendered}");
    }

    fn header_test_view() -> PackageView {
        PackageView {
            ident: "acme#matrix".to_string(),
            owner: "acme".to_string(),
            ident_key: "00".to_string(),
            ident_fingerprint: "00".to_string(),
            server_fingerprint: "00".to_string(),
            author: None,
            url: None,
            description: None,
            latest_version: None,
            latest_state: None,
            versions: Vec::new(),
        }
    }

    fn header_test_version(version: &str, state: &str) -> VersionRow {
        VersionRow {
            version: version.to_string(),
            hash: "sha256:00".to_string(),
            published_at: 0,
            state: state.to_string(),
            abi_symbols: 1,
            log_index: Some(1),
            targets: Vec::new(),
        }
    }

    // === plan-126-F: tabs and the Docs tab ================================

    /// Every tab variant marks exactly one tab current, and it is the right one.
    #[test]
    fn package_tabs_mark_exactly_the_current_tab() {
        for (current, label) in [
            (PackageTab::Overview, "Overview"),
            (PackageTab::Docs, "Docs"),
            (PackageTab::Audit, "Audit"),
        ] {
            let rendered = package_tabs("acme#matrix", current).into_string();
            assert_eq!(
                rendered.matches("aria-current=\"page\"").count(),
                1,
                "{current:?}: {rendered}"
            );
            assert!(
                rendered.contains(&format!("aria-current=\"page\">{label}</a>")),
                "{current:?} must mark {label}: {rendered}"
            );
        }
    }

    fn docs_view(page: Option<mfb_wire::docpage::DocPage>) -> DocsView {
        DocsView {
            ident: "acme#matrix".to_string(),
            version: Some("4.2.0".to_string()),
            page,
        }
    }

    fn doc_page_with_intro(
        desc: Vec<(u8, String)>,
        deprecated: Option<String>,
    ) -> mfb_wire::docpage::DocPage {
        mfb_wire::docpage::from_package(
            mfb_wire::docs::PackageDocs {
                package: Some(mfb_wire::docs::PackageDocEntry {
                    name: "matrix".to_string(),
                    desc,
                    deprecated,
                }),
                decls: Vec::new(),
            },
            "matrix",
        )
    }

    /// No documentation is stated explicitly, names the release, points at
    /// `mfb man`, and renders no documentation markup.
    #[test]
    fn the_empty_docs_page_states_the_author_included_no_documentation() {
        let rendered = docs_page("reg", &docs_view(None)).into_string();
        assert!(
            rendered.contains("did not include documentation"),
            "{rendered}"
        );
        assert!(rendered.contains("v4.2.0"), "{rendered}");
        assert!(rendered.contains("mfb man"), "{rendered}");
        assert!(!rendered.contains("callout"), "{rendered}");
        assert!(!rendered.contains("doc-intro"), "{rendered}");
        assert_eq!(rendered.matches("aria-current=\"page\"").count(), 1);
        assert!(
            rendered.contains("aria-current=\"page\">Docs</a>"),
            "{rendered}"
        );
    }

    /// The first `DESC` is the subtitle, the rest is intro, and each of the four
    /// prose kinds renders as its own element and callout class.
    #[test]
    fn the_docs_page_renders_package_prose_and_every_callout_kind() {
        let page = doc_page_with_intro(
            vec![
                (0, "The subtitle.".to_string()),
                (0, "A paragraph.".to_string()),
                (1, "A warning.".to_string()),
                (2, "A note.".to_string()),
                (3, "A security caveat.".to_string()),
            ],
            Some("use matrix2".to_string()),
        );
        let rendered = docs_page("reg", &docs_view(Some(page))).into_string();
        assert!(rendered.contains("The subtitle."), "{rendered}");
        assert!(rendered.contains("<p>A paragraph.</p>"), "{rendered}");
        assert!(rendered.contains("callout--warning"), "{rendered}");
        assert!(rendered.contains("callout--info"), "{rendered}");
        assert!(rendered.contains("callout--danger"), "{rendered}");
        assert!(rendered.contains("Deprecated."), "{rendered}");
        assert!(rendered.contains("use matrix2"), "{rendered}");
        assert!(
            !rendered.contains("did not include documentation"),
            "{rendered}"
        );
    }

    /// Publisher prose is escaped, never rendered as markup. The full escaping
    /// proof across signatures, examples and parameters is plan-126-F Phase 3.
    #[test]
    fn docs_page_escapes_publisher_prose() {
        let hostile = "<script>alert(1)</script>";
        let page = doc_page_with_intro(
            vec![(0, hostile.to_string()), (2, hostile.to_string())],
            Some(hostile.to_string()),
        );
        let rendered = docs_page("reg", &docs_view(Some(page))).into_string();
        assert!(!rendered.contains("<script"), "{rendered}");
        assert!(rendered.contains("&lt;script&gt;"), "{rendered}");
    }

    fn doc_decl_entry(kind: &str, name: &str, group: &str) -> mfb_wire::docs::DeclDocEntry {
        mfb_wire::docs::DeclDocEntry {
            kind: kind.to_string(),
            name: name.to_string(),
            signature: String::new(),
            group: group.to_string(),
            desc: Vec::new(),
            args: Vec::new(),
            props: Vec::new(),
            ret: String::new(),
            errors: Vec::new(),
            example: String::new(),
            internal: false,
            deprecated: None,
        }
    }

    fn doc_page_with_decls(
        desc: Vec<(u8, String)>,
        decls: Vec<mfb_wire::docs::DeclDocEntry>,
    ) -> mfb_wire::docpage::DocPage {
        mfb_wire::docpage::from_package(
            mfb_wire::docs::PackageDocs {
                package: Some(mfb_wire::docs::PackageDocEntry {
                    name: "matrix".to_string(),
                    desc,
                    deprecated: None,
                }),
                decls,
            },
            "matrix",
        )
    }

    /// **plan-126-F Phase 3.** Every part of a declaration renders: group
    /// heading, index link, anchored section, kind badge, signature, deprecation,
    /// description, parameters, members under the kind's label, returns, errors
    /// and example.
    #[test]
    fn the_docs_page_renders_every_part_of_a_declaration() {
        let mut add = doc_decl_entry("func", "addUp", "Arithmetic");
        add.signature = "EXPORT FUNC addUp(a AS Integer, b AS Integer) AS Integer".to_string();
        add.desc = vec![(0, "Adds two integers.".to_string())];
        add.args = vec![
            ("a".to_string(), "The first addend.".to_string()),
            ("b".to_string(), "The second addend.".to_string()),
        ];
        add.ret = "The sum.".to_string();
        add.errors = vec![("ErrOverflow".to_string(), "The sum overflowed.".to_string())];
        add.example = "PRINT matrix::addUp(1, 2)".to_string();
        add.deprecated = Some("use addAll".to_string());
        let mut point = doc_decl_entry("type", "Point", "");
        point.props = vec![("x".to_string(), "Horizontal.".to_string())];

        let page = doc_page_with_decls(vec![(0, "Sub.".to_string())], vec![add, point]);
        let rendered = docs_page("reg", &docs_view(Some(page))).into_string();

        // Groups in first-appearance order, each with its heading.
        let arithmetic = rendered.find(">Arithmetic</h2>").expect(&rendered);
        let types = rendered.find(">Types</h2>").expect(&rendered);
        assert!(arithmetic < types, "{rendered}");
        // Anchored sections, linked from the index and from their own heading.
        assert!(
            rendered.contains(r#"<section class="decl" id="doc-addup">"#),
            "{rendered}"
        );
        assert!(
            rendered.contains(r#"<section class="decl" id="doc-point">"#),
            "{rendered}"
        );
        assert_eq!(
            rendered.matches(r##"href="#doc-addup""##).count(),
            2,
            "{rendered}"
        );
        assert!(rendered.contains("doc-badge--function"), "{rendered}");
        assert!(rendered.contains(">Function</span>"), "{rendered}");
        assert!(rendered.contains(">Type</span>"), "{rendered}");
        assert!(
            rendered.contains("<pre class=\"decl__sig\"><code>EXPORT FUNC addUp(a AS Integer, b AS Integer) AS Integer</code></pre>"),
            "{rendered}"
        );
        assert!(rendered.contains("use addAll"), "{rendered}");
        assert!(rendered.contains("<p>Adds two integers.</p>"), "{rendered}");
        assert!(rendered.contains(">Parameters</h4>"), "{rendered}");
        assert!(rendered.contains("The second addend."), "{rendered}");
        assert!(rendered.contains(">Fields</h4>"), "{rendered}");
        assert!(rendered.contains("Horizontal."), "{rendered}");
        assert!(
            rendered.contains(">Returns</h4><p>The sum.</p>"),
            "{rendered}"
        );
        assert!(rendered.contains(">Errors</h4>"), "{rendered}");
        assert!(rendered.contains("ErrOverflow"), "{rendered}");
        assert!(
            rendered.contains("<pre><code>PRINT matrix::addUp(1, 2)</code></pre>"),
            "{rendered}"
        );
        // The index is open by default, so it works without CSS.
        assert!(
            rendered.contains(r#"id="doc_index_fold" checked"#),
            "{rendered}"
        );
        assert!(
            rendered.contains("<span class=\"tgt-count\">2</span>"),
            "{rendered}"
        );
        assert!(!rendered.contains("<script"), "{rendered}");
        assert!(!rendered.contains("style="), "{rendered}");
    }

    /// The index renders before the content inside the sidebar layout, and each
    /// group title in it links to that group's heading by a positional id that
    /// occurs exactly once.
    #[test]
    fn the_sidebar_index_links_every_group_heading() {
        let add = doc_decl_entry("func", "addUp", "Arithmetic");
        let point = doc_decl_entry("type", "Point", "");
        let page = doc_page_with_decls(vec![(0, "Sub.".to_string())], vec![add, point]);
        let rendered = docs_page("reg", &docs_view(Some(page))).into_string();

        let layout = rendered
            .find(r#"<div class="doc-layout">"#)
            .expect(&rendered);
        let index = rendered.find(r#"<nav class="doc-index""#).expect(&rendered);
        let main = rendered.find(r#"<div class="doc-main">"#).expect(&rendered);
        assert!(layout < index && index < main, "{rendered}");
        for (id, title) in [("doc_group_0", "Arithmetic"), ("doc_group_1", "Types")] {
            assert!(
                rendered.contains(&format!(r##"<a href="#{id}">{title}</a>"##)),
                "{rendered}"
            );
            assert!(
                rendered.contains(&format!(
                    r#"<h2 class="section-title doc-group" id="{id}">{title}</h2>"#
                )),
                "{rendered}"
            );
            assert_eq!(rendered.matches(&format!(r#"id="{id}""#)).count(), 1);
        }
    }

    /// A declaration its author marked `INTERNAL` is not rendered anywhere on the
    /// page — not in the index and not as a section.
    #[test]
    fn internal_declarations_are_not_rendered() {
        let public = doc_decl_entry("func", "visibleOne", "");
        let mut hidden = doc_decl_entry("func", "secretHelper", "");
        hidden.internal = true;
        let page = doc_page_with_decls(Vec::new(), vec![public, hidden]);
        assert_eq!(
            page.internal.len(),
            1,
            "the fixture must carry an internal decl"
        );
        let rendered = docs_page("reg", &docs_view(Some(page))).into_string();
        assert!(rendered.contains("visibleOne"), "{rendered}");
        assert!(!rendered.contains("secretHelper"), "{rendered}");
        assert!(!rendered.contains("secrethelper"), "{rendered}");
    }

    /// A package documented with no public declarations says so rather than
    /// rendering an empty index.
    #[test]
    fn a_page_with_no_public_declarations_says_so() {
        let page = doc_page_with_decls(vec![(0, "Sub.".to_string())], Vec::new());
        let rendered = docs_page("reg", &docs_view(Some(page))).into_string();
        assert!(
            rendered.contains("none of its public declarations"),
            "{rendered}"
        );
        assert!(!rendered.contains("doc-index"), "{rendered}");
    }

    /// Backtick spans become `<code>`, escaped like the text around them; an
    /// unclosed backtick stays a literal character — the compiler renderer's rule.
    #[test]
    fn doc_inline_renders_backtick_spans_as_escaped_code() {
        assert_eq!(
            doc_inline("call `f(<a>)` now").into_string(),
            "call <code>f(&lt;a&gt;)</code> now"
        );
        assert_eq!(
            doc_inline("a `b` c `d`").into_string(),
            "a <code>b</code> c <code>d</code>"
        );
        assert_eq!(doc_inline("tick ` alone").into_string(), "tick ` alone");
        assert_eq!(doc_inline("``").into_string(), "<code></code>");
        assert_eq!(doc_inline("").into_string(), "");
    }

    /// The page header's search box is `id="q"`. A declaration named `q` must not
    /// duplicate that id, or its index link would jump to the search box.
    #[test]
    fn a_declaration_named_like_a_shell_id_does_not_duplicate_it() {
        let page = doc_page_with_decls(Vec::new(), vec![doc_decl_entry("func", "q", "")]);
        let rendered = docs_page("reg", &docs_view(Some(page))).into_string();
        assert_eq!(rendered.matches(r#"id="q""#).count(), 1, "{rendered}");
        assert!(rendered.contains(r#"id="doc-q""#), "{rendered}");
        assert!(rendered.contains(r##"href="#doc-q""##), "{rendered}");
    }

    /// **plan-126-F Phase 3's escaping test.** Each publisher-controlled field
    /// carries its own hostile marker, so the assertions prove every field was
    /// *rendered* escaped — a field silently dropped would otherwise pass as
    /// "escaped" too.
    #[test]
    fn docs_page_escapes_every_publisher_controlled_field() {
        let attack = |field: &str| format!("<script>alert('{field}')</script>\" onmouseover=\"x");
        let mut decl = doc_decl_entry("func", "addUp", "");
        decl.signature = attack("signature");
        decl.example = attack("example");
        decl.args = vec![("a".to_string(), attack("param"))];
        decl.ret = attack("returns");
        decl.errors = vec![("ErrX".to_string(), attack("error"))];
        decl.desc = vec![(0, attack("decl-desc"))];
        let page = doc_page_with_decls(
            vec![(0, attack("package")), (1, attack("callout"))],
            vec![decl],
        );
        let rendered = docs_page("reg", &docs_view(Some(page))).into_string();

        for field in [
            "package",
            "callout",
            "signature",
            "example",
            "param",
            "returns",
            "error",
            "decl-desc",
        ] {
            let escaped =
                format!("&lt;script&gt;alert('{field}')&lt;/script&gt;&quot; onmouseover=&quot;x");
            assert!(
                rendered.contains(&escaped),
                "{field} must render escaped: {rendered}"
            );
        }
        assert!(!rendered.contains("<script"), "{rendered}");
        // No attribute break: every `onmouseover` on the page is the escaped
        // text of one of the eight fields, never an attribute. (A bare
        // ` onmouseover=` substring check would match that escaped *text*.)
        assert!(!rendered.contains("\" onmouseover=\""), "{rendered}");
        assert_eq!(rendered.matches("onmouseover").count(), 8, "{rendered}");
        assert_eq!(
            rendered.matches("&quot; onmouseover=&quot;x").count(),
            8,
            "{rendered}"
        );
    }

    /// maud escapes interpolated values by default. This asserts the property
    /// the whole layer-1 argument rests on, rather than assuming it.
    #[test]
    fn interpolated_values_are_escaped_by_the_template_engine() {
        let hostile = "<script>alert(1)</script>";
        let rendered = html! { p { (hostile) } }.into_string();
        assert!(!rendered.contains("<script"), "{rendered}");
        assert!(rendered.contains("&lt;script&gt;"), "{rendered}");

        // Including inside an attribute value.
        let rendered = html! { span title=(hostile) { "x" } }.into_string();
        assert!(!rendered.contains("<script"), "{rendered}");

        // And a hostile *url* rendered through external_link.
        let rendered = external_link("javascript:\"><script>alert(1)</script>").into_string();
        assert!(!rendered.contains("<script"), "{rendered}");
    }

    /// Every HTML response carries the CSP because the shared builder attaches
    /// it — no handler opts in, so no handler can forget.
    #[test]
    fn every_html_response_carries_the_csp_from_the_shared_builder() {
        let response = html_response(StatusCode::OK, html! { p { "hi" } });
        let headers = response.headers();
        assert_eq!(
            headers
                .get(header::CONTENT_SECURITY_POLICY)
                .unwrap()
                .to_str()
                .unwrap(),
            CONTENT_SECURITY_POLICY,
        );
        assert_eq!(
            headers.get(header::CONTENT_TYPE).unwrap(),
            "text/html; charset=utf-8",
        );
        assert_eq!(
            headers.get(header::X_CONTENT_TYPE_OPTIONS).unwrap(),
            "nosniff"
        );
        assert_eq!(headers.get(header::REFERRER_POLICY).unwrap(), "no-referrer");

        // Also on a non-200 — an error page is exactly where a forgotten
        // header would be least noticed.
        let response = html_response(StatusCode::NOT_FOUND, html! { p { "nope" } });
        assert_eq!(
            response
                .headers()
                .get(header::CONTENT_SECURITY_POLICY)
                .unwrap()
                .to_str()
                .unwrap(),
            CONTENT_SECURITY_POLICY,
        );
    }

    /// The policy must not quietly acquire a script permission. Spelled out as
    /// a test so weakening it requires editing an assertion that says why.
    #[test]
    fn the_policy_permits_no_script_at_all() {
        assert!(CONTENT_SECURITY_POLICY.contains("default-src 'none'"));
        assert!(
            !CONTENT_SECURITY_POLICY.contains("script-src"),
            "default-src 'none' already denies script; a script-src value can \
             only widen it",
        );
        assert!(!CONTENT_SECURITY_POLICY.contains("unsafe-inline"));
        assert!(!CONTENT_SECURITY_POLICY.contains("unsafe-eval"));
        assert!(CONTENT_SECURITY_POLICY.contains("frame-ancestors 'none'"));
        assert!(CONTENT_SECURITY_POLICY.contains("base-uri 'none'"));
    }

    /// The page shell needs no script and links the stylesheet as a real
    /// route, which is what `style-src 'self'` (no `'unsafe-inline'`) requires.
    #[test]
    fn the_page_shell_is_script_free_and_links_a_real_stylesheet() {
        let rendered = page("t", "reg.example", html! { p { "body" } }).into_string();
        assert!(!rendered.contains("<script"), "{rendered}");
        assert!(!rendered.contains("style="), "no inline styles: {rendered}");
        assert!(rendered.contains("<link rel=\"stylesheet\" href=\"/style.css\""));
        // The search form is a plain GET form, so search works with JavaScript
        // disabled.
        assert!(rendered.contains("action=\"/search.html\""));
    }
}
