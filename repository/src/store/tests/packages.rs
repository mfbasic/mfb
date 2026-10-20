//! plan-176-I: `store::tests` — the packages tests. The shared store harness and the helpers other\n//! hosts import (`register_keys`, `legacy_transfer`, `drop_tables`) live in `store/tests/mod.rs`.

use super::*;

#[test]
fn publish_log_lookup_does_not_cross_match_like_wildcards() {
    // REPO-14: an owner name may contain `_`, a SQL LIKE wildcard. The
    // publish-log lookup must match ident/version literally — not let `_`
    // (or `%`) resolve to a *different* package's entry, which would corrupt
    // the inclusion-proof mapping a client verifies.
    let (_temp, store) = test_store();
    register_keys(&store, "axb");
    let axb_id = store.owner_with_ident_key("axb").unwrap().unwrap().0.id;
    store
        .publish_package_version(
            axb_id,
            "axb#pkg",
            "1.0.0",
            "hash",
            "path",
            "{}",
            &[],
            &PublishMetadata::default(),
        )
        .unwrap();

    // The real entry resolves.
    assert!(store
        .publish_log_entry("axb#pkg", "1.0.0")
        .unwrap()
        .is_some());
    // A distinct ident that only matches under LIKE-wildcard semantics must
    // NOT resolve to axb's entry.
    assert!(store
        .publish_log_entry("a_b#pkg", "1.0.0")
        .unwrap()
        .is_none());
    assert!(store
        .publish_log_entry("axb#pkg", "1.0.%")
        .unwrap()
        .is_none());
}

/// bug-420 item 3: the anonymous fuzzy-search tail must only pull
/// length-compatible rows out of SQL, not the whole `packages` table.
/// `within_edit_distance_one` can never match when the ident length differs
/// by more than one, so scanning every row (and allocating a `Vec<char>`
/// per row) on a no-match query is wasted O(n) work an anonymous client can
/// force. Without the SQL length bound this returns every package.
#[test]
fn fuzzy_search_only_scans_length_compatible_candidates() {
    let (_temp, store) = test_store();
    register_keys(&store, "alice");
    let alice_id = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
    // Idents of deliberately varied lengths (8, 12, and 27 chars).
    for ident in ["alice#db", "alice#sqlite", "alice#reallylongpackagename"] {
        store
            .publish_package_version(
                alice_id,
                ident,
                "1.0.0",
                "hash",
                "path",
                "{}",
                &[],
                &PublishMetadata::default(),
            )
            .unwrap();
    }

    // A 12-char query (one edit from "alice#sqlite") must only surface
    // candidates within one character of that length.
    let query = "alice#sqlitx";
    let query_len = query.chars().count();
    let candidates = fuzzy_search_candidates(&store.conn(), query_len).unwrap();
    let returned: Vec<&str> = candidates.iter().map(|(i, _)| i.as_str()).collect();

    assert!(
        returned.contains(&"alice#sqlite"),
        "the length-compatible candidate must be present: {returned:?}"
    );
    assert!(
        !returned.contains(&"alice#db"),
        "a far-shorter ident must not be pulled into the scan: {returned:?}"
    );
    assert!(
        !returned.contains(&"alice#reallylongpackagename"),
        "a far-longer ident must not be pulled into the scan: {returned:?}"
    );
    for (ident, _) in &candidates {
        let len = ident.chars().count();
        assert!(
            (query_len - 1..=query_len + 1).contains(&len),
            "candidate {ident:?} (len {len}) is outside the edit-distance-1 length window"
        );
    }
}

#[test]
fn publish_rejects_unsafe_package_and_version() {
    // REPO-17: the ident's package component and the version are restricted to
    // a safe charset before they can reach the log payload / index / LIKE
    // pattern. Control chars, quotes, `#`, `%`, `/`, and whitespace are out.
    let (_temp, store) = test_store();
    register_keys(&store, "alice");
    let id = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
    for ident in [
        "alice#pk g",
        "alice#pk\"g",
        "alice#pk%g",
        "alice#pk/g",
        "alice#pk\ng",
        "alice#",
        "no-hash",
    ] {
        assert!(
            store
                .publish_package_version(
                    id,
                    ident,
                    "1.0.0",
                    "h",
                    "p",
                    "{}",
                    &[],
                    &PublishMetadata::default()
                )
                .is_err(),
            "{ident} should be rejected"
        );
    }
    for version in ["1.0 0", "1.0\"0", "1.0%0", "1/0", "1.0\n0", ""] {
        assert!(
            store
                .publish_package_version(
                    id,
                    "alice#pkg",
                    version,
                    "h",
                    "p",
                    "{}",
                    &[],
                    &PublishMetadata::default()
                )
                .is_err(),
            "{version} should be rejected"
        );
    }
    // A clean publish still works.
    assert!(store
        .publish_package_version(
            id,
            "alice#pkg",
            "1.0.0",
            "h",
            "p",
            "{}",
            &[],
            &PublishMetadata::default()
        )
        .is_ok());
}

#[test]
fn every_state_change_appends_exactly_one_log_entry() {
    let (_temp, store) = test_store();
    assert_eq!(store.log_size().unwrap(), 0);

    // register
    let keys = register_keys(&store, "alice");
    assert_eq!(store.log_size().unwrap(), 1);
    let owner_id = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;

    // attestation request
    store
        .record_signing_request(owner_id, "alice#toolbox", "1.0.0", "fp")
        .unwrap();
    assert_eq!(store.log_size().unwrap(), 2);

    // publish
    store
        .publish_package_version(
            owner_id,
            "alice#toolbox",
            "1.0.0",
            "hash",
            "path",
            "{}",
            &[],
            &PublishMetadata::default(),
        )
        .unwrap();
    assert_eq!(store.log_size().unwrap(), 3);

    // machine link
    let (machine_public, machine_private) = crypto::generate_keypair();
    let proof = crypto::sign(
        &machine_private,
        &crypto::registration_message(crypto::ROLE_AUTH, "alice", &machine_public),
    )
    .unwrap();
    let (_owner, machine_key) = store
        .add_auth_key("alice", &machine_public, &proof)
        .unwrap();
    assert_eq!(store.log_size().unwrap(), 4);

    // auth revoke
    assert!(store
        .revoke_auth_key(owner_id, &machine_key.fingerprint)
        .unwrap());
    assert_eq!(store.log_size().unwrap(), 5);

    // ident rotation
    let (new_public, new_private) = crypto::generate_keypair();
    let chain_signature = crypto::sign(
        &keys.ident_private,
        &crypto::ident_rotation_message(
            "alice",
            &crypto::fingerprint(&keys.ident_public),
            &new_public,
        ),
    )
    .unwrap();
    let possession_proof = crypto::sign(
        &new_private,
        &crypto::registration_message(crypto::ROLE_IDENT, "alice", &new_public),
    )
    .unwrap();
    store
        .rotate_ident("alice", &new_public, &chain_signature, &possession_proof)
        .unwrap();
    assert_eq!(store.log_size().unwrap(), 6);

    // re-anchor
    let (anchor_public, _anchor_private) = crypto::generate_keypair();
    store.reanchor_ident("alice", &anchor_public).unwrap();
    assert_eq!(store.log_size().unwrap(), 7);

    // The publish entry is findable and the leaves form a stable tree.
    let entry = store
        .publish_log_entry("alice#toolbox", "1.0.0")
        .unwrap()
        .expect("publish entry recorded");
    assert_eq!(entry.index, 2);
    let leaves = store.log_leaf_hashes(None).unwrap();
    assert_eq!(leaves.len(), 7);
    let root = crate::log::root(&leaves);
    let path = crate::log::inclusion_path(2, &leaves);
    crate::log::verify_inclusion(2, 7, &entry.leaf_hash, &path, &root)
        .expect("publish entry inclusion verifies");
}

#[test]
fn typosquat_and_reaping_hardening() {
    let (_temp, store) = test_store();
    let keys = register_keys(&store, "alice");
    let owner_id = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
    store
        .publish_package_version(
            owner_id,
            "alice#toolbox",
            "1.0.0",
            "hash",
            "path",
            "{}",
            &[],
            &PublishMetadata::default(),
        )
        .unwrap();

    // A one-edit-away ident is flagged; an exact match and a far ident are not.
    assert_eq!(
        store.typosquat_candidates("alice#toolbx").unwrap(),
        vec!["alice#toolbox".to_string()]
    );
    assert!(store
        .typosquat_candidates("alice#toolbox")
        .unwrap()
        .is_empty());
    assert!(store
        .typosquat_candidates("alice#unrelated")
        .unwrap()
        .is_empty());

    // Reaping drops expired challenges.
    let challenge = store.create_challenge("alice").unwrap();
    store.force_expire_challenge(&challenge.id).unwrap();
    // Also expire a session so reaping closes it.
    store
        .insert_session(&NewSession {
            owner_id,
            key_id: store.owner_with_auth_key("alice").unwrap().unwrap().1.id,
            jwt_id: "expired-sess".to_string(),
            issued_at: now_unix() - 7200,
            expires_at: now_unix() - 3600,
        })
        .unwrap();
    assert!(store.session_exists("expired-sess").unwrap());
    let reaped = store.reap_expired().unwrap();
    assert!(reaped >= 2, "reaped {reaped}");
    assert!(!store.session_exists("expired-sess").unwrap());
    let _ = keys;
}

#[test]
fn edit_distance_one_covers_insert_delete_substitute() {
    assert!(within_edit_distance_one("toolbox", "toolbox")); // equal (0)
    assert!(within_edit_distance_one("toolbox", "toolbux")); // substitution
    assert!(within_edit_distance_one("toolbox", "toolbx")); // deletion
    assert!(within_edit_distance_one("toolbox", "toolboxs")); // insertion
    assert!(!within_edit_distance_one("toolbox", "tulbox")); // two edits
    assert!(!within_edit_distance_one("toolbox", "widget")); // far
}

/// bug-579: `signed_checkpoint` memoises the signed tree head by log size,
/// so the anonymous routes stop loading every leaf and re-hashing the whole
/// Merkle tree on every request.
///
/// Observing a cache is harder than it looks: Ed25519 signing is
/// deterministic (RFC 8032), so "the same bytes came back" is equally true
/// of a full recompute and proves nothing. The discriminator used here is a
/// leaf corruption applied IN PLACE, which leaves the row count — and so the
/// memo key — unchanged. A recompute surfaces `malformed log leaf hash`; a
/// cache hit returns the head it already holds.
///
/// That is a PROBE of the mechanism, not a supported state. It is also the
/// soundness argument written as a test: size is a valid key precisely
/// because production only ever appends, and `log_leaf_hashes` is
/// deliberately left un-memoised so it still reports the corruption — see
/// `log_readers_reject_a_malformed_leaf_hash`.
#[test]
fn signed_checkpoint_is_memoised_by_log_size() {
    let (_temp, store) = test_store();
    let keys = register_keys(&store, "alice");
    let owner_id = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;

    let (size, root, signature) = store.signed_checkpoint().unwrap();
    assert_eq!(size, store.log_size().unwrap());

    {
        let conn = store.conn();
        conn.execute("UPDATE log_entries SET leaf_hash = x'0011'", [])
            .unwrap();
    }
    assert!(
        store.log_leaf_hashes(None).is_err(),
        "the un-memoised reader must still see the corruption"
    );
    let (cached_size, cached_root, cached_signature) = store.signed_checkpoint().unwrap();
    assert_eq!(cached_size, size, "the memo answered");
    assert_eq!(cached_root, root);
    assert_eq!(cached_signature, signature);

    // An append changes the size, so the key misses and the head is rebuilt
    // — which, over the corrupted leaves, now correctly ERRORS rather than
    // serving a stale root under a new size. Masking that would be the
    // dangerous failure.
    store
        .publish_package_version(
            owner_id,
            "alice#toolbox",
            "1.0.0",
            "hash",
            "path",
            "{}",
            &[],
            &PublishMetadata::default(),
        )
        .unwrap();
    assert!(
        store.signed_checkpoint().is_err(),
        "a size change must re-read, never serve the previous root"
    );
    let _ = keys;
}

/// A negative log size is refused rather than silently selecting zero leaves
/// (bug-276 R6).
///
/// `WHERE idx < ?1` with a negative bound returns nothing, and
/// `log_consistency_proof` would then compute `to = 0` and produce a
/// structurally valid empty-range proof for a log that is not empty. The
/// handlers take this straight off an unvalidated query parameter.
#[test]
fn a_negative_log_size_is_rejected() {
    let (_temp, store) = test_store();
    register_keys(&store, "alice");

    assert!(store
        .log_leaf_hashes(Some(-1))
        .unwrap_err()
        .contains("must not be negative"));
    // Zero and None remain meaningful: an empty prefix and "everything".
    assert!(store.log_leaf_hashes(Some(0)).unwrap().is_empty());
    assert!(!store.log_leaf_hashes(None).unwrap().is_empty());
}

#[test]
fn publish_records_publisher_and_epoch() {
    let (_temp, store) = test_store();
    let alice = register_keys(&store, "alice");
    let bob = register_keys(&store, "bob");
    let alice_fp = crypto::fingerprint(&alice.ident_public);
    let bob_fp = crypto::fingerprint(&bob.ident_public);
    publish_as(
        &store,
        "alice",
        "alice#toolbox",
        "1.0.0",
        Some(alice_fp.clone()),
    )
    .unwrap();
    offer_and_accept_for_test(&store, "alice#toolbox", "alice", &alice, "bob", &bob);
    publish_as(
        &store,
        "bob",
        "alice#toolbox",
        "2.0.0",
        Some(bob_fp.clone()),
    )
    .unwrap();
    // A fingerprint that is not one of the publisher's keys refuses the
    // publish: bob cannot record alice's key as his.
    let err = publish_as(
        &store,
        "bob",
        "alice#toolbox",
        "3.0.0",
        Some(alice_fp.clone()),
    )
    .unwrap_err();
    assert!(err.contains("not one of the publisher's keys"), "{err}");

    let versions = store.list_package_versions("alice#toolbox").unwrap();
    let summary: Vec<_> = versions
        .iter()
        .map(|row| {
            (
                row.version.as_str(),
                row.publisher_display.as_deref(),
                row.publisher_ident_fingerprint.as_deref(),
                row.transfer_epoch,
            )
        })
        .collect();
    assert_eq!(
        summary,
        vec![
            ("1.0.0", Some("alice"), Some(alice_fp.as_str()), 0),
            ("2.0.0", Some("bob"), Some(bob_fp.as_str()), 1),
        ]
    );
}

// === plan-126-A: latest-active-version selection ======================
//
// Publish two versions and move the newer one's state, then assert which
// version the selection names. `publish_package_version` stamps both rows
// with the same `created_at` second, so the tie resolves on `pv.id DESC` —
// the later-inserted row wins, which is what makes "newest" meaningful in a
// test that runs in milliseconds.

/// Publish `versions` in order under `alice#toolbox` and return the store.
fn store_with_versions(versions: &[(&str, &str)]) -> (tempfile::TempDir, Store) {
    let (temp, store) = test_store();
    register_keys(&store, "alice");
    let owner_id = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
    for (version, state) in versions {
        store
            .publish_package_version(
                owner_id,
                "alice#toolbox",
                version,
                &format!("hash-{version}"),
                &format!("path-{version}"),
                "{}",
                &[],
                &PublishMetadata::default(),
            )
            .unwrap();
        if *state != "available" {
            store
                .set_release_state(owner_id, "alice#toolbox", version, state)
                .unwrap();
        }
    }
    (temp, store)
}

/// The bug plan-126-A fixes: the newest release is yanked, so the headline
/// version must be the older active one — not the yanked newest.
#[test]
fn latest_active_version_skips_a_yanked_newest_release() {
    let (_temp, store) = store_with_versions(&[("1.5.0", "available"), ("2.0.0", "yanked")]);
    assert_eq!(
        store.latest_active_version("alice#toolbox").unwrap(),
        Some(("1.5.0".to_string(), "available".to_string())),
    );
    // The transparency listing is untouched: both versions are still there.
    let versions = store.list_package_versions("alice#toolbox").unwrap();
    assert_eq!(versions.len(), 2);
}

/// `blocked` is operator-set and never reaches the maintainer route, so a
/// denylist written as `state != 'yanked'` would pass the test above and
/// fail this one. That is the whole reason the predicate is an allowlist.
#[test]
fn latest_active_version_skips_a_blocked_newest_release() {
    let (_temp, store) = store_with_versions(&[("1.5.0", "available"), ("2.0.0", "blocked")]);
    assert_eq!(
        store.latest_active_version("alice#toolbox").unwrap(),
        Some(("1.5.0".to_string(), "available".to_string())),
    );
}

/// The second operator-only state, for the same reason.
#[test]
fn latest_active_version_skips_a_legal_tombstoned_newest_release() {
    let (_temp, store) =
        store_with_versions(&[("1.5.0", "available"), ("2.0.0", "legal-tombstoned")]);
    assert_eq!(
        store.latest_active_version("alice#toolbox").unwrap(),
        Some(("1.5.0".to_string(), "available".to_string())),
    );
}

/// `deprecated` **is** active — the install client installs it on a floating
/// add — and the state travels with the version so the page can badge it.
#[test]
fn latest_active_version_returns_a_deprecated_release_with_its_state() {
    let (_temp, store) = store_with_versions(&[("1.5.0", "available"), ("2.0.0", "deprecated")]);
    assert_eq!(
        store.latest_active_version("alice#toolbox").unwrap(),
        Some(("2.0.0".to_string(), "deprecated".to_string())),
    );
}

/// Every published version inactive: `None` means "no active release", a
/// statement the page renders — not "this package does not exist".
#[test]
fn latest_active_version_is_none_when_every_version_is_inactive() {
    let (_temp, store) = store_with_versions(&[("1.5.0", "yanked"), ("2.0.0", "blocked")]);
    assert_eq!(store.latest_active_version("alice#toolbox").unwrap(), None);
    // The versions themselves are still listed.
    assert_eq!(
        store.list_package_versions("alice#toolbox").unwrap().len(),
        2
    );
}

/// No versions at all, and an ident no package carries: both `None`, and
/// neither is an error.
#[test]
fn latest_active_version_is_none_for_no_versions_and_unknown_idents() {
    let (_temp, store) = store_with_versions(&[]);
    assert_eq!(store.latest_active_version("alice#toolbox").unwrap(), None);
    assert_eq!(store.latest_active_version("alice#nope").unwrap(), None);
    assert_eq!(store.latest_active_version("not-an-ident").unwrap(), None);
}

// === plan-126-E: per-version documentation storage =====================

/// The `package_versions.id` of `alice#toolbox@version`.
fn version_id(store: &Store, version: &str) -> i64 {
    store
        .all_package_versions()
        .unwrap()
        .into_iter()
        .find(|(_, ident, v, _)| ident == "alice#toolbox" && v == version)
        .map(|(id, ..)| id)
        .expect("version is published")
}

/// A stored section reads back byte for byte -- including zero bytes and
/// values above 0x7F, which a text column would mangle.
#[test]
fn version_docs_round_trip_byte_for_byte() {
    let (_temp, store) = store_with_versions(&[("1.0.0", "available")]);
    let section = vec![0x01, 0x00, 0xFF, 0x7F, 0x00, 0x42];
    store
        .put_version_docs(version_id(&store, "1.0.0"), &section)
        .unwrap();
    assert_eq!(
        store.latest_active_version_docs("alice#toolbox").unwrap(),
        Some(("1.0.0".to_string(), section)),
    );
}

/// No docs row is `None`, and so is an ident no package carries.
#[test]
fn a_version_with_no_docs_row_yields_none() {
    let (_temp, store) = store_with_versions(&[("1.0.0", "available")]);
    assert_eq!(
        store.latest_active_version_docs("alice#toolbox").unwrap(),
        None
    );
    assert_eq!(
        store.latest_active_version_docs("alice#nope").unwrap(),
        None
    );
}

/// **plan-126-E Phase 1's acceptance test.** The newest release is yanked
/// and both releases carry docs. The accessor must return the older active
/// release's docs -- not the yanked newest's, and not `None`. An accessor
/// written as its own `ORDER BY created_at LIMIT 1` would return 2.0.0's, so
/// this is what proves it is built on the plan-126-A selection.
#[test]
fn a_yanked_newest_release_falls_back_to_the_older_active_releases_docs() {
    let (_temp, store) = store_with_versions(&[("1.5.0", "available"), ("2.0.0", "yanked")]);
    store
        .put_version_docs(version_id(&store, "1.5.0"), b"docs-1.5.0")
        .unwrap();
    store
        .put_version_docs(version_id(&store, "2.0.0"), b"docs-2.0.0")
        .unwrap();
    assert_eq!(
        store.latest_active_version_docs("alice#toolbox").unwrap(),
        Some(("1.5.0".to_string(), b"docs-1.5.0".to_vec())),
    );
}

/// The contract plan-126-F's Docs tab relies on: an undocumented *current*
/// release reads as undocumented. It never borrows an older release's docs,
/// which would describe a version the tab is not showing.
#[test]
fn an_undocumented_latest_release_never_borrows_an_older_releases_docs() {
    let (_temp, store) = store_with_versions(&[("1.0.0", "available"), ("2.0.0", "available")]);
    store
        .put_version_docs(version_id(&store, "1.0.0"), b"docs-1.0.0")
        .unwrap();
    assert_eq!(
        store.latest_active_version_docs("alice#toolbox").unwrap(),
        None
    );
}

/// `INSERT OR IGNORE`: a second write for the same version keeps the first.
/// The backfill sweep relies on this to stay idempotent.
#[test]
fn putting_version_docs_twice_keeps_the_first_row() {
    let (_temp, store) = store_with_versions(&[("1.0.0", "available")]);
    let id = version_id(&store, "1.0.0");
    store.put_version_docs(id, b"first").unwrap();
    store.put_version_docs(id, b"second").unwrap();
    assert_eq!(
        store.latest_active_version_docs("alice#toolbox").unwrap(),
        Some(("1.0.0".to_string(), b"first".to_vec())),
    );
}

#[test]
fn set_release_state_rejects_unpublished_and_updates_published() {
    let (_temp, store) = test_store();
    let keys = register_keys(&store, "alice");
    register_keys(&store, "bob");
    let owner_id = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
    let bob_id = store.owner_with_ident_key("bob").unwrap().unwrap().0.id;
    assert!(store
        .set_release_state(owner_id, "alice#toolbox", "1.0.0", "yanked")
        .unwrap_err()
        .contains("is not published"));
    store
        .publish_package_version(
            owner_id,
            "alice#toolbox",
            "1.0.0",
            "hash",
            "path",
            "{}",
            &[],
            &PublishMetadata::default(),
        )
        .unwrap();
    store
        .set_release_state(owner_id, "alice#toolbox", "1.0.0", "deprecated")
        .unwrap();
    let versions = store.list_package_versions("alice#toolbox").unwrap();
    assert_eq!(versions[0].state, "deprecated");
    // bug-494: the row is resolved under the caller's owner id, so an
    // account that does not own the package cannot move its state — the
    // store is safe on its own, independent of the handler's check.
    let log_before = store.log_size().unwrap();
    assert!(store
        .set_release_state(bob_id, "alice#toolbox", "1.0.0", "yanked")
        .unwrap_err()
        .contains("is not published by this account"));
    assert_eq!(
        store.list_package_versions("alice#toolbox").unwrap()[0].state,
        "deprecated"
    );
    assert_eq!(store.log_size().unwrap(), log_before);
    let _ = keys;
}

#[test]
fn index_canonical_hash_changes_with_index_state() {
    let (_temp, store) = test_store();
    let keys = register_keys(&store, "alice");
    let owner_id = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
    let empty = store.index_canonical_hash().unwrap();
    store
        .publish_package_version(
            owner_id,
            "alice#toolbox",
            "1.0.0",
            "hash",
            "path",
            "{}",
            &[],
            &PublishMetadata::default(),
        )
        .unwrap();
    let with_pkg = store.index_canonical_hash().unwrap();
    assert_ne!(empty, with_pkg);
    // Deterministic for the same state.
    assert_eq!(with_pkg, store.index_canonical_hash().unwrap());
    let _ = keys;
}

#[test]
fn package_version_exists_reflects_publishes() {
    let (_temp, store) = test_store();
    register_keys(&store, "alice");
    let owner_id = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
    assert!(!store
        .package_version_exists("alice#toolbox", "1.0.0")
        .unwrap());
    store
        .publish_package_version(
            owner_id,
            "alice#toolbox",
            "1.0.0",
            "hash",
            "path",
            "{}",
            &[],
            &PublishMetadata::default(),
        )
        .unwrap();
    assert!(store
        .package_version_exists("alice#toolbox", "1.0.0")
        .unwrap());
    // A duplicate publish is rejected.
    assert!(store
        .publish_package_version(
            owner_id,
            "alice#toolbox",
            "1.0.0",
            "hash",
            "path",
            "{}",
            &[],
            &PublishMetadata::default()
        )
        .unwrap_err()
        .contains("already published"));
    // publish_log_entry finds the publish; a missing one is None.
    assert!(store
        .publish_log_entry("alice#toolbox", "1.0.0")
        .unwrap()
        .is_some());
    assert!(store
        .publish_log_entry("alice#toolbox", "9.9.9")
        .unwrap()
        .is_none());
}

#[test]
fn owner_version_count_totals_all_versions_across_packages() {
    // bug-188 / REPO-13: the per-owner publish quota counts every
    // package_versions row an owner owns, across all their packages.
    let (_temp, store) = test_store();
    register_keys(&store, "alice");
    register_keys(&store, "bob");
    let alice = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
    let bob = store.owner_with_ident_key("bob").unwrap().unwrap().0.id;
    assert_eq!(store.owner_version_count(alice).unwrap(), 0);
    store
        .publish_package_version(
            alice,
            "alice#toolbox",
            "1.0.0",
            "h1",
            "p1",
            "{}",
            &[],
            &PublishMetadata::default(),
        )
        .unwrap();
    store
        .publish_package_version(
            alice,
            "alice#toolbox",
            "1.1.0",
            "h2",
            "p2",
            "{}",
            &[],
            &PublishMetadata::default(),
        )
        .unwrap();
    store
        .publish_package_version(
            alice,
            "alice#widgets",
            "0.1.0",
            "h3",
            "p3",
            "{}",
            &[],
            &PublishMetadata::default(),
        )
        .unwrap();
    store
        .publish_package_version(
            bob,
            "bob#thing",
            "1.0.0",
            "h4",
            "p4",
            "{}",
            &[],
            &PublishMetadata::default(),
        )
        .unwrap();
    // Three versions across two of alice's packages; bob's row is not counted.
    assert_eq!(store.owner_version_count(alice).unwrap(), 3);
    assert_eq!(store.owner_version_count(bob).unwrap(), 1);
}

#[test]
fn publishing_into_another_accounts_package_ident_is_refused() {
    // `packages.ident` is UNIQUE, so the `INSERT OR IGNORE` silently does
    // nothing when the ident already exists under a different account. The
    // owner-scoped re-read is what turns that into a refusal instead of
    // letting bob append a version to alice's package.
    let (_temp, store) = test_store();
    register_keys(&store, "alice");
    register_keys(&store, "bob");
    let alice = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
    let bob = store.owner_with_ident_key("bob").unwrap().unwrap().0.id;
    store
        .publish_package_version(
            alice,
            "alice#toolbox",
            "1.0.0",
            "h1",
            "p1",
            "{}",
            &[],
            &PublishMetadata::default(),
        )
        .unwrap();

    let err = store
        .publish_package_version(
            bob,
            "alice#toolbox",
            "2.0.0",
            "evil",
            "p2",
            "{}",
            &[],
            &PublishMetadata::default(),
        )
        .unwrap_err();
    assert!(
        err.contains("owned by another owner"),
        "bob must not publish into alice's ident: {err}"
    );
    // The refusal rolled back: alice's package is untouched and bob's
    // version never landed.
    let versions = store.list_package_versions("alice#toolbox").unwrap();
    assert_eq!(versions.len(), 1);
    assert_eq!(versions[0].hash, "h1");
    assert_eq!(
        store
            .package_owner("alice#toolbox")
            .unwrap()
            .unwrap()
            .owner_display,
        "alice"
    );
}

// --- Corrupt transparency-log rows --------------------------------------

#[test]
fn a_malformed_log_leaf_hash_is_rejected_rather_than_truncated() {
    // A leaf hash is a fixed 32 bytes. A short/long blob in that column would
    // otherwise be silently zero-padded into a `[u8; 32]`, producing a
    // well-formed-looking inclusion/consistency proof over a hash the log
    // never actually committed to.
    let (_temp, store) = test_store();
    register_keys(&store, "alice");
    let owner = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
    store
        .publish_package_version(
            owner,
            "alice#toolbox",
            "1.0.0",
            "h1",
            "p1",
            "{}",
            &[],
            &PublishMetadata::default(),
        )
        .unwrap();
    // Sound to begin with.
    assert_eq!(store.log_leaf_hashes(None).unwrap().len(), 2);
    assert!(store
        .publish_log_entry("alice#toolbox", "1.0.0")
        .unwrap()
        .is_some());

    {
        let conn = store.conn();
        conn.execute("UPDATE log_entries SET leaf_hash = x'0011'", [])
            .unwrap();
    }
    assert_eq!(
        store.log_leaf_hashes(None).unwrap_err(),
        "malformed log leaf hash"
    );
    assert_eq!(
        store
            .publish_log_entry("alice#toolbox", "1.0.0")
            .unwrap_err(),
        "malformed log leaf hash"
    );
}
