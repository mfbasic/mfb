//! The read-only browse routes (`server/browse.rs`) against a registry that is
//! busy, damaged, or holds rows no current store method writes: each route must
//! answer with a named error or a degraded-but-true view, never a panic and
//! never a silently wrong page.

use super::pages::toolbox_doc_section;
use super::*;
use crate::store::tests::{drop_tables, tamper};

/// The peer `get_page` sends every request from.
const PAGE_PEER: &str = "203.0.113.1";

/// Spend `peer`'s whole budget in one rate-limit bucket.
fn exhaust(h: &Harness, bucket: &str, max: usize) {
    for _ in 0..max {
        assert!(h.state.rate_limiter.allow(bucket, max, 60));
    }
}

fn error_message(body: &str) -> String {
    serde_json::from_str::<ErrorResponse>(body)
        .unwrap_or_else(|err| panic!("an error body ({err}): {body}"))
        .error
}

/// Rotate `owner`'s ident key through the store, returning the new public key.
fn rotate_owner_ident(h: &Harness, owner: &str, keys: &TestOwnerKeys) -> Vec<u8> {
    let (new_public, new_private) = crypto::generate_keypair();
    let chain = crypto::sign(
        &keys.ident_private,
        &crypto::ident_rotation_message(
            owner,
            &crypto::fingerprint(&keys.ident_public),
            &new_public,
        ),
    )
    .unwrap();
    let possession = crypto::sign(
        &new_private,
        &crypto::registration_message(crypto::ROLE_IDENT, owner, &new_public),
    )
    .unwrap();
    h.store
        .rotate_ident(owner, &new_public, &chain, &possession)
        .unwrap();
    new_public
}

#[tokio::test]
async fn the_search_page_refuses_a_client_past_its_search_budget() {
    let h = harness();
    exhaust(&h, &format!("search:{PAGE_PEER}"), SEARCH_PER_IP_MAX);
    let (status, _headers, body) = get_page(&h.state, "/search.html?q=toolbox").await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert!(body.contains("Too many searches"), "{body}");
    assert!(
        body.contains("made too many searches in a short window"),
        "{body}"
    );
}

#[tokio::test]
async fn the_search_page_reports_a_failed_search_rather_than_no_results() {
    let h = harness();
    seed_packages(&h, &["alice#toolbox"]);
    drop_tables(&h.store, &["packages"]);
    let (status, _headers, body) = get_page(&h.state, "/search.html?q=toolbox").await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(body.contains("Search failed"), "{body}");
    assert!(
        body.contains("The registry could not complete this search."),
        "{body}"
    );
    // The page never claims the search came back empty.
    assert!(!body.contains("No packages"), "{body}");
}

#[tokio::test]
async fn undecodable_stored_documentation_is_an_error_on_both_docs_surfaces() {
    let h = harness();
    seed_toolbox(&h, Some(toolbox_doc_section()));
    tamper(
        &h.store,
        "UPDATE package_version_docs SET doc_section = x'00'",
    );

    let (status, _headers, body) = get_page(&h.state, "/p/alice%23toolbox/docs").await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(body.contains("Documentation unavailable"), "{body}");
    assert!(
        body.contains("The stored documentation for this release could not be decoded."),
        "{body}"
    );

    let (status, _headers, body) = get_page(&h.state, "/packages/alice%23toolbox/docs").await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        error_message(&body),
        "The stored documentation for this release could not be decoded."
    );
}

#[tokio::test]
async fn an_unreadable_documentation_table_is_an_error_not_an_undocumented_release() {
    let h = harness();
    seed_toolbox(&h, Some(toolbox_doc_section()));
    drop_tables(&h.store, &["package_version_docs"]);
    let (status, _headers, body) = get_page(&h.state, "/p/alice%23toolbox/docs").await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        body.contains("The registry could not load this package&#39;s documentation.")
            || body.contains("The registry could not load this package's documentation."),
        "{body}"
    );
    let (status, _headers, body) = get_page(&h.state, "/packages/alice%23toolbox/docs").await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        error_message(&body),
        "The registry could not load this package's documentation."
    );
}

#[tokio::test]
async fn the_audit_surfaces_list_every_ident_rotation_of_the_owner() {
    let h = harness();
    let alice = register_owner_with_all_keys(&h.store, "alice");
    let alice_id = h.store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
    h.store
        .publish_package_version(
            alice_id,
            "alice#toolbox",
            "1.0.0",
            "hash-1.0.0",
            "data/1.0.0.mfp",
            "{}",
            &[],
            &crate::store::PublishMetadata::default(),
        )
        .unwrap();
    let new_public = rotate_owner_ident(&h, "alice", &alice);

    let (status, _headers, body) = get_page(&h.state, "/packages/alice%23toolbox/audit").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let audit: PackageAuditResponse = serde_json::from_str(&body).unwrap();
    assert_eq!(audit.ident_chain.len(), 1);
    let rotation = &audit.ident_chain[0];
    assert_eq!(
        rotation.old_key,
        format!("ed25519:{}", crypto::encode_bytes(&alice.ident_public))
    );
    assert_eq!(
        rotation.new_key,
        format!("ed25519:{}", crypto::encode_bytes(&new_public))
    );
    // The served link verifies: it is the one the old key signed.
    crypto::verify(
        &alice.ident_public,
        &crypto::ident_rotation_message(
            "alice",
            &crypto::fingerprint(&alice.ident_public),
            &new_public,
        ),
        &crypto::decode_bytes(&rotation.signature, "signature").unwrap(),
    )
    .unwrap();

    let (status, _headers, body) = get_page(&h.state, "/p/alice%23toolbox/audit").await;
    assert_eq!(status, StatusCode::OK);
    assert!(!body.contains("No rotations recorded"), "{body}");
    assert!(body.contains("data-label=\"old key\""), "{body}");
}

#[tokio::test]
async fn the_audit_route_refuses_a_client_past_its_log_budget() {
    let h = harness();
    seed_packages(&h, &["alice#toolbox"]);
    exhaust(&h, &format!("log:{PAGE_PEER}"), LOG_PER_IP_MAX);
    let (status, _headers, body) = get_page(&h.state, "/packages/alice%23toolbox/audit").await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(error_message(&body), too_many_requests().1 .0.error);
}

#[tokio::test]
async fn the_audit_omits_a_release_with_no_publish_entry_and_a_proof_past_the_log() {
    let h = harness();
    seed_packages(&h, &["alice#toolbox"]);
    let alice_id = h.store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
    h.store
        .publish_package_version(
            alice_id,
            "alice#toolbox",
            "2.0.0",
            "hash-2.0.0",
            "data/2.0.0.mfp",
            "{}",
            &[],
            &crate::store::PublishMetadata::default(),
        )
        .unwrap();
    // 1.0.0's publish entry is gone (a release recorded before the log), and
    // 2.0.0's names an index past the end of the log.
    tamper(
        &h.store,
        "DELETE FROM log_entries WHERE kind = 'publish' AND payload LIKE '%\"1.0.0\"%';
         UPDATE log_entries SET idx = 1000 WHERE kind = 'publish' AND payload LIKE '%\"2.0.0\"%';",
    );
    let (status, _headers, body) = get_page(&h.state, "/packages/alice%23toolbox/audit").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let audit: PackageAuditResponse = serde_json::from_str(&body).unwrap();
    let publishes: Vec<_> = audit
        .publishes
        .iter()
        .map(|entry| (entry.version.as_str(), entry.index, entry.proof.len()))
        .collect();
    // No proof can be built for an entry outside the checkpoint's leaves.
    assert_eq!(publishes, vec![("2.0.0", 1000, 0)]);
}

#[tokio::test]
async fn a_stored_abi_index_that_is_not_json_is_served_as_an_empty_index() {
    let h = harness();
    register_owner_with_all_keys(&h.store, "alice");
    let alice_id = h.store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
    h.store
        .publish_package_version(
            alice_id,
            "alice#toolbox",
            "1.0.0",
            "hash-1.0.0",
            "data/1.0.0.mfp",
            "not json",
            &[],
            &crate::store::PublishMetadata::default(),
        )
        .unwrap();
    let detail = package_detail(
        State(h.state.clone()),
        axum::extract::Path("alice#toolbox".to_string()),
    )
    .await
    .expect("detail served")
    .0;
    assert_eq!(detail.versions[0].abi_index, serde_json::json!({}));
    let index = package_index(
        State(h.state.clone()),
        axum::extract::Path("alice#toolbox".to_string()),
    )
    .await
    .expect("index served")
    .0;
    assert_eq!(index.versions[0].abi_index, serde_json::json!({}));
}

#[tokio::test]
async fn a_package_with_no_versions_has_no_headline_metadata() {
    let h = harness();
    seed_packages(&h, &["alice#toolbox"]);
    tamper(&h.store, "DELETE FROM package_versions");
    let detail = package_detail(
        State(h.state.clone()),
        axum::extract::Path("alice#toolbox".to_string()),
    )
    .await
    .expect("detail served")
    .0;
    assert!(detail.versions.is_empty());
    assert_eq!(
        (detail.author, detail.url, detail.description),
        (None, None, None)
    );
    assert_eq!(detail.latest_version, None);
}

#[tokio::test]
async fn the_index_refuses_a_release_with_a_negative_epoch() {
    let h = harness();
    seed_packages(&h, &["alice#toolbox"]);
    tamper(&h.store, "UPDATE package_versions SET transfer_epoch = -1");
    let (status, message) = err_of(
        package_index(
            State(h.state.clone()),
            axum::extract::Path("alice#toolbox".to_string()),
        )
        .await,
    );
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(message, "release 1.0.0 has a negative epoch");
}

#[tokio::test]
async fn an_owner_without_a_current_ident_key_cannot_be_served_or_bound() {
    let h = harness();
    seed_packages(&h, &["alice#toolbox"]);
    register_owner_with_all_keys(&h.store, "bob");
    crate::store::tests::legacy_transfer(&h.store, "alice#toolbox", "alice", "bob");
    h.store.migrate().unwrap();
    tamper(
        &h.store,
        "UPDATE keys SET status = 'past'
         WHERE role = 'ident'
           AND owner_id = (SELECT id FROM owners WHERE owner_folded = 'bob')",
    );

    // The detail names the stored owner, bob, and has no key to show for him.
    let (status, message) = err_of(
        package_detail(
            State(h.state.clone()),
            axum::extract::Path("alice#toolbox".to_string()),
        )
        .await,
    );
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(message, "unknown package");

    // The index cannot mint a binding for the transfer's recipient.
    let (status, message) = err_of(
        package_index(
            State(h.state.clone()),
            axum::extract::Path("alice#toolbox".to_string()),
        )
        .await,
    );
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(message, "transfer recipient bob has no current ident key");
}
