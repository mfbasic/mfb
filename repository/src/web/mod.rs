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

/// The shell every page shares: doctype, head, stylesheet link, and the masthead.
pub fn page(title: &str, body: Markup) -> Markup {
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
                    a."masthead-brand" href="/" { "mfb-repo" }
                    form."masthead-search" method="get" action="/search.html" {
                        input type="search" name="q" placeholder="Search packages"
                              aria-label="Search packages";
                        button type="submit" { "Search" }
                    }
                }
                main { (body) }
                footer."site-footer" {
                    p {
                        "Anonymous, read-only. No account, no cookie, no JavaScript. "
                        a href="/search.html" { "Browse" }
                    }
                }
            }
        }
    }
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
        let rendered = page("t", html! { p { "body" } }).into_string();
        assert!(!rendered.contains("<script"), "{rendered}");
        assert!(!rendered.contains("style="), "no inline styles: {rendered}");
        assert!(rendered.contains("<link rel=\"stylesheet\" href=\"/style.css\""));
        // The search form is a plain GET form, so search works with JavaScript
        // disabled.
        assert!(rendered.contains("method=\"get\" action=\"/search.html\""));
    }
}
