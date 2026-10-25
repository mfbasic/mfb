//! `store/versions.rs` against a database that refuses: a connection that
//! cannot begin a transaction, a statement a trigger aborts, a commit a
//! deferred constraint fails, and rows of the wrong type. Every write must
//! report the step that failed and leave nothing half-written; every read must
//! report instead of returning a short or empty answer.

use super::*;

fn vendor(hash: &str) -> VendorBlobRef {
    VendorBlobRef {
        logical: "snd".to_string(),
        source: "snd-glibc.a".to_string(),
        hash: hash.to_string(),
        os: "linux".to_string(),
        arch: Some("x86_64".to_string()),
        libc: Some("glibc".to_string()),
        lib_type: "vendor".to_string(),
    }
}

fn publish(
    store: &Store,
    owner: i64,
    version: &str,
    vendors: &[VendorBlobRef],
    metadata: &PublishMetadata,
) -> Result<PublishedVersion, String> {
    store.publish_package_version(
        owner,
        "alice#toolbox",
        version,
        &format!("hash-{version}"),
        &format!("data/{version}.mfp"),
        "{}",
        vendors,
        metadata,
    )
}

/// A store with alice registered and the `h-snd` native blob uploaded;
/// returns her owner id.
fn alice_store() -> (tempfile::TempDir, Store, i64) {
    let (temp, store) = test_store();
    register_keys(&store, "alice");
    store.record_native_blob("h-snd", "data/h-snd").unwrap();
    let owner = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
    (temp, store, owner)
}

fn versions_of(store: &Store) -> Vec<String> {
    store
        .list_package_versions("alice#toolbox")
        .unwrap()
        .into_iter()
        .map(|row| row.version)
        .collect()
}

#[test]
fn every_publish_step_that_fails_aborts_the_whole_publish() {
    let docs = PublishMetadata {
        docs: Some(vec![1, 2, 3]),
        ..Default::default()
    };
    let cases: [(&str, &str, &[VendorBlobRef], &PublishMetadata, &str); 4] = [
        (
            "package_blobs",
            "INSERT",
            &[],
            &PublishMetadata::default(),
            "failed to store package blob metadata: injected INSERT failure",
        ),
        (
            "package_version_docs",
            "INSERT",
            &[],
            &docs,
            "failed to record version documentation: injected INSERT failure",
        ),
        (
            "package_version_blobs",
            "INSERT",
            &[vendor("h-snd")][..],
            &PublishMetadata::default(),
            "failed to record version blob edge: injected INSERT failure",
        ),
        (
            "package_version_targets",
            "INSERT",
            &[vendor("h-snd")][..],
            &PublishMetadata::default(),
            "failed to record version target: injected INSERT failure",
        ),
    ];
    for (table, op, vendors, metadata, expected) in cases {
        let (_temp, store, owner) = alice_store();
        refuse_writes(&store, table, op);
        let err = publish(&store, owner, "1.0.0", vendors, metadata).unwrap_err();
        assert_eq!(err, expected);
        assert!(versions_of(&store).is_empty(), "{table}: nothing published");
    }
}

#[test]
fn a_publish_that_cannot_begin_or_commit_publishes_nothing() {
    let (_temp, store, owner) = alice_store();
    wedge_transaction(&store);
    let err = publish(&store, owner, "1.0.0", &[], &PublishMetadata::default()).unwrap_err();
    assert!(
        err.starts_with("failed to start publish transaction: "),
        "{err}"
    );

    let (_temp, store, owner) = alice_store();
    refuse_commit_after(&store, "package_versions", "INSERT");
    let err = publish(&store, owner, "1.0.0", &[], &PublishMetadata::default()).unwrap_err();
    assert_eq!(
        err,
        "failed to commit publish: FOREIGN KEY constraint failed"
    );
    assert!(versions_of(&store).is_empty());
    assert_eq!(store.log_size().unwrap(), 1, "only alice's registration");
}

#[test]
fn a_publish_that_cannot_read_the_publishers_keys_is_refused() {
    let (_temp, store, owner) = alice_store();
    let fingerprint = store
        .owner_with_ident_key("alice")
        .unwrap()
        .unwrap()
        .1
        .fingerprint;
    drop_tables(&store, &["keys"]);
    let err = publish(
        &store,
        owner,
        "1.0.0",
        &[],
        &PublishMetadata {
            publisher_ident_fingerprint: Some(fingerprint),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert_eq!(
        err,
        "failed to load publisher ident key: no such table: keys"
    );
}

#[test]
fn a_release_state_change_that_fails_at_any_step_changes_nothing() {
    let (_temp, store, owner) = alice_store();
    publish(&store, owner, "1.0.0", &[], &PublishMetadata::default()).unwrap();
    let state_of = |store: &Store| {
        store.list_package_versions("alice#toolbox").unwrap()[0]
            .state
            .clone()
    };

    refuse_writes(&store, "package_versions", "UPDATE");
    assert_eq!(
        store
            .set_release_state(owner, "alice#toolbox", "1.0.0", "yanked")
            .unwrap_err(),
        "failed to update release state: injected UPDATE failure"
    );
    tamper(&store, "DROP TRIGGER refuse_UPDATE_package_versions;");

    refuse_writes(&store, "log_entries", "INSERT");
    assert_eq!(
        store
            .set_release_state(owner, "alice#toolbox", "1.0.0", "yanked")
            .unwrap_err(),
        "failed to append log entry: injected INSERT failure"
    );
    tamper(&store, "DROP TRIGGER refuse_INSERT_log_entries;");

    refuse_commit_after(&store, "release_state_changes", "INSERT");
    assert_eq!(
        store
            .set_release_state(owner, "alice#toolbox", "1.0.0", "yanked")
            .unwrap_err(),
        "failed to commit release-state change: FOREIGN KEY constraint failed"
    );
    assert_eq!(state_of(&store), "available");

    wedge_transaction(&store);
    let err = store
        .set_release_state(owner, "alice#toolbox", "1.0.0", "yanked")
        .unwrap_err();
    assert!(
        err.starts_with("failed to start release-state transaction: "),
        "{err}"
    );
}

#[test]
fn a_metadata_backfill_that_fails_at_any_step_keeps_the_old_metadata() {
    let (_temp, store, owner) = alice_store();
    let original = PublishMetadata {
        author: Some("Alice".to_string()),
        ..Default::default()
    };
    publish(&store, owner, "1.0.0", &[vendor("h-snd")], &original).unwrap();
    let (id, _, _, _) = store.all_package_versions().unwrap()[0].clone();
    let rewrite = PublishMetadata {
        author: Some("Mallory".to_string()),
        ..Default::default()
    };
    let author = |store: &Store| store.version_metadata_for_test("alice#toolbox", "1.0.0").0;

    for (table, op, expected) in [
        (
            "package_versions",
            "UPDATE",
            "failed to update version metadata: injected UPDATE failure",
        ),
        (
            "package_version_targets",
            "DELETE",
            "failed to clear version targets: injected DELETE failure",
        ),
    ] {
        refuse_writes(&store, table, op);
        assert_eq!(
            store
                .replace_version_metadata(id, &rewrite, &[])
                .unwrap_err(),
            expected
        );
        tamper(&store, &format!("DROP TRIGGER refuse_{op}_{table};"));
    }

    refuse_commit_after(&store, "package_versions", "UPDATE");
    assert_eq!(
        store
            .replace_version_metadata(id, &rewrite, &[])
            .unwrap_err(),
        "failed to commit backfill: FOREIGN KEY constraint failed"
    );
    assert_eq!(author(&store).as_deref(), Some("Alice"));
    assert_eq!(store.target_rows_for_test().len(), 1, "targets kept");

    wedge_transaction(&store);
    let err = store
        .replace_version_metadata(id, &rewrite, &[])
        .unwrap_err();
    assert!(
        err.starts_with("failed to start backfill transaction: "),
        "{err}"
    );
}

#[test]
fn a_version_row_of_the_wrong_type_is_a_read_error_not_a_short_list() {
    let (_temp, store, owner) = alice_store();
    publish(&store, owner, "1.0.0", &[], &PublishMetadata::default()).unwrap();
    publish(&store, owner, "1.1.0", &[], &PublishMetadata::default()).unwrap();
    tamper(
        &store,
        "UPDATE package_versions SET hash = x'00' WHERE version = '1.1.0'",
    );
    let err = store.all_package_versions().unwrap_err();
    assert!(err.starts_with("failed to read package version: "), "{err}");
    let err = store.list_package_versions("alice#toolbox").unwrap_err();
    assert!(err.starts_with("failed to read package version: "), "{err}");
}

#[test]
fn version_reads_report_a_missing_versions_table() {
    let (_temp, store, owner) = alice_store();
    publish(&store, owner, "1.0.0", &[], &PublishMetadata::default()).unwrap();
    drop_tables(&store, &["package_version_docs", "package_versions"]);
    assert_eq!(
        store.all_package_versions().unwrap_err(),
        "failed to enumerate package versions: no such table: package_versions"
    );
    assert_eq!(
        store.latest_active_version("alice#toolbox").unwrap_err(),
        "failed to load latest active version: no such table: package_versions"
    );
}

#[test]
fn recording_documentation_reports_a_refused_insert() {
    let (_temp, store, owner) = alice_store();
    publish(&store, owner, "1.0.0", &[], &PublishMetadata::default()).unwrap();
    let (id, _, _, _) = store.all_package_versions().unwrap()[0].clone();
    refuse_writes(&store, "package_version_docs", "INSERT");
    assert_eq!(
        store.put_version_docs(id, &[1, 2, 3]).unwrap_err(),
        "failed to record version documentation: injected INSERT failure"
    );
}

#[test]
fn a_version_listing_whose_ident_cannot_be_bound_is_an_error_not_an_empty_list() {
    let (_temp, store, _owner) = alice_store();
    limit_value_length(&store, 32);
    let ident = format!("alice#{}", "x".repeat(64));
    let err = store.list_package_versions(&ident).unwrap_err();
    assert!(
        err.starts_with("failed to list package versions: "),
        "{err}"
    );
    assert!(err.contains("too big"), "{err}");
}
