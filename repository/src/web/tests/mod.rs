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
