//! `store/schema.rs` — `migrate` against databases it cannot upgrade: one opened
//! read-only that still lacks a column or a table, and a pre-plan transparency
//! log the legacy-transfer replay cannot follow. `migrate` must refuse by name
//! and leave no half-replayed history behind.

use super::*;

fn read_only(store: &Store) {
    store
        .conn()
        .pragma_update(None, "query_only", true)
        .unwrap();
}

#[test]
fn a_read_only_database_missing_a_column_names_the_table_it_cannot_upgrade() {
    for (table, column) in [
        ("package_versions", "abi_index"),
        ("package_blobs", "kind"),
        ("auth_challenges", "purpose"),
        ("pairing_blobs", "approval_key"),
        ("package_versions", "publisher_ident_key_id"),
        ("package_versions", "transfer_epoch"),
    ] {
        let (_temp, store) = test_store();
        tamper(&store, &format!("ALTER TABLE {table} DROP COLUMN {column}"));
        read_only(&store);
        assert_eq!(
            store.migrate().unwrap_err(),
            format!("failed to add column to {table}: attempt to write a readonly database"),
            "{column}"
        );
    }
}

#[test]
fn a_read_only_database_missing_a_table_cannot_be_migrated() {
    let (_temp, store) = test_store();
    drop_tables(&store, &["registry_config"]);
    read_only(&store);
    assert_eq!(
        store.migrate().unwrap_err(),
        "failed to migrate database: attempt to write a readonly database"
    );
}

#[test]
fn a_read_only_database_with_unrecorded_publishers_cannot_be_migrated() {
    let (_temp, store) = test_store();
    register_keys(&store, "alice");
    publish_as(&store, "alice", "alice#toolbox", "1.0.0", None).unwrap();
    tamper(
        &store,
        "UPDATE package_versions SET publisher_owner_id = NULL",
    );
    read_only(&store);
    assert_eq!(
        store.migrate().unwrap_err(),
        "failed to backfill release publishers: attempt to write a readonly database"
    );
}

/// alice published `alice#toolbox@1.0.0` and handed it to bob before signed
/// transfers existed; carol is a bystander.
fn legacy_store() -> (tempfile::TempDir, Store) {
    let (temp, store) = test_store();
    register_keys(&store, "alice");
    register_keys(&store, "bob");
    register_keys(&store, "carol");
    publish_as(&store, "alice", "alice#toolbox", "1.0.0", None).unwrap();
    legacy_transfer(&store, "alice#toolbox", "alice", "bob");
    (temp, store)
}

fn history_len(store: &Store) -> usize {
    store.transfer_history("alice#toolbox").unwrap().len()
}

#[test]
fn a_legacy_replay_that_fails_at_any_write_records_no_history() {
    for (table, op, expected) in [
        (
            "transfer_history",
            "INSERT",
            "failed to record legacy transfer: injected INSERT failure",
        ),
        (
            "package_versions",
            "UPDATE",
            "failed to record legacy release epoch: injected UPDATE failure",
        ),
    ] {
        let (_temp, store) = legacy_store();
        refuse_writes(&store, table, op);
        assert_eq!(store.migrate().unwrap_err(), expected);
        assert_eq!(history_len(&store), 0, "{table}");
    }

    let (_temp, store) = legacy_store();
    refuse_commit_after(&store, "transfer_history", "INSERT");
    assert_eq!(
        store.migrate().unwrap_err(),
        "failed to commit legacy transfer replay: FOREIGN KEY constraint failed"
    );
    assert_eq!(history_len(&store), 0);

    let (_temp, store) = legacy_store();
    wedge_transaction(&store);
    let err = store.migrate().unwrap_err();
    assert!(
        err.starts_with("failed to start legacy transfer replay: "),
        "{err}"
    );
}

#[test]
fn a_legacy_log_naming_owners_the_registry_does_not_know_is_refused() {
    // The ident's prefix owner is gone.
    let (_temp, store) = legacy_store();
    tamper(
        &store,
        "UPDATE owners SET owner_folded = 'alice-gone' WHERE owner_folded = 'alice'",
    );
    assert_eq!(
        store.migrate().unwrap_err(),
        "transfer log for alice#toolbox starts at unknown owner alice"
    );

    // An accept names a recipient that never registered.
    let (_temp, store) = test_store();
    register_keys(&store, "alice");
    publish_as(&store, "alice", "alice#toolbox", "1.0.0", None).unwrap();
    legacy_log(
        &store,
        "transfer-accept",
        serde_json::json!({"ident": "alice#toolbox", "to": "zed"}),
    );
    assert_eq!(
        store.migrate().unwrap_err(),
        "transfer log for alice#toolbox names unknown owner zed"
    );

    // The package row names an owner id with no account behind it.
    let (_temp, store) = legacy_store();
    tamper(&store, "UPDATE packages SET owner_id = 999");
    assert_eq!(
        store.migrate().unwrap_err(),
        "transfer log for alice#toolbox ends at bob but the package is owned by unknown owner \
         id 999"
    );
    assert_eq!(history_len(&store), 0);
}

#[test]
fn an_unreadable_owner_row_stops_the_legacy_replay() {
    // The prefix owner's row cannot be read.
    let (_temp, store) = legacy_store();
    tamper(
        &store,
        "UPDATE owners SET owner_display = x'00' WHERE owner_folded = 'alice'",
    );
    let err = store.migrate().unwrap_err();
    assert!(err.starts_with("failed to load owner: "), "{err}");

    // The stored owner's row cannot be read once the walk disagrees with it.
    let (_temp, store) = legacy_store();
    tamper(
        &store,
        "UPDATE packages SET owner_id = (SELECT id FROM owners WHERE owner_folded = 'carol');
         UPDATE owners SET owner_display = x'00' WHERE owner_folded = 'carol';",
    );
    let err = store.migrate().unwrap_err();
    assert!(err.starts_with("failed to load owner: "), "{err}");
}

#[test]
fn a_damaged_legacy_log_entry_is_refused_by_index() {
    let accept_idx = |store: &Store| -> i64 {
        store
            .conn()
            .query_row(
                "SELECT idx FROM log_entries WHERE kind = 'transfer-accept'",
                [],
                |row| row.get(0),
            )
            .unwrap()
    };

    let (_temp, store) = legacy_store();
    tamper(
        &store,
        "UPDATE log_entries SET payload = 'not json' WHERE kind = 'transfer-accept'",
    );
    let err = store.migrate().unwrap_err();
    assert!(
        err.starts_with(&format!(
            "malformed transfer-accept log payload at index {}: ",
            accept_idx(&store)
        )),
        "{err}"
    );

    let (_temp, store) = legacy_store();
    tamper(
        &store,
        "UPDATE log_entries SET payload = '{\"ident\":\"alice#toolbox\"}'
         WHERE kind = 'transfer-accept'",
    );
    assert_eq!(
        store.migrate().unwrap_err(),
        format!(
            "transfer-accept log payload at index {} has no to",
            accept_idx(&store)
        )
    );

    let (_temp, store) = legacy_store();
    tamper(
        &store,
        "UPDATE log_entries SET payload = x'00' WHERE kind = 'transfer-accept'",
    );
    let err = store.migrate().unwrap_err();
    assert!(err.starts_with("failed to read log entry: "), "{err}");
}

#[test]
fn an_unreadable_package_row_stops_the_legacy_scan() {
    let (_temp, store) = legacy_store();
    tamper(&store, "UPDATE packages SET owner_id = 'bob'");
    let err = store.migrate().unwrap_err();
    assert!(
        err.starts_with("failed to read package for legacy transfers: "),
        "{err}"
    );
}

/// `CREATE TABLE IF NOT EXISTS` passes over a VIEW of the same name, so a
/// database where a table was replaced by a narrower view survives the schema
/// batch; the legacy scans must then refuse it rather than skip the replay.
#[test]
fn a_table_replaced_by_a_narrower_view_stops_the_legacy_scans() {
    let (_temp, store) = legacy_store();
    tamper(
        &store,
        "ALTER TABLE packages RENAME TO packages_old;
         CREATE VIEW packages AS SELECT id, owner_id FROM packages_old;",
    );
    let err = store.migrate().unwrap_err();
    assert!(
        err.starts_with("failed to scan release publishers: no such column: p.ident"),
        "{err}"
    );

    let (_temp, store) = legacy_store();
    tamper(
        &store,
        "ALTER TABLE packages RENAME TO packages_old;
         CREATE VIEW packages AS SELECT id, ident FROM packages_old;",
    );
    let err = store.migrate().unwrap_err();
    assert!(
        err.starts_with("failed to prepare legacy transfer scan: no such column: p.owner_id"),
        "{err}"
    );

    let (_temp, store) = legacy_store();
    tamper(
        &store,
        "ALTER TABLE log_entries RENAME TO log_entries_old;
         CREATE VIEW log_entries AS SELECT idx, kind, created_at FROM log_entries_old;",
    );
    let err = store.migrate().unwrap_err();
    assert!(
        err.starts_with("failed to prepare legacy log scan: no such column: payload"),
        "{err}"
    );
    assert_eq!(history_len(&store), 0);
}
