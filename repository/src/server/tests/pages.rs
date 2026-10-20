//! plan-176-I: `server::tests` — the pages tests. The shared harnesses (owners, transfers,\n//! MFPC builders, signed requests) live in `server/tests/mod.rs`.

use super::*;

/// plan-61-B Phase 1 / §4 — the sub-plan's central behavioural claim.
///
/// A **yanked** version must still be listed. A view that silently omits
/// non-current versions reproduces exactly the truncation the open SUP-03
/// downgrade attack performs (`plan-61-repo-web.md` §4), so `state` is a
/// field to render and never a filter the server applies. This is the test
/// that would fail if someone later "tidied" the query with a WHERE clause.
#[tokio::test]
async fn package_detail_lists_every_version_including_yanked_ones() {
    let h = harness();
    register_owner_with_all_keys(&h.store, "alice");
    let alice_id = h.store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
    h.store
        .record_native_blob("vendorhash", "data/vendorhash.bin")
        .unwrap();

    let targets = [
        crate::abi::VendorBlobRef {
            logical: "snd".to_string(),
            source: "libsnd.dylib".to_string(),
            hash: "vendorhash".to_string(),
            os: "macos".to_string(),
            arch: None,
            libc: None,
            lib_type: "vendor".to_string(),
        },
        crate::abi::VendorBlobRef {
            logical: "snd".to_string(),
            source: "libsnd.a".to_string(),
            hash: "vendorhash".to_string(),
            os: "linux".to_string(),
            arch: Some("x86_64".to_string()),
            libc: Some("musl".to_string()),
            lib_type: "vendor".to_string(),
        },
    ];
    for (version, blobs) in [("1.0.0", &targets[..]), ("2.0.0", &[][..])] {
        h.store
            .publish_package_version(
                alice_id,
                "alice#toolbox",
                version,
                &format!("hash-{version}"),
                &format!("data/{version}.mfp"),
                "{}",
                blobs,
                &crate::store::PublishMetadata {
                    author: Some("alice".to_string()),
                    url: Some("https://example.invalid".to_string()),
                    description: None,
                    docs: None,
                    publisher_ident_fingerprint: None,
                },
            )
            .unwrap();
    }
    h.store
        .set_release_state(alice_id, "alice#toolbox", "1.0.0", "yanked")
        .unwrap();

    // `%23` is percent-decoded by axum before the handler sees it, so the
    // handler receives a literal `#`.
    let detail = package_detail(
        State(h.state.clone()),
        axum::extract::Path("alice#toolbox".to_string()),
    )
    .await
    .expect("detail served")
    .0;

    assert_eq!(detail.ident, "alice#toolbox");
    assert_eq!(detail.owner, "alice");
    assert_eq!(detail.versions.len(), 2, "the yanked version is still here");
    // The yanked release here is the *older* one, so the newest-active
    // selection (plan-126-A) and a bare newest-row pick agree: 2.0.0.
    // The case where they disagree is covered by
    // `package_detail_names_the_newest_active_release_not_a_yanked_newest`.
    assert_eq!(detail.latest_version.as_deref(), Some("2.0.0"));
    assert_eq!(detail.latest_state.as_deref(), Some("available"));
    let yanked = detail
        .versions
        .iter()
        .find(|version| version.version == "1.0.0")
        .expect("the yanked version must be listed, not filtered out");
    assert_eq!(yanked.state, "yanked");

    // Both platform targets render, and the any-arch wildcard stays null
    // rather than collapsing into the concrete arch.
    assert_eq!(yanked.targets.len(), 2);
    let macos = &yanked.targets[0];
    assert_eq!(macos.os, "macos");
    assert_eq!(macos.arch, None);
    assert_eq!(macos.lib_type, "vendor");
    let linux = &yanked.targets[1];
    assert_eq!(linux.arch.as_deref(), Some("x86_64"));
    assert_eq!(linux.libc.as_deref(), Some("musl"));

    // Package-level metadata comes from the newest version.
    assert_eq!(detail.author.as_deref(), Some("alice"));
    assert_eq!(detail.url.as_deref(), Some("https://example.invalid"));
    // `description` is in the shape from day one, null until plan-61-E, so
    // that sub-plan adds no field and breaks no consumer.
    assert_eq!(detail.description, None);
}

/// plan-126-A — the handler seam, which the `web::package_page` tests cannot
/// reach: `package_detail` must read its headline version from
/// `Store::latest_active_version`, not from `versions.first()`.
///
/// The newest release is yanked here, which is precisely the shape
/// `package_detail_lists_every_version_including_yanked_ones` does *not*
/// cover — that test yanks the older version, so both selections agree and
/// it stayed green throughout the bug's lifetime.
#[tokio::test]
async fn package_detail_names_the_newest_active_release_not_a_yanked_newest() {
    let h = harness();
    register_owner_with_all_keys(&h.store, "alice");
    let alice_id = h.store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
    for version in ["1.5.0", "2.0.0"] {
        h.store
            .publish_package_version(
                alice_id,
                "alice#toolbox",
                version,
                &format!("hash-{version}"),
                &format!("data/{version}.mfp"),
                "{}",
                &[],
                &crate::store::PublishMetadata::default(),
            )
            .unwrap();
    }
    h.store
        .set_release_state(alice_id, "alice#toolbox", "2.0.0", "yanked")
        .unwrap();

    let detail = package_detail(
        State(h.state.clone()),
        axum::extract::Path("alice#toolbox".to_string()),
    )
    .await
    .expect("detail served")
    .0;

    assert_eq!(
        detail.latest_version.as_deref(),
        Some("1.5.0"),
        "a yanked newest release must not be advertised as the latest",
    );
    assert_eq!(detail.latest_state.as_deref(), Some("available"));
    // And the listing is still complete: the selection narrowed, the
    // transparency view did not.
    assert_eq!(detail.versions.len(), 2);
    assert_eq!(detail.versions[0].version, "2.0.0");
    assert_eq!(detail.versions[0].state, "yanked");

    // Withdraw the fallback too: now there is no active release, and the
    // field is null rather than naming an ineligible version.
    h.store
        .set_release_state(alice_id, "alice#toolbox", "1.5.0", "yanked")
        .unwrap();
    let detail = package_detail(
        State(h.state.clone()),
        axum::extract::Path("alice#toolbox".to_string()),
    )
    .await
    .expect("detail served")
    .0;
    assert_eq!(detail.latest_version, None);
    assert_eq!(detail.latest_state, None);
    assert_eq!(detail.versions.len(), 2, "still listed, still auditable");
}

/// plan-61-B §4: the audit route exposes an **inclusion proof** per publish,
/// not just an index. A rendered log index nobody can verify proves nothing;
/// the proof against the checkpoint in the same response is what lets a
/// third-party monitor catch a registry equivocating.
#[tokio::test]
async fn package_audit_returns_a_checkpoint_with_verifiable_inclusion_proofs() {
    let h = harness();
    register_owner_with_all_keys(&h.store, "alice");
    let alice_id = h.store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
    for version in ["1.0.0", "2.0.0"] {
        h.store
            .publish_package_version(
                alice_id,
                "alice#toolbox",
                version,
                &format!("hash-{version}"),
                &format!("data/{version}.mfp"),
                "{}",
                &[],
                &crate::store::PublishMetadata::default(),
            )
            .unwrap();
    }
    h.store
        .set_release_state(alice_id, "alice#toolbox", "1.0.0", "yanked")
        .unwrap();

    let audit = package_audit(
        State(h.state.clone()),
        peer("127.0.0.1"),
        axum::extract::Path("alice#toolbox".to_string()),
    )
    .await
    .expect("audit served")
    .0;

    assert_eq!(audit.ident, "alice#toolbox");
    assert_eq!(audit.owner, "alice");
    assert_eq!(audit.publishes.len(), 2);
    assert!(audit.log_checkpoint.size >= 2);

    // Every proof actually verifies against the checkpoint served
    // alongside it — the property that makes the tab worth anything.
    let root: [u8; 32] = hex::decode(&audit.log_checkpoint.root_hash)
        .unwrap()
        .try_into()
        .unwrap();
    for entry in &audit.publishes {
        let leaf: [u8; 32] = hex::decode(&entry.leaf_hash).unwrap().try_into().unwrap();
        let path: Vec<[u8; 32]> = entry
            .proof
            .iter()
            .map(|hop| hex::decode(hop).unwrap().try_into().unwrap())
            .collect();
        crate::log::verify_inclusion(
            entry.index as usize,
            audit.log_checkpoint.size as usize,
            &leaf,
            &path,
            &root,
        )
        .unwrap_or_else(|err| {
            panic!(
                "inclusion proof for {} must verify against the served checkpoint: {err}",
                entry.version,
            )
        });
    }

    // The yank shows up as a release-state transition.
    assert!(
        audit
            .state_changes
            .iter()
            .any(|change| change.version == "1.0.0" && change.state == "yanked"),
        "{:?}",
        audit.state_changes,
    );
}

/// The rendered-HTML half of the phase: the yanked version **string** must
/// not reach the search result chip. A JSON assertion cannot see a renderer
/// that fell back to some other field.
#[tokio::test]
async fn the_search_page_never_shows_a_yanked_version_in_the_result_chip() {
    let h = harness();
    seed_packages(&h, &["alice#toolbox"]);
    let alice_id = h.store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
    h.store
        .publish_package_version(
            alice_id,
            "alice#toolbox",
            "9.9.9",
            "hash-9",
            "data/9.mfp",
            "{}",
            &[],
            &crate::store::PublishMetadata::default(),
        )
        .unwrap();
    h.store
        .set_release_state(alice_id, "alice#toolbox", "9.9.9", "yanked")
        .unwrap();

    let (status, _headers, body) = get_page(&h.state, "/search.html?q=toolbox").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("alice#toolbox"), "{body}");
    assert!(body.contains("v1.0.0"), "{body}");
    assert!(
        !body.contains("9.9.9"),
        "the yanked version must not appear anywhere in the result: {body}",
    );

    // And with nothing active, the page states it rather than showing a chip.
    h.store
        .set_release_state(alice_id, "alice#toolbox", "1.0.0", "yanked")
        .unwrap();
    let (status, _headers, body) = get_page(&h.state, "/search.html?q=toolbox").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("alice#toolbox"), "{body}");
    assert!(body.contains("no active release"), "{body}");
    assert!(!body.contains("v1.0.0"), "{body}");
}

/// A `deprecated` search headline renders its badge, which is the thing the
/// bare version chip could never express.
#[tokio::test]
async fn the_search_page_badges_a_deprecated_headline_release() {
    let h = harness();
    seed_packages(&h, &["alice#toolbox"]);
    let alice_id = h.store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
    h.store
        .set_release_state(alice_id, "alice#toolbox", "1.0.0", "deprecated")
        .unwrap();

    let (status, _headers, body) = get_page(&h.state, "/search.html?q=toolbox").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("v1.0.0"), "{body}");
    assert!(body.contains("state--deprecated"), "{body}");
}

/// plan-61-C Phase 2: adding a `/` handler and the `.html` routes must not
/// shadow any existing JSON route. `/health` and `/ident` are called out
/// specifically because a catch-all or a misplaced `/` route is exactly how
/// they would silently start returning HTML.
#[tokio::test]
async fn the_html_routes_do_not_shadow_any_json_route() {
    let h = harness();

    for (uri, expected) in [
        ("/health", "application/json"),
        ("/ident", "application/json"),
        ("/log/checkpoint", "application/json"),
        ("/search?q=x", "application/json"),
    ] {
        let (status, headers, _body) = get_page(&h.state, uri).await;
        assert_eq!(status, StatusCode::OK, "{uri}");
        assert!(
            headers
                .get(header::CONTENT_TYPE)
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with(expected),
            "{uri} must still be JSON, not HTML",
        );
    }

    // And the HTML routes are HTML, with the CSP.
    for uri in ["/", "/search.html?q=x"] {
        let (status, headers, _body) = get_page(&h.state, uri).await;
        assert_eq!(status, StatusCode::OK, "{uri}");
        assert!(headers
            .get(header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("text/html"));
        assert_eq!(
            headers
                .get(header::CONTENT_SECURITY_POLICY)
                .unwrap()
                .to_str()
                .unwrap(),
            crate::web::CONTENT_SECURITY_POLICY,
            "{uri}",
        );
    }

    // The stylesheet is a real route, which is what `style-src 'self'`
    // with no `'unsafe-inline'` requires.
    let (status, headers, body) = get_page(&h.state, "/style.css").await;
    assert_eq!(status, StatusCode::OK);
    assert!(headers
        .get(header::CONTENT_TYPE)
        .unwrap()
        .to_str()
        .unwrap()
        .starts_with("text/css"));
    assert!(
        body.contains("--font-sans"),
        "the real stylesheet is served"
    );
}

/// The landing page shows the **root** fingerprint, worded as something to
/// compare rather than as a trust claim (plan-61-C §4). Showing the
/// `/ident` server fingerprint above a `mfb repo trust` command — which
/// consumes the *root* fingerprint — would read as a verified copy-paste
/// and then fail.
#[tokio::test]
async fn the_landing_page_presents_the_root_fingerprint_as_a_thing_to_compare() {
    let h = harness();
    h.store
        .init_registry_root("reg.example", 4_102_444_800)
        .unwrap();
    let config = h.store.registry_config().unwrap().unwrap();
    let root_fingerprint = crypto::fingerprint(&config.root_public);
    let server_fingerprint = crypto::fingerprint(&h.store.server_keypair().unwrap().0);

    let (status, _headers, body) = get_page(&h.state, "/").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains(&root_fingerprint),
        "the root fingerprint must be shown",
    );
    assert!(
        !body.contains(&server_fingerprint),
        "the /ident server fingerprint is a different value from a different \
             key and must not appear next to `mfb repo trust`",
    );
    assert!(body.contains("mfb repo trust reg.example"));

    // The copy must not claim the page authenticates itself.
    assert!(body.contains("Compare this"));
    assert!(body.contains("cannot prove that it is the real mfb-repo"));
    // Scoped to the fingerprint section: the footer legitimately says
    // content is "unverified by the registry", which contains "verified".
    let section = body
        .split("class=\"fingerprint\"")
        .nth(1)
        .expect("the fingerprint section is present")
        .split("</section>")
        .next()
        .unwrap();
    for overclaim in [
        "is verified",
        "Verified",
        "secure connection",
        "trusted registry",
        "authentic",
    ] {
        assert!(
            !section.contains(overclaim),
            "the fingerprint block must not overclaim with {overclaim:?}",
        );
    }

    // No script, no inline style, and a real stylesheet link.
    assert!(!body.contains("<script"));
    assert!(!body.contains("style="));
    assert!(body.contains("href=\"/style.css\""));
}

/// A search that matches nothing renders a "no results" page with **HTTP
/// 200**, not a 404: the request succeeded and the answer is "none".
#[tokio::test]
async fn the_search_page_renders_all_three_states() {
    let h = harness();
    seed_packages(&h, &["alice#toolbox"]);

    // Results.
    let (status, _headers, body) = get_page(&h.state, "/search.html?q=toolbox").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("alice#toolbox"));
    assert!(
        body.contains("/p/alice%23toolbox"),
        "results link to the package page"
    );
    assert!(body.contains("1</strong> results"), "{body}");

    // No results — 200, not 404.
    let (status, _headers, body) = get_page(&h.state, "/search.html?q=xyzzy-nope").await;
    assert_eq!(
        status,
        StatusCode::OK,
        "an empty result set is a successful request, not a missing page",
    );
    assert!(body.contains("No packages match this query."));

    // Empty query — the form, and no enumeration of the whole table.
    let (status, _headers, body) = get_page(&h.state, "/search.html").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("Search the registry"));
    assert!(
        !body.contains("alice#toolbox"),
        "an empty query must not list the registry",
    );
}

/// A hostile ident renders escaped in the results list. The ident charset is
/// restricted at publish, so this drives the renderer directly — the
/// escaping must not depend on an upstream validator that a future change
/// could relax.
#[tokio::test]
async fn search_results_escape_html_metacharacters_in_publisher_values() {
    let rows = [crate::web::SearchRow {
        ident: "alice#<script>alert(1)</script>".to_string(),
        owner: "<img src=x onerror=alert(1)>".to_string(),
        latest_version: Some("1.0.0\"><script>".to_string()),
        // plan-126-A added this field, so it joins the hostile-value set:
        // it reaches a `class=` attribute via `state_modifier`, and a value
        // outside the vocabulary must not be able to break out of it.
        latest_state: Some("\" onmouseover=\"alert(1)".to_string()),
        description: Some("<b>bold</b>".to_string()),
        published_at: Some(1_700_000_000),
    }];
    let rendered = crate::web::search_page("reg", "<script>", &rows).into_string();

    // Assert the absence of live *markup*, not of scary substrings: an
    // escaped `&lt;img src=x onerror=alert(1)&gt;` legitimately still
    // contains the text "onerror=", and asserting on that would be a test
    // that fails for the wrong reason.
    assert!(!rendered.contains("<script"), "{rendered}");
    assert!(!rendered.contains("<img"), "{rendered}");
    assert!(!rendered.contains("<b>bold"), "{rendered}");
    assert!(rendered.contains("&lt;script&gt;"));
    assert!(
        rendered.contains("&lt;img src=x onerror=alert(1)&gt;"),
        "{rendered}"
    );
    // The echoed query is publisher-independent but still user-controlled.
    assert!(rendered.contains("query-echo"));

    // plan-126-A: `latest_state` reaches a `class=` attribute through
    // `state_modifier`, a total map onto a fixed vocabulary — an
    // out-of-vocabulary value becomes `state--other`, so the hostile string
    // never reaches the attribute at all.
    assert!(rendered.contains("state--other"), "{rendered}");
    // The raw value does still render as visible text, so — per this test's
    // own rule — assert on the absence of an attribute *break*, not on the
    // scary substring: an escaped `&quot; onmouseover=&quot;` legitimately
    // contains the text `onmouseover=`. The quote is what would end the
    // attribute, and it is escaped.
    assert!(
        !rendered.contains(r#"" onmouseover=""#),
        "the state value must not break out of its attribute: {rendered}",
    );
    assert!(
        rendered.contains("&quot; onmouseover=&quot;alert(1)"),
        "{rendered}"
    );
}

/// **The XSS regression test** (plan-61-C Phase 3) — the single most
/// important test in this sub-plan.
///
/// Publishes a package whose `author` is `<script>alert(1)</script>` and
/// whose `url` is `javascript:alert(1)`, then asserts the rendered page:
/// shows the author as visible escaped text, contains no `<script`
/// substring anywhere, and renders the hostile url as text rather than an
/// anchor. This is the same hostile fixture the mockups use, kept in sync
/// as §3.1 requires.
///
/// It drives the **real route through the real router**, not the template
/// function, so it covers the whole path including the CSP header.
#[tokio::test]
async fn a_hostile_author_and_url_render_inert() {
    let h = harness();
    register_owner_with_all_keys(&h.store, "alice");
    let alice_id = h.store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
    h.store
        .record_native_blob("vendorhash", "data/vendorhash.bin")
        .unwrap();
    h.store
        .publish_package_version(
            alice_id,
            "alice#toolbox",
            "1.0.0",
            "hash-1",
            "data/1.mfp",
            "{}",
            &[crate::abi::VendorBlobRef {
                logical: "<script>alert('logical')</script>".to_string(),
                source: "<img src=x onerror=alert('source')>".to_string(),
                hash: "vendorhash".to_string(),
                os: "linux".to_string(),
                arch: None,
                libc: None,
                lib_type: "vendor".to_string(),
            }],
            &crate::store::PublishMetadata {
                author: Some("<script>alert(1)</script>".to_string()),
                url: Some("javascript:alert(1)".to_string()),
                // plan-61-E extends this fixture to the description too.
                description: Some("<img src=x onerror=alert('desc')>".to_string()),
                docs: None,
                publisher_ident_fingerprint: None,
            },
        )
        .unwrap();

    let (status, headers, body) = get_page(&h.state, "/p/alice%23toolbox").await;
    assert_eq!(status, StatusCode::OK);

    // 1. No live script anywhere on the page — from any of the four
    //    publisher-controlled fields.
    assert!(!body.contains("<script"), "{body}");
    assert!(!body.contains("<img"), "{body}");
    // NB: the *string* `javascript:` is expected to appear — §2 requires
    // the hostile url to render as visible inert text, so asserting its
    // absence would contradict the design. What must never appear is the
    // url in an attribute position; that is asserted below.

    // 2. The hostile author is *visible*, escaped. Hiding it would keep the
    //    hostile value from the reader best placed to notice it.
    assert!(
        body.contains("&lt;script&gt;alert(1)&lt;/script&gt;"),
        "the author must render as visible escaped text: {body}",
    );

    // 3. The hostile url is text, not an anchor, and is annotated as
    //    withheld rather than silently dropped.
    assert!(
        body.contains("url-inert"),
        "the javascript: url must render inert: {body}",
    );
    assert!(body.contains("link withheld"), "{body}");
    assert!(
        body.contains(">javascript:alert(1)<"),
        "the hostile url must still be visible to the reader: {body}",
    );
    assert!(
        !body.contains("href=\"javascript"),
        "the hostile url must never become an href: {body}",
    );
    assert!(
        !body.contains("=\"javascript:"),
        "the hostile url must never reach any attribute: {body}",
    );

    // 4. plan-61-E: the description is publisher-controlled too, and gets
    //    the same treatment — visible, escaped, and never a live attribute.
    assert!(
        body.contains("&lt;img src=x onerror=alert(&#39;desc&#39;)&gt;")
            || body.contains("&lt;img src=x onerror=alert('desc')&gt;"),
        "the hostile description must render as visible escaped text: {body}",
    );

    // 5. The CSP is present, so even a total escaping failure could not
    //    execute.
    assert_eq!(
        headers
            .get(header::CONTENT_SECURITY_POLICY)
            .unwrap()
            .to_str()
            .unwrap(),
        crate::web::CONTENT_SECURITY_POLICY,
    );
}

/// The rendered version table shows every state, and the target table
/// renders a NULL arch as "any" rather than as a blank cell that would read
/// as missing data (plan-61-C Phase 3).
#[tokio::test]
async fn the_package_page_shows_yanked_versions_and_the_target_matrix() {
    let h = harness();
    register_owner_with_all_keys(&h.store, "alice");
    let alice_id = h.store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
    h.store
        .record_native_blob("vendorhash", "data/vendorhash.bin")
        .unwrap();
    let targets = [
        crate::abi::VendorBlobRef {
            logical: "snd".to_string(),
            source: "libsnd.dylib".to_string(),
            hash: "vendorhash".to_string(),
            os: "macos".to_string(),
            arch: None,
            libc: None,
            lib_type: "vendor".to_string(),
        },
        crate::abi::VendorBlobRef {
            logical: "snd".to_string(),
            source: "libsnd.a".to_string(),
            hash: "vendorhash".to_string(),
            os: "linux".to_string(),
            arch: Some("x86_64".to_string()),
            libc: Some("musl".to_string()),
            lib_type: "vendor".to_string(),
        },
    ];
    for (version, blobs) in [("1.0.0", &targets[..]), ("2.0.0", &[][..])] {
        h.store
            .publish_package_version(
                alice_id,
                "alice#toolbox",
                version,
                &format!("hash-{version}"),
                &format!("data/{version}.mfp"),
                "{}",
                blobs,
                &crate::store::PublishMetadata::default(),
            )
            .unwrap();
    }
    h.store
        .set_release_state(alice_id, "alice#toolbox", "1.0.0", "yanked")
        .unwrap();

    let (status, _headers, body) = get_page(&h.state, "/p/alice%23toolbox").await;
    assert_eq!(status, StatusCode::OK);

    // Every version present, the yanked one labeled and visually marked.
    assert!(
        body.contains("Versions <span class=\"muted\">(2)"),
        "{body}"
    );
    assert!(body.contains("state--yanked"), "{body}");
    assert!(body.contains(">yanked<"), "{body}");
    assert!(body.contains("state--available"), "{body}");

    // Two target rows, and the wildcard arch reads as "any".
    assert!(body.contains("Native targets — 2 for v1.0.0"), "{body}");
    assert!(
        body.contains(">any<"),
        "a NULL arch renders as \"any\": {body}"
    );
    assert!(body.contains("x86_64"));
    assert!(body.contains("musl"));

    // No script on the page at all.
    assert!(!body.contains("<script"));
}

/// The audit tab renders the checkpoint, the per-publish inclusion proof
/// path, and links the raw JSON so a monitor can script against it.
#[tokio::test]
async fn the_audit_tab_renders_proofs_and_links_the_raw_json() {
    let h = harness();
    register_owner_with_all_keys(&h.store, "alice");
    let alice_id = h.store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
    for version in ["1.0.0", "2.0.0"] {
        h.store
            .publish_package_version(
                alice_id,
                "alice#toolbox",
                version,
                &format!("hash-{version}"),
                &format!("data/{version}.mfp"),
                "{}",
                &[],
                &crate::store::PublishMetadata::default(),
            )
            .unwrap();
    }
    h.store
        .set_release_state(alice_id, "alice#toolbox", "1.0.0", "yanked")
        .unwrap();

    let (status, _headers, body) = get_page(&h.state, "/p/alice%23toolbox/audit").await;
    assert_eq!(status, StatusCode::OK);

    assert!(body.contains("Log checkpoint"));
    assert!(body.contains("entries"));
    assert!(body.contains("inclusion proof"));
    assert!(
        body.contains("proof-path"),
        "the proof path renders: {body}"
    );
    // The raw JSON endpoint is linked for third-party monitors.
    assert!(
        body.contains("/packages/alice%23toolbox/audit"),
        "the raw JSON endpoint must be linked: {body}",
    );
    // The yank appears as a state transition.
    assert!(body.contains("State changes"));
    assert!(body.contains("state--yanked"));
    // The copy frames this as evidence, not assurance.
    assert!(body.contains("evidence for you to check"));
    assert!(!body.contains("<script"));
}

fn toolbox_doc_section() -> Vec<u8> {
    mfb_wire::docs::encode_doc_table(&mfb_wire::docs::PackageDocs {
        package: Some(mfb_wire::docs::PackageDocEntry {
            name: "toolbox".to_string(),
            desc: vec![
                (0, "Toolbox subtitle.".to_string()),
                (2, "An informational note.".to_string()),
            ],
            deprecated: None,
        }),
        decls: Vec::new(),
    })
}

/// **plan-126-F Phase 1's acceptance test.** Each of the three package pages,
/// rendered through the real router, carries exactly one
/// `aria-current="page"`, on its own tab.
#[tokio::test]
async fn every_package_tab_page_marks_exactly_one_current_tab() {
    let h = harness();
    seed_toolbox(&h, None);
    for (uri, label) in [
        ("/p/alice%23toolbox", "Overview"),
        ("/p/alice%23toolbox/docs", "Docs"),
        ("/p/alice%23toolbox/audit", "Audit"),
    ] {
        let (status, _headers, body) = get_page(&h.state, uri).await;
        assert_eq!(status, StatusCode::OK, "{uri}");
        assert_eq!(
            body.matches("aria-current=\"page\"").count(),
            1,
            "{uri} must mark exactly one tab: {body}"
        );
        assert!(
            body.contains(&format!("aria-current=\"page\">{label}</a>")),
            "{uri} must mark {label}: {body}"
        );
    }
}

/// A release with no stored documentation gets the tab, HTTP 200, and an
/// explicit statement — not a 404 and not a hidden tab.
#[tokio::test]
async fn the_docs_tab_states_when_a_release_has_no_documentation() {
    let h = harness();
    seed_toolbox(&h, None);
    let (status, headers, body) = get_page(&h.state, "/p/alice%23toolbox/docs").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers.get(header::CONTENT_SECURITY_POLICY).unwrap(),
        crate::web::CONTENT_SECURITY_POLICY,
    );
    assert!(body.contains("did not include documentation"), "{body}");
    assert!(body.contains("v1.0.0"), "{body}");
}

/// Stored documentation renders under the unchanged site CSP, with no inline
/// style and no script — the two things the compiler's own renderer would
/// have needed.
#[tokio::test]
async fn the_docs_tab_renders_stored_documentation_under_the_site_csp() {
    let h = harness();
    seed_toolbox(&h, Some(toolbox_doc_section()));
    let (status, headers, body) = get_page(&h.state, "/p/alice%23toolbox/docs").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers.get(header::CONTENT_SECURITY_POLICY).unwrap(),
        crate::web::CONTENT_SECURITY_POLICY,
    );
    assert!(body.contains("Toolbox subtitle."), "{body}");
    assert!(body.contains("An informational note."), "{body}");
    assert!(body.contains("callout--info"), "{body}");
    assert!(!body.contains("did not include documentation"), "{body}");
    assert!(!body.contains("<style"), "no inline style: {body}");
    assert!(!body.contains("<script"), "no script: {body}");
}

/// An unknown ident's Docs tab 404s exactly like the other tabs, with the
/// same not-found page, because it resolves through `package_detail`.
#[tokio::test]
async fn an_unknown_packages_docs_tab_404s_like_the_other_tabs() {
    let h = harness();
    seed_toolbox(&h, None);
    let (overview_status, _h1, overview_body) = get_page(&h.state, "/p/alice%23nope").await;
    let (docs_status, _h2, docs_body) = get_page(&h.state, "/p/alice%23nope/docs").await;
    assert_eq!(docs_status, StatusCode::NOT_FOUND);
    assert_eq!(docs_status, overview_status);
    assert!(docs_body.contains("Package not found"), "{docs_body}");
    assert!(
        overview_body.contains("Package not found"),
        "{overview_body}"
    );
}

/// A doc section whose subtitle names the release, with one public
/// declaration (in group `Math`) and one the author marked `INTERNAL`.
fn release_doc_section(release_tag: &str) -> Vec<u8> {
    let decl = |name: &str, internal: bool| mfb_wire::docs::DeclDocEntry {
        kind: "func".to_string(),
        name: name.to_string(),
        signature: format!("EXPORT FUNC {name}() AS Integer"),
        group: "Math".to_string(),
        desc: vec![(0, format!("{name} of {release_tag}."))],
        args: vec![("a".to_string(), "An argument.".to_string())],
        props: Vec::new(),
        ret: "A number.".to_string(),
        errors: Vec::new(),
        example: format!("PRINT toolbox::{name}()"),
        internal,
        deprecated: None,
    };
    mfb_wire::docs::encode_doc_table(&mfb_wire::docs::PackageDocs {
        package: Some(mfb_wire::docs::PackageDocEntry {
            name: "toolbox".to_string(),
            desc: vec![(0, format!("Toolbox {release_tag}."))],
            deprecated: None,
        }),
        decls: vec![decl("addUp", false), decl("secretHelper", true)],
    })
}

fn publish_toolbox_release(h: &Harness, version: &str, docs: Option<Vec<u8>>) {
    let alice_id = h.store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
    h.store
        .publish_package_version(
            alice_id,
            "alice#toolbox",
            version,
            &format!("hash-{version}"),
            &format!("data/{version}.mfp"),
            "{}",
            &[],
            &crate::store::PublishMetadata {
                docs,
                ..Default::default()
            },
        )
        .unwrap();
}

fn docs_json(body: &str) -> PackageDocsResponse {
    serde_json::from_str(body).unwrap_or_else(|err| panic!("{err}: {body}"))
}

/// **plan-126-F Validation Plan: the JSON/HTML parity test.** Both routes
/// report the same release, the same groups and the same declaration
/// anchors, and both omit the internal declaration.
#[tokio::test]
async fn the_docs_json_route_mirrors_the_docs_tab() {
    let h = harness();
    register_owner_with_all_keys(&h.store, "alice");
    publish_toolbox_release(&h, "1.0.0", Some(release_doc_section("r1")));

    let (json_status, json_headers, json_body) =
        get_page(&h.state, "/packages/alice%23toolbox/docs").await;
    let (html_status, _headers, html_body) = get_page(&h.state, "/p/alice%23toolbox/docs").await;
    assert_eq!(json_status, StatusCode::OK, "{json_body}");
    assert_eq!(html_status, StatusCode::OK, "{html_body}");
    assert!(json_headers
        .get(header::CONTENT_TYPE)
        .unwrap()
        .to_str()
        .unwrap()
        .starts_with("application/json"));

    let docs = docs_json(&json_body);
    assert_eq!(docs.ident, "alice#toolbox");
    assert_eq!(docs.version.as_deref(), Some("1.0.0"));
    assert!(html_body.contains("v1.0.0"), "{html_body}");
    // The HTML tab links its JSON mirror.
    assert!(
        html_body.contains("/packages/alice%23toolbox/docs"),
        "{html_body}"
    );

    let documentation = docs.documentation.expect("documented release");
    assert_eq!(documentation.subtitle, "Toolbox r1.");
    assert!(html_body.contains("Toolbox r1."), "{html_body}");
    assert_eq!(documentation.groups.len(), 1);
    let group = &documentation.groups[0];
    assert_eq!(group.title, "Math");
    assert!(html_body.contains(">Math</h2>"), "{html_body}");
    let anchors: Vec<&str> = group
        .declarations
        .iter()
        .map(|decl| decl.anchor.as_str())
        .collect();
    assert_eq!(anchors, ["doc-addup"]);
    for anchor in anchors {
        assert!(
            html_body.contains(&format!("id=\"{anchor}\"")),
            "the JSON anchor {anchor} must be an element id on the tab: {html_body}"
        );
    }
    let add = &group.declarations[0];
    assert_eq!(add.signature, "EXPORT FUNC addUp() AS Integer");
    assert_eq!(add.parameters[0].name, "a");
    assert_eq!(add.returns, "A number.");
    assert_eq!(add.example, "PRINT toolbox::addUp()");
    assert!(!json_body.contains("secretHelper"), "{json_body}");
    assert!(!html_body.contains("secretHelper"), "{html_body}");
}

/// An undocumented release is `documentation: null` on the JSON route —
/// the state the HTML tab states in words.
#[tokio::test]
async fn the_docs_json_route_reports_null_for_an_undocumented_release() {
    let h = harness();
    seed_toolbox(&h, None);
    let (status, _headers, body) = get_page(&h.state, "/packages/alice%23toolbox/docs").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let docs = docs_json(&body);
    assert_eq!(docs.version.as_deref(), Some("1.0.0"));
    assert!(docs.documentation.is_none(), "{body}");
    assert!(body.contains("\"documentation\":null"), "{body}");
}

/// The JSON route refuses an unknown package with the same status as
/// `GET /packages/:ident`.
#[tokio::test]
async fn the_docs_json_route_404s_like_the_package_route() {
    let h = harness();
    seed_toolbox(&h, None);
    let (detail_status, _h1, _b1) = get_page(&h.state, "/packages/alice%23nope").await;
    let (docs_status, _h2, docs_body) = get_page(&h.state, "/packages/alice%23nope/docs").await;
    assert_eq!(detail_status, StatusCode::NOT_FOUND);
    assert_eq!(docs_status, detail_status);
    assert!(docs_body.contains("unknown package"), "{docs_body}");
}

/// **plan-126-F Validation Plan: the cross-surface check.** When the newest
/// release is yanked, both the tab and its JSON mirror serve the older
/// active release's documentation and name that release — plan-126-A's
/// selection end to end, not merely the newest row.
#[tokio::test]
async fn the_docs_tab_documents_the_older_active_release_when_the_newest_is_yanked() {
    let h = harness();
    register_owner_with_all_keys(&h.store, "alice");
    publish_toolbox_release(&h, "1.0.0", Some(release_doc_section("r1")));
    publish_toolbox_release(&h, "2.0.0", Some(release_doc_section("r2")));
    let alice_id = h.store.owner_with_ident_key("alice").unwrap().unwrap().0.id;

    let (_status, _headers, before) = get_page(&h.state, "/p/alice%23toolbox/docs").await;
    assert!(before.contains("Toolbox r2."), "{before}");
    assert!(before.contains("v2.0.0"), "{before}");

    h.store
        .set_release_state(alice_id, "alice#toolbox", "2.0.0", "yanked")
        .unwrap();

    let (status, _headers, html_body) = get_page(&h.state, "/p/alice%23toolbox/docs").await;
    assert_eq!(status, StatusCode::OK);
    assert!(html_body.contains("Toolbox r1."), "{html_body}");
    assert!(html_body.contains("v1.0.0"), "{html_body}");
    assert!(!html_body.contains("Toolbox r2."), "{html_body}");
    assert!(!html_body.contains("v2.0.0"), "{html_body}");

    let (_status, _headers, json_body) = get_page(&h.state, "/packages/alice%23toolbox/docs").await;
    let docs = docs_json(&json_body);
    assert_eq!(docs.version.as_deref(), Some("1.0.0"));
    assert_eq!(docs.documentation.unwrap().subtitle, "Toolbox r1.");
}

/// An unknown package renders a 404 **page**, not a bare status — and that
/// page still carries the CSP, because it is built by the shared builder.
#[tokio::test]
async fn an_unknown_package_renders_a_404_page_with_the_csp() {
    let h = harness();
    for uri in ["/p/alice%23nope", "/p/alice%23nope/audit"] {
        let (status, headers, body) = get_page(&h.state, uri).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
        assert!(body.contains("Package not found"), "{uri}: {body}");
        assert!(body.contains("<html"), "{uri} must render a page");
        assert_eq!(
            headers
                .get(header::CONTENT_SECURITY_POLICY)
                .unwrap()
                .to_str()
                .unwrap(),
            crate::web::CONTENT_SECURITY_POLICY,
            "{uri}",
        );
    }
}
