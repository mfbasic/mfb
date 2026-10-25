//! `store/browse.rs` — the read models behind the package page, the audit tab,
//! search and the typosquat check, against a damaged database: a missing
//! table, a table replaced by a narrower view, a row of the wrong type, and a
//! query too long to bind. Each must be an error, never a page that silently
//! shows fewer versions, targets, state changes or results than exist.

use super::*;

/// alice's `alice#toolbox@1.0.0` with one native target.
fn toolbox_store() -> (tempfile::TempDir, Store) {
    let (temp, store) = test_store();
    register_keys(&store, "alice");
    let owner = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
    store.record_native_blob("h-snd", "data/h-snd").unwrap();
    store
        .publish_package_version(
            owner,
            "alice#toolbox",
            "1.0.0",
            "hash-1.0.0",
            "data/1.0.0.mfp",
            "{}",
            &[VendorBlobRef {
                logical: "snd".to_string(),
                source: "snd-glibc.a".to_string(),
                hash: "h-snd".to_string(),
                os: "linux".to_string(),
                arch: Some("x86_64".to_string()),
                libc: Some("glibc".to_string()),
                lib_type: "vendor".to_string(),
            }],
            &PublishMetadata::default(),
        )
        .unwrap();
    (temp, store)
}

fn starts(err: String, prefix: &str) {
    assert!(err.starts_with(prefix), "expected `{prefix}…`, got: {err}");
}

#[test]
fn the_package_detail_reports_every_table_it_cannot_read() {
    let (_temp, store) = toolbox_store();
    drop_tables(&store, &["package_version_targets"]);
    starts(
        store.package_detail("alice#toolbox").unwrap_err(),
        "failed to prepare target query: no such table: package_version_targets",
    );
    drop_tables(
        &store,
        &[
            "package_version_docs",
            "package_version_blobs",
            "release_state_changes",
            "package_versions",
        ],
    );
    starts(
        store.package_detail("alice#toolbox").unwrap_err(),
        "failed to prepare detail query: no such table: package_versions",
    );
    drop_tables(&store, &["transfer_offers", "transfer_history", "packages"]);
    starts(
        store.package_detail("alice#toolbox").unwrap_err(),
        "failed to prepare detail query: no such table: packages",
    );
}

#[test]
fn the_package_detail_reports_a_version_or_target_row_it_cannot_read() {
    let (_temp, store) = toolbox_store();
    tamper(&store, "UPDATE package_version_targets SET os = x'00'");
    starts(
        store.package_detail("alice#toolbox").unwrap_err(),
        "failed to read version target: ",
    );
    tamper(&store, "UPDATE package_versions SET hash = x'00'");
    starts(
        store.package_detail("alice#toolbox").unwrap_err(),
        "failed to read package detail: ",
    );
}

#[test]
fn the_package_audit_reports_every_table_it_cannot_read() {
    let (_temp, store) = toolbox_store();
    tamper(
        &store,
        "ALTER TABLE package_versions RENAME TO package_versions_old;
         CREATE VIEW package_versions AS
             SELECT id, package_id, version FROM package_versions_old;",
    );
    starts(
        store.package_audit("alice#toolbox").unwrap_err(),
        "failed to prepare version query: no such column: pv.created_at",
    );

    let (_temp, store) = toolbox_store();
    drop_tables(&store, &["release_state_changes"]);
    starts(
        store.package_audit("alice#toolbox").unwrap_err(),
        "failed to prepare state-change query: no such table: release_state_changes",
    );

    let (_temp, store) = toolbox_store();
    drop_tables(&store, &["packages"]);
    starts(
        store.package_audit("alice#toolbox").unwrap_err(),
        "failed to prepare version query: no such table: packages",
    );
}

#[test]
fn the_package_audit_reports_a_state_change_or_version_row_it_cannot_read() {
    let (_temp, store) = toolbox_store();
    tamper(&store, "UPDATE package_versions SET version = x'00'");
    starts(
        store.package_audit("alice#toolbox").unwrap_err(),
        "failed to read package version: ",
    );

    let (_temp, store) = toolbox_store();
    let owner = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
    store
        .set_release_state(owner, "alice#toolbox", "1.0.0", "yanked")
        .unwrap();
    tamper(&store, "UPDATE release_state_changes SET state = x'00'");
    starts(
        store.package_audit("alice#toolbox").unwrap_err(),
        "failed to read state change: ",
    );
}

#[test]
fn a_one_typo_query_finds_the_package_through_the_fuzzy_tail() {
    let (_temp, store) = toolbox_store();
    let results = store.search_packages("alice#toolbx", 10, 0).unwrap();
    let found: Vec<_> = results
        .iter()
        .map(|row| (row.ident.as_str(), row.owner.as_str()))
        .collect();
    assert_eq!(found, vec![("alice#toolbox", "alice")]);
    assert_eq!(results[0].latest_version.as_deref(), Some("1.0.0"));
}

#[test]
fn search_reports_a_query_it_cannot_bind_and_rows_it_cannot_read() {
    let (_temp, store) = toolbox_store();
    limit_value_length(&store, 32);
    starts(
        store.search_packages(&"q".repeat(64), 10, 0).unwrap_err(),
        "failed to search packages: ",
    );

    // A matching row whose owner cannot be read.
    let (_temp, store) = toolbox_store();
    tamper(&store, "UPDATE owners SET owner_display = x'00'");
    starts(
        store.search_packages("toolbox", 10, 0).unwrap_err(),
        "failed to read search result: ",
    );
    // The same row met by the fuzzy tail.
    starts(
        store.search_packages("alice#toolbx", 10, 0).unwrap_err(),
        "failed to read package: ",
    );

    // A match whose newest release cannot be read.
    let (_temp, store) = toolbox_store();
    tamper(&store, "UPDATE package_versions SET version = x'00'");
    starts(
        store.search_packages("toolbox", 10, 0).unwrap_err(),
        "failed to read latest version: ",
    );

    // Versions without a state column: the latest-release query cannot run.
    let (_temp, store) = toolbox_store();
    tamper(
        &store,
        "ALTER TABLE package_versions RENAME TO package_versions_old;
         CREATE VIEW package_versions AS
             SELECT id, package_id, version, created_at, description
             FROM package_versions_old;",
    );
    starts(
        store.search_packages("toolbox", 10, 0).unwrap_err(),
        "failed to prepare latest-version query: no such column: pv.state",
    );
}

#[test]
fn the_typosquat_check_reports_an_ident_it_cannot_read() {
    let (_temp, store) = toolbox_store();
    tamper(&store, "UPDATE packages SET ident = x'00'");
    starts(
        store.typosquat_candidates("alice#toolbax").unwrap_err(),
        "failed to read package ident: ",
    );
}

/// The read models' answer shapes: an unknown ident is `None`; a package with
/// no versions still has its owner and empty lists; versions come newest
/// first on the detail and oldest first on the audit, with the STORED owner.
#[test]
fn package_detail_and_audit_answer_for_unknown_empty_and_published_packages() {
    let (_temp, store) = toolbox_store();
    assert!(store.package_detail("alice#nothing").unwrap().is_none());
    assert!(store.package_audit("alice#nothing").unwrap().is_none());

    let owner = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
    store
        .publish_package_version(
            owner,
            "alice#toolbox",
            "1.1.0",
            "hash-1.1.0",
            "data/1.1.0.mfp",
            "{}",
            &[],
            &PublishMetadata::default(),
        )
        .unwrap();
    let detail = store.package_detail("alice#toolbox").unwrap().unwrap();
    assert_eq!(detail.owner, "alice");
    let versions: Vec<_> = detail.versions.iter().map(|v| v.version.as_str()).collect();
    assert_eq!(versions, vec!["1.1.0", "1.0.0"]);
    assert_eq!(detail.versions[1].targets.len(), 1);
    assert!(detail.versions[0].targets.is_empty());
    let audit = store.package_audit("alice#toolbox").unwrap().unwrap();
    assert_eq!(audit.owner, "alice");
    assert_eq!(audit.versions, vec!["1.0.0", "1.1.0"]);

    tamper(
        &store,
        "DELETE FROM package_version_targets; DELETE FROM package_version_blobs;
         DELETE FROM package_versions;",
    );
    let detail = store.package_detail("alice#toolbox").unwrap().unwrap();
    assert_eq!(detail.owner, "alice");
    assert!(detail.versions.is_empty());
    let audit = store.package_audit("alice#toolbox").unwrap().unwrap();
    assert_eq!(audit.owner, "alice");
    assert!(audit.versions.is_empty() && audit.state_changes.is_empty());
}

/// The one statement that binds the ident on each read model reports a
/// parameter it cannot bind rather than answering "no such package".
#[test]
fn package_detail_and_audit_report_an_ident_they_cannot_bind() {
    let (_temp, store) = toolbox_store();
    limit_value_length(&store, 32);
    let ident = format!("alice#{}", "x".repeat(64));
    let err = store.package_detail(&ident).unwrap_err();
    starts(err.clone(), "failed to load package detail: ");
    assert!(err.contains("too big"), "{err}");
    let err = store.package_audit(&ident).unwrap_err();
    starts(err.clone(), "failed to list package versions: ");
    assert!(err.contains("too big"), "{err}");
}
