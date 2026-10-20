//! plan-176-I: `server::tests` — the search_limits tests. The shared harnesses (owners, transfers,\n//! MFPC builders, signed requests) live in `server/tests/mod.rs`.

use super::*;

#[tokio::test]
async fn challenge_rate_limit_trips_after_the_window_cap() {
    let temp = tempfile::tempdir().unwrap();
    let opened =
        Store::open_repository(&temp.path().join("meta.db"), &temp.path().join("data")).unwrap();
    let store = opened.store;
    register_owner_with_all_keys(&store, "alice");
    let state = AppState {
        store: store.clone(),
        blob_store: BlobStore::local(opened.packages_dir.clone()),
        rate_limiter: RateLimiter::new(),
    };
    // The per-IP challenge cap is `CHALLENGE_PER_IP_MAX` per window; the
    // next request from the same client IP is refused with 429.
    let mut last = Ok(());
    for _ in 0..(CHALLENGE_PER_IP_MAX + 1) {
        last = challenge(
            State(state.clone()),
            peer("127.0.0.1"),
            Json(ChallengeRequest {
                owner: "alice".to_string(),
                auth_fingerprint: crypto::fingerprint(&register_dummy()),
            }),
        )
        .await
        .map(|_| ())
        .map_err(|(status, _)| status);
    }
    assert_eq!(last.unwrap_err(), StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn challenge_rate_limit_cannot_lock_out_a_victim_by_owner_name() {
    // bug-419: the challenge bucket was keyed on the unauthenticated
    // `request.owner`, so an anonymous attacker flooding challenges for a
    // victim's name from one IP exhausted the *victim's* budget and blocked
    // the victim's own login (a challenge is the login prerequisite).
    // Re-keyed to the peer IP, the attacker only spends their own bucket.
    let temp = tempfile::tempdir().unwrap();
    let opened =
        Store::open_repository(&temp.path().join("meta.db"), &temp.path().join("data")).unwrap();
    let store = opened.store;
    let keys = register_owner_with_all_keys(&store, "victim");
    let victim_fingerprint =
        crypto::fingerprint(&crypto::public_from_private(&keys.auth_private).unwrap());
    let state = AppState {
        store: store.clone(),
        blob_store: BlobStore::local(opened.packages_dir.clone()),
        rate_limiter: RateLimiter::new(),
    };
    // Attacker floods challenges naming the victim, well past the cap, all
    // from one IP. A blind attacker need not know the victim's key: the rate
    // gate runs before the fingerprint check, so each attempt still counts.
    for _ in 0..(CHALLENGE_PER_IP_MAX + 5) {
        let _ = challenge(
            State(state.clone()),
            peer("10.9.9.9"),
            Json(ChallengeRequest {
                owner: "victim".to_string(),
                auth_fingerprint: crypto::fingerprint(&register_dummy()),
            }),
        )
        .await;
    }
    // The victim, from their own IP with their real auth fingerprint, must
    // still obtain a challenge — the only thing that could turn this valid
    // request into a 429 is the (owner-keyed) rate limiter this bug is about.
    let victim = challenge(
        State(state.clone()),
        peer("10.1.1.1"),
        Json(ChallengeRequest {
            owner: "victim".to_string(),
            auth_fingerprint: victim_fingerprint,
        }),
    )
    .await;
    assert!(
        victim.is_ok(),
        "victim locked out by an attacker's owner-named flood: {:?}",
        victim.err().map(|(status, _)| status),
    );
}

fn register_dummy() -> Vec<u8> {
    crypto::generate_keypair().0
}

#[tokio::test]
async fn register_rate_limit_is_per_client_ip() {
    // REPO-12 / bug-188: register/login buckets are keyed by peer IP, so one
    // abusive client can no longer lock every user out of registration.
    let temp = tempfile::tempdir().unwrap();
    let opened =
        Store::open_repository(&temp.path().join("meta.db"), &temp.path().join("data")).unwrap();
    let state = AppState {
        store: opened.store,
        blob_store: BlobStore::local(opened.packages_dir.clone()),
        rate_limiter: RateLimiter::new(),
    };
    // Malformed body: the rate gate runs *before* validation, so each attempt
    // still counts, then fails with 400 until the bucket is exhausted.
    let dummy = || RegisterRequest {
        owner: "alice".to_string(),
        auth_key: String::new(),
        ident_key: String::new(),
        proofs: RegisterProofs {
            auth: String::new(),
            ident: String::new(),
        },
    };
    let ip_a: SocketAddr = "127.0.0.1:5000".parse().unwrap();
    let ip_b: SocketAddr = "127.0.0.2:6000".parse().unwrap();
    let mut last = StatusCode::OK;
    for _ in 0..(REGISTER_PER_IP_MAX + 1) {
        last = register(State(state.clone()), ConnectInfo(ip_a), Json(dummy()))
            .await
            .map(|_| StatusCode::OK)
            .unwrap_or_else(|(status, _)| status);
    }
    assert_eq!(
        last,
        StatusCode::TOO_MANY_REQUESTS,
        "IP A must be throttled once past its per-client cap",
    );
    // A different client IP still gets in (its own empty bucket): no global
    // lockout. It fails with 400 on the malformed body, never 429.
    let other = register(State(state.clone()), ConnectInfo(ip_b), Json(dummy()))
        .await
        .map(|_| StatusCode::OK)
        .unwrap_or_else(|(status, _)| status);
    assert_ne!(
        other,
        StatusCode::TOO_MANY_REQUESTS,
        "a different client IP must not be locked out by IP A's abuse",
    );
}

/// plan-61-B §3: exact beats prefix beats substring, and the owner match is
/// the tail of the ranked set.
#[tokio::test]
async fn search_ranks_exact_then_prefix_then_substring() {
    let h = harness();
    seed_packages(
        &h,
        &["alice#sql", "alice#sqlite", "alice#mysqlclient", "bob#tool"],
    );

    let response = search_for(&h, "http://x/search?q=sql").await;
    let idents: Vec<&str> = response
        .results
        .iter()
        .map(|result| result.ident.as_str())
        .collect();
    assert_eq!(
        idents,
        vec!["alice#sql", "alice#sqlite", "alice#mysqlclient"],
        "exact, then prefix, then substring",
    );
    assert_eq!(response.query, "sql");
    assert_eq!(response.total, 3);
    assert_eq!(response.results[0].latest_version.as_deref(), Some("1.0.0"));
    assert_eq!(
        response.results[0].latest_state.as_deref(),
        Some("available")
    );
    // `description` is in the shape from day one and null until plan-61-E.
    assert_eq!(response.results[0].description, None);

    // An owner-name query finds that owner's packages.
    let response = search_for(&h, "http://x/search?q=bob").await;
    assert_eq!(response.results.len(), 1);
    assert_eq!(response.results[0].ident, "bob#tool");
}

/// plan-126-A Phase 3 — the search path had its **own** unfiltered
/// newest-version statement, and `SearchResultRow` carried no state field at
/// all, so a yanked release appeared here as a package's current version
/// with no marker of any kind. This is the surface where the missing filter
/// was not merely cosmetic.
#[tokio::test]
async fn search_names_the_newest_active_release_and_carries_its_state() {
    let h = harness();
    seed_packages(&h, &["alice#toolbox"]);
    let alice_id = h.store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
    // seed_packages published 1.0.0; add a newer 2.0.0 and yank it.
    h.store
        .publish_package_version(
            alice_id,
            "alice#toolbox",
            "2.0.0",
            "hash-2",
            "data/2.mfp",
            "{}",
            &[],
            &crate::store::PublishMetadata::default(),
        )
        .unwrap();
    h.store
        .set_release_state(alice_id, "alice#toolbox", "2.0.0", "yanked")
        .unwrap();

    let response = search_for(&h, "http://x/search?q=toolbox").await;
    assert_eq!(response.results.len(), 1);
    assert_eq!(
        response.results[0].latest_version.as_deref(),
        Some("1.0.0"),
        "a yanked newest release must not be the search headline",
    );
    assert_eq!(
        response.results[0].latest_state.as_deref(),
        Some("available")
    );

    // A `deprecated` newest release IS active — the install client installs
    // it on a floating add — and its state travels so the page can badge it.
    h.store
        .set_release_state(alice_id, "alice#toolbox", "2.0.0", "deprecated")
        .unwrap();
    let response = search_for(&h, "http://x/search?q=toolbox").await;
    assert_eq!(response.results[0].latest_version.as_deref(), Some("2.0.0"));
    assert_eq!(
        response.results[0].latest_state.as_deref(),
        Some("deprecated"),
    );

    // Withdraw everything: the package keeps its result row — it exists and
    // the query found it — but names no version.
    for version in ["1.0.0", "2.0.0"] {
        h.store
            .set_release_state(alice_id, "alice#toolbox", version, "blocked")
            .unwrap();
    }
    let response = search_for(&h, "http://x/search?q=toolbox").await;
    assert_eq!(
        response.results.len(),
        1,
        "a fully-withdrawn package is still a match, not a hidden row",
    );
    assert_eq!(response.results[0].latest_version, None);
    assert_eq!(response.results[0].latest_state, None);
}

/// An empty or whitespace-only query returns nothing — never the whole
/// table. This is an anonymous route; "no filter" must not mean "enumerate
/// the registry".
#[tokio::test]
async fn search_with_an_empty_query_returns_nothing() {
    let h = harness();
    seed_packages(&h, &["alice#sql", "alice#tool"]);

    for uri in [
        "http://x/search",
        "http://x/search?q=",
        "http://x/search?q=%20%20",
    ] {
        let response = search_for(&h, uri).await;
        assert!(
            response.results.is_empty(),
            "{uri} must not enumerate the registry: {:?}",
            response.results,
        );
    }
}

/// plan-61-B Phase 2 acceptance: `?limit` above the cap is **clamped**, not
/// honoured — an uncapped limit on an anonymous enumerate route is a
/// resource-exhaustion lever.
#[tokio::test]
async fn search_clamps_an_over_cap_limit_and_pages_by_offset() {
    let h = harness();
    let idents: Vec<String> = (0..(SEARCH_LIMIT_MAX + 5))
        .map(|n| format!("alice#pkg{n:03}"))
        .collect();
    seed_packages(&h, &idents.iter().map(String::as_str).collect::<Vec<_>>());

    let response = search_for(&h, "http://x/search?q=pkg&limit=100000").await;
    assert_eq!(
        response.results.len() as i64,
        SEARCH_LIMIT_MAX,
        "an over-cap limit is clamped to the cap, not honoured",
    );

    // A negative limit clamps to zero rather than reaching SQLite as -1,
    // which SQLite reads as "no limit".
    let response = search_for(&h, "http://x/search?q=pkg&limit=-1").await;
    assert!(response.results.is_empty());

    // Offset pages within the capped window.
    let first = search_for(&h, "http://x/search?q=pkg&limit=2").await;
    let second = search_for(&h, "http://x/search?q=pkg&limit=2&offset=2").await;
    assert_eq!(first.results.len(), 2);
    assert_eq!(second.results.len(), 2);
    assert_ne!(first.results[0].ident, second.results[0].ident);
}

/// A query full of SQL/LIKE metacharacters must be treated as literal text.
///
/// `%` and `_` are LIKE wildcards: unescaped, `?q=%` would match every
/// package and hand an anonymous caller the whole registry through a route
/// whose empty-query case deliberately returns nothing. `'` and the escape
/// character itself are the injection shapes.
#[tokio::test]
async fn search_treats_like_and_sql_metacharacters_as_literal_text() {
    let h = harness();
    seed_packages(&h, &["alice#sql", "alice#tool", "bob#thing"]);

    for query in ["%", "_", "%%", "'", "' OR 1=1 --", "\\", "%_\\"] {
        let encoded: String = query.bytes().map(|byte| format!("%{byte:02X}")).collect();
        let response = search_for(&h, &format!("http://x/search?q={encoded}")).await;
        assert!(
            response.results.is_empty(),
            "query {query:?} must match literally and return nothing, not \
                 wildcard-match the whole registry: {:?}",
            response.results,
        );
    }

    // A package whose ident genuinely contains an underscore is still
    // findable — escaping must not break literal matching.
    seed_packages(&h, &["alice#with_underscore"]);
    let response = search_for(&h, "http://x/search?q=with_under").await;
    assert_eq!(response.results.len(), 1);
    assert_eq!(response.results[0].ident, "alice#with_underscore");
}

/// The edit-distance-1 fuzzy tail runs only when the ranked query finds
/// nothing — it is an unindexed full table scan (`store.rs`
/// `typosquat_candidates` shape), so it must not sit on the common path.
#[tokio::test]
async fn search_falls_back_to_the_fuzzy_tail_only_when_nothing_matched() {
    let h = harness();
    seed_packages(&h, &["alice#sqlite"]);

    // A typo one edit away from the full ident finds it.
    let response = search_for(&h, "http://x/search?q=alice%23sqlit").await;
    assert_eq!(response.results.len(), 1);
    assert_eq!(response.results[0].ident, "alice#sqlite");

    // Something far away still finds nothing.
    let response = search_for(&h, "http://x/search?q=zzzzzzzz").await;
    assert!(response.results.is_empty());
}

/// `/search` is anonymous but rate-limited per peer IP, following the
/// `REGISTER_PER_IP_MAX` precedent — there is no `claims.sub` to key on.
#[tokio::test]
async fn search_is_rate_limited_per_peer_ip() {
    let h = harness();
    seed_packages(&h, &["alice#sql"]);
    let query =
        || axum::extract::Query::try_from_uri(&"http://x/search?q=sql".parse().unwrap()).unwrap();

    for _ in 0..SEARCH_PER_IP_MAX {
        let _ = search(State(h.state.clone()), peer("198.51.100.7"), query())
            .await
            .expect("within the per-IP window");
    }
    let (status, _message) =
        err_of(search(State(h.state.clone()), peer("198.51.100.7"), query()).await);
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);

    // A different client is unaffected: the bucket is per IP, not global.
    let _ = search(State(h.state.clone()), peer("198.51.100.8"), query())
        .await
        .expect("a different peer has its own bucket");
}

/// Seed a package whose newest version carries `description`.
fn seed_with_description(h: &Harness, ident: &str, description: &str) {
    let owner = ident.split('#').next().unwrap();
    if h.store.owner_with_ident_key(owner).unwrap().is_none() {
        register_owner_with_all_keys(&h.store, owner);
    }
    let owner_id = h.store.owner_with_ident_key(owner).unwrap().unwrap().0.id;
    h.store
        .publish_package_version(
            owner_id,
            ident,
            "1.0.0",
            &format!("hash-{ident}"),
            &format!("data/{ident}.mfp"),
            "{}",
            &[],
            &crate::store::PublishMetadata {
                description: Some(description.to_string()),
                ..Default::default()
            },
        )
        .unwrap();
}

/// plan-61-E Phase 1/3: a description flows database → JSON → HTML, and a
/// package without one stays `null` and renders no stray "None".
#[tokio::test]
async fn a_description_reaches_the_json_and_the_page() {
    let h = harness();
    seed_with_description(&h, "alice#toolbox", "Dense matrix primitives.");
    seed_packages(&h, &["bob#plain"]);

    let detail = package_detail(
        State(h.state.clone()),
        axum::extract::Path("alice#toolbox".to_string()),
    )
    .await
    .expect("detail served")
    .0;
    assert_eq!(
        detail.description.as_deref(),
        Some("Dense matrix primitives.")
    );

    let (_status, _headers, body) = get_page(&h.state, "/p/alice%23toolbox").await;
    assert!(body.contains("Dense matrix primitives."), "{body}");

    // A package with no description: null in JSON, and the page shows the
    // placeholder rather than an empty element or a stray "None".
    let detail = package_detail(
        State(h.state.clone()),
        axum::extract::Path("bob#plain".to_string()),
    )
    .await
    .expect("detail served")
    .0;
    assert_eq!(detail.description, None);
    let (_status, _headers, body) = get_page(&h.state, "/p/bob%23plain").await;
    assert!(body.contains("No description provided."), "{body}");
    assert!(
        !body.contains("None"),
        "no stray Rust Option rendering: {body}"
    );
}

/// plan-61-E Phase 2: a package findable **only** by a word in its
/// description is returned, and an ident match still outranks a description
/// match for the same query.
#[tokio::test]
async fn search_matches_descriptions_but_ranks_them_below_idents() {
    let h = harness();
    seed_with_description(&h, "alice#toolbox", "A library for zygomorphic layouts.");
    seed_packages(&h, &["bob#zygomorphic"]);

    // Findable only by a description word.
    let response = search_for(&h, "http://x/search?q=layouts").await;
    assert_eq!(response.results.len(), 1, "{:?}", response.results);
    assert_eq!(response.results[0].ident, "alice#toolbox");

    // For a term appearing in one package's *ident* and another's
    // *description*, the ident wins.
    let response = search_for(&h, "http://x/search?q=zygomorphic").await;
    let idents: Vec<&str> = response
        .results
        .iter()
        .map(|result| result.ident.as_str())
        .collect();
    assert_eq!(
        idents,
        vec!["bob#zygomorphic", "alice#toolbox"],
        "an ident match must outrank a description match",
    );
}

/// A search result carries a **clamped** description; the package page
/// carries the full text. Without the clamp, one page of 50 results could
/// ship 50 × 4096 bytes on an anonymous route.
#[tokio::test]
async fn search_results_clamp_long_descriptions_on_a_character_boundary() {
    let h = harness();
    // Multi-byte characters, so a byte clamp would split one and produce
    // invalid UTF-8 or a broken glyph.
    let long: String = "é".repeat(SEARCH_DESCRIPTION_PREVIEW_CHARS + 50);
    seed_with_description(&h, "alice#toolbox", &long);

    let response = search_for(&h, "http://x/search?q=toolbox").await;
    let preview = response.results[0].description.as_ref().unwrap();
    assert_eq!(
        preview.chars().count(),
        SEARCH_DESCRIPTION_PREVIEW_CHARS + 1,
        "clamped to the preview length plus the ellipsis",
    );
    assert!(preview.ends_with('…'));
    assert!(preview.starts_with('é'), "no split character: {preview}");

    // The package page still shows the whole thing.
    let detail = package_detail(
        State(h.state.clone()),
        axum::extract::Path("alice#toolbox".to_string()),
    )
    .await
    .expect("detail served")
    .0;
    assert_eq!(detail.description.as_deref(), Some(long.as_str()));

    // A description at or under the limit is returned untouched, with no
    // gratuitous ellipsis.
    let exact: String = "x".repeat(SEARCH_DESCRIPTION_PREVIEW_CHARS);
    seed_with_description(&h, "alice#exact", &exact);
    let response = search_for(&h, "http://x/search?q=exact").await;
    assert_eq!(
        response.results[0].description.as_deref(),
        Some(exact.as_str())
    );
}

/// bug-579: the anonymous log routes now consume a shared per-IP budget.
#[tokio::test]
async fn anonymous_log_routes_are_rate_limited_per_ip() {
    let h = harness();
    for _ in 0..LOG_PER_IP_MAX {
        let _ = log_checkpoint(State(h.state.clone()), peer("10.0.0.2"))
            .await
            .expect("within budget");
    }
    let (status, _) = err_of(log_checkpoint(State(h.state.clone()), peer("10.0.0.2")).await);
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);

    // The budget is shared across the log routes, so an exhausted peer
    // cannot simply switch to the proof route for more full-log work.
    let (status, _) = err_of(
        log_consistency_proof(
            State(h.state.clone()),
            peer("10.0.0.2"),
            axum::extract::Query(ConsistencyQuery { from: 0, to: None }),
        )
        .await,
    );
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);

    // It is PER IP: one abusive peer must not deny the rest of the world.
    let _ = log_checkpoint(State(h.state.clone()), peer("10.0.0.3"))
        .await
        .expect("a different peer has its own budget");
}

/// POSITIVE (bug-579): the budget must clear the worst LEGITIMATE client.
///
/// `verify_publish_inclusion` makes three log requests — `/log/checkpoint`,
/// `/log/publish`, `/log/proof/:index` — and `mfb pkg install --proof` calls
/// it once per dependency, so a large install is a single burst of roughly
/// `3 x deps` requests, all from one IP. Behind a NAT or a CI egress address
/// that IP is shared by everyone. A budget tuned for "abuse" without this
/// number would not harden the registry, it would break installs.
///
/// This pins the headroom explicitly so that anyone tightening
/// `LOG_PER_IP_MAX` has to confront the client's real request shape.
#[tokio::test]
async fn the_log_budget_clears_a_full_install_burst_from_one_ip() {
    let h = harness();
    const DEPENDENCIES: usize = 200;
    const REQUESTS_PER_DEPENDENCY: usize = 3;
    let burst = DEPENDENCIES * REQUESTS_PER_DEPENDENCY;
    assert!(
        burst <= LOG_PER_IP_MAX,
        "a {DEPENDENCIES}-dependency `pkg install --proof` is {burst} log \
             requests from one IP; LOG_PER_IP_MAX is {LOG_PER_IP_MAX}"
    );
    for _ in 0..burst {
        let _ = log_checkpoint(State(h.state.clone()), peer("10.0.0.4"))
            .await
            .expect("an ordinary install burst must not be throttled");
    }
}

#[tokio::test]
async fn login_is_throttled_per_client_ip() {
    let h = harness();
    // The gate runs before signature decoding, so malformed attempts still
    // spend the abuser's own bucket — and only their own.
    let attempt = |ip: &'static str| {
        login(
            State(h.state.clone()),
            peer(ip),
            Json(LoginRequest {
                challenge_id: "no-such-challenge".to_string(),
                signature: NOT_BASE64.to_string(),
            }),
        )
    };
    let mut last = StatusCode::OK;
    for _ in 0..(LOGIN_PER_IP_MAX + 1) {
        last = err_of(attempt("10.1.1.1").await).0;
    }
    assert_eq!(last, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(err_of(attempt("10.1.1.2").await).0, StatusCode::BAD_REQUEST);
}

#[test]
fn rate_limiter_prune_drops_only_fully_elapsed_windows() {
    let limiter = RateLimiter::new();
    assert!(limiter.allow("key", 1, 60));
    assert!(!limiter.allow("key", 1, 60), "the cap is one per window");

    // A prune with a window the entry still falls inside keeps counting it.
    limiter.prune(3600);
    assert!(!limiter.allow("key", 1, 60));

    // A prune whose window has fully elapsed drops the key entirely.
    limiter.prune(0);
    assert!(
        limiter.allow("key", 1, 60),
        "the elapsed window was released"
    );
}
