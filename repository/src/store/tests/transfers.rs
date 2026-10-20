//! plan-176-I: `store::tests` — the transfers tests. The shared store harness and the helpers other\n//! hosts import (`register_keys`, `legacy_transfer`, `drop_tables`) live in `store/tests/mod.rs`.

use super::*;

/// A transfer offer is only acceptable while the account that made it still
/// owns the package (bug-274).
///
/// `create_transfer_offer` authorized under one lock acquisition and wrote
/// under another, and its UPSERT resets `accepted_at = NULL` unconditionally.
/// Since axum handlers run concurrently over one `Arc<Mutex<Connection>>`, an
/// in-flight offer could commit *after* an accept and re-list itself as
/// pending under the previous owner's stale authority.
///
/// Rather than race two threads and hope to hit the window, this writes the
/// exact row state that interleaving produces — a pending offer whose
/// `from_owner_id` is no longer the owner — and asserts it cannot be
/// accepted. That is the outcome the race was able to reach, tested
/// deterministically.
#[test]
fn a_stale_offer_cannot_rebind_a_package_after_ownership_moved() {
    let (_temp, store) = test_store();
    let alice = register_keys(&store, "alice");
    let bob = register_keys(&store, "bob");
    let carol = register_keys(&store, "carol");
    let alice_id = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
    store
        .publish_package_version(
            alice_id,
            "alice#toolbox",
            "1.0.0",
            "hash",
            "path",
            "{}",
            &[],
            &PublishMetadata::default(),
        )
        .unwrap();

    // Ownership legitimately moves alice -> bob.
    offer_and_accept_for_test(&store, "alice#toolbox", "alice", &alice, "bob", &bob);

    // Reproduce the resurrection: alice's offer row is re-listed as pending
    // (to carol) with `from_owner_id` still alice, even though bob now owns
    // the package. This is what the losing side of the race committed.
    // Resolve carol's id *before* taking the connection: `conn()` holds a
    // non-reentrant mutex, and `owner_with_ident_key` acquires it too.
    let carol_id = store.owner_with_ident_key("carol").unwrap().unwrap().0.id;
    {
        let conn = store.conn();
        let package_id: i64 = conn
            .query_row(
                "SELECT id FROM packages WHERE ident = ?1",
                params!["alice#toolbox"],
                |row| row.get(0),
            )
            .unwrap();
        conn.execute(
            "UPDATE transfer_offers
                 SET from_owner_id = ?1, to_owner_id = ?2, accepted_at = NULL
                 WHERE package_id = ?3",
            params![alice_id, carol_id, package_id],
        )
        .unwrap();
    }

    // Carol must not be able to accept it: alice no longer owns the package.
    let carol_accept = sign_accept_for_test(&store, "alice#toolbox", &carol, "carol");
    let err = store
        .accept_transfer("alice#toolbox", "carol", &carol_accept)
        .expect_err("a stale offer must not re-bind the package");
    assert!(
        err.contains("no pending transfer"),
        "expected the stale offer to be invisible, got: {err}"
    );
    assert_eq!(
        store
            .package_owner("alice#toolbox")
            .unwrap()
            .unwrap()
            .owner_display,
        "bob",
        "ownership must stay with bob"
    );

    // And the dispossessed owner cannot open a fresh offer either.
    let (sequence, digest, signature) =
        sign_offer_for_test(&store, "alice#toolbox", &alice, "carol");
    assert!(store
        .create_transfer_offer(
            "alice#toolbox",
            "alice",
            "carol",
            sequence,
            &digest,
            &signature
        )
        .unwrap_err()
        .contains("does not currently own"));
}

#[test]
fn transfer_offer_and_accept_error_branches() {
    let (_temp, store) = test_store();
    let alice = register_keys(&store, "alice");
    let bob = register_keys(&store, "bob");
    let alice_id = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
    store
        .publish_package_version(
            alice_id,
            "alice#toolbox",
            "1.0.0",
            "hash",
            "path",
            "{}",
            &[],
            &PublishMetadata::default(),
        )
        .unwrap();

    let (sequence, digest, offer) = sign_offer_for_test(&store, "alice#toolbox", &alice, "bob");
    let offer_to = |ident: &str, from: &str, to: &str| {
        store.create_transfer_offer(ident, from, to, sequence, &digest, &offer)
    };
    // Unknown package.
    assert!(offer_to("alice#missing", "alice", "bob")
        .unwrap_err()
        .contains("unknown package"));
    // Offering owner mismatch.
    assert!(offer_to("alice#toolbox", "bob", "alice")
        .unwrap_err()
        .contains("does not currently own"));
    // Unknown recipient.
    assert!(offer_to("alice#toolbox", "alice", "nosuch")
        .unwrap_err()
        .contains("unknown recipient"));
    // Cannot transfer to self.
    assert!(offer_to("alice#toolbox", "alice", "alice")
        .unwrap_err()
        .contains("current owner"));

    // Accept with no pending offer for the account.
    let accept = sign_accept_for_test(&store, "alice#toolbox", &bob, "bob");
    assert!(store
        .accept_transfer("alice#toolbox", "bob", &accept)
        .unwrap_err()
        .contains("no pending transfer"));
    assert!(store
        .accept_transfer("alice#toolbox", "nosuch", &accept)
        .unwrap_err()
        .contains("unknown recipient"));

    // A real offer then acceptance re-binds the package.
    offer_to("alice#toolbox", "alice", "bob").unwrap();
    // Re-offering updates the existing row (ON CONFLICT path).
    offer_to("alice#toolbox", "alice", "bob").unwrap();
    store
        .accept_transfer("alice#toolbox", "bob", &accept)
        .unwrap();
    assert_eq!(
        store
            .package_owner("alice#toolbox")
            .unwrap()
            .unwrap()
            .owner_display,
        "bob"
    );
    assert!(store.package_owner("alice#missing").unwrap().is_none());
    let _ = alice;
}

#[test]
fn transfer_history_is_append_only_and_sequenced() {
    let (_temp, store) = test_store();
    let alice = register_keys(&store, "alice");
    let bob = register_keys(&store, "bob");
    let carol = register_keys(&store, "carol");
    publish_as(&store, "alice", "alice#toolbox", "1.0.0", None).unwrap();
    offer_and_accept_for_test(&store, "alice#toolbox", "alice", &alice, "bob", &bob);
    offer_and_accept_for_test(&store, "alice#toolbox", "bob", &bob, "carol", &carol);

    let history = store.transfer_history("alice#toolbox").unwrap();
    assert_eq!(history.len(), 2);
    let digest = crypto::release_digest([("1.0.0", "hash-1.0.0")]);
    for (record, (sequence, from, from_keys, to, to_keys)) in history.iter().zip([
        (0, "alice", &alice, "bob", &bob),
        (1, "bob", &bob, "carol", &carol),
    ]) {
        assert_eq!(record.sequence, sequence);
        assert_eq!(record.from_owner, from);
        assert_eq!(record.to_owner, to);
        assert_eq!(
            record.from_ident_key.as_ref(),
            Some(&from_keys.ident_public)
        );
        assert_eq!(record.to_ident_key.as_ref(), Some(&to_keys.ident_public));
        assert_eq!(record.release_digest.as_deref(), Some(digest.as_str()));
        // Both recorded signatures verify over the v2 terms of THEIR link.
        let from_fp = crypto::fingerprint(&from_keys.ident_public);
        let terms = crypto::TransferTermsRef {
            ident: "alice#toolbox",
            sequence,
            from_owner: from,
            from_ident_fingerprint: &from_fp,
            to_owner: to,
            release_digest: &digest,
        };
        crypto::verify(
            &from_keys.ident_public,
            &crypto::transfer_offer_message_v2(terms),
            record.offer_signature.as_ref().unwrap(),
        )
        .unwrap();
        crypto::verify(
            &to_keys.ident_public,
            &crypto::transfer_accept_message_v2(terms),
            record.accept_signature.as_ref().unwrap(),
        )
        .unwrap();
    }
    assert_eq!(
        store
            .transfer_terms("alice#toolbox")
            .unwrap()
            .unwrap()
            .sequence,
        2
    );

    // The history cannot be rewritten or erased, even by direct SQL.
    let conn = store.conn();
    let update = conn
        .execute(
            "UPDATE transfer_history SET to_owner_id = from_owner_id",
            [],
        )
        .unwrap_err()
        .to_string();
    assert!(update.contains("append-only"), "{update}");
    let delete = conn
        .execute("DELETE FROM transfer_history", [])
        .unwrap_err()
        .to_string();
    assert!(delete.contains("append-only"), "{delete}");
    let rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM transfer_history", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(rows, 2);
}

#[test]
fn stale_terms_refuse_offer_and_accept() {
    let (_temp, store) = test_store();
    let alice = register_keys(&store, "alice");
    let bob = register_keys(&store, "bob");
    publish_as(&store, "alice", "alice#toolbox", "1.0.0", None).unwrap();

    // An offer naming the wrong position is refused outright.
    let (sequence, digest, signature) = sign_offer_for_test(&store, "alice#toolbox", &alice, "bob");
    assert!(store
        .create_transfer_offer(
            "alice#toolbox",
            "alice",
            "bob",
            sequence + 1,
            &digest,
            &signature
        )
        .unwrap_err()
        .contains("transfer terms are stale"));

    // A release published between the offer and the accept: bob signs the
    // terms as they now stand, which are not the terms alice offered.
    store
        .create_transfer_offer(
            "alice#toolbox",
            "alice",
            "bob",
            sequence,
            &digest,
            &signature,
        )
        .unwrap();
    publish_as(&store, "alice", "alice#toolbox", "2.0.0", None).unwrap();
    let accept = sign_accept_for_test(&store, "alice#toolbox", &bob, "bob");
    let err = store
        .accept_transfer("alice#toolbox", "bob", &accept)
        .unwrap_err();
    assert!(err.contains("transfer terms are stale"), "{err}");
    // Re-signing the OLD terms does not help: they are not the current ones.
    let old_terms = crypto::transfer_accept_message_v2(crypto::TransferTermsRef {
        ident: "alice#toolbox",
        sequence,
        from_owner: "alice",
        from_ident_fingerprint: &crypto::fingerprint(&alice.ident_public),
        to_owner: "bob",
        release_digest: &digest,
    });
    let old_accept = crypto::sign(&bob.ident_private, &old_terms).unwrap();
    assert!(store
        .accept_transfer("alice#toolbox", "bob", &old_accept)
        .unwrap_err()
        .contains("invalid transfer accept ident signature"));
    assert_eq!(
        store
            .package_owner("alice#toolbox")
            .unwrap()
            .unwrap()
            .owner_display,
        "alice"
    );
    assert!(store.transfer_history("alice#toolbox").unwrap().is_empty());

    // Re-offering on the current terms goes through.
    offer_and_accept_for_test(&store, "alice#toolbox", "alice", &alice, "bob", &bob);
    assert_eq!(
        store
            .package_owner("alice#toolbox")
            .unwrap()
            .unwrap()
            .owner_display,
        "bob"
    );
}

#[test]
fn legacy_pending_offer_cannot_be_accepted() {
    let (_temp, store) = test_store();
    register_keys(&store, "alice");
    let bob = register_keys(&store, "bob");
    publish_as(&store, "alice", "alice#toolbox", "1.0.0", None).unwrap();
    let alice_id = store.owner_record("alice").unwrap().unwrap().id;
    let bob_id = store.owner_record("bob").unwrap().unwrap().id;
    {
        // The row a pre-plan offer left behind: no terms, no signature.
        let conn = store.conn();
        conn.execute(
            "INSERT INTO transfer_offers (package_id, from_owner_id, to_owner_id, created_at)
                 SELECT id, ?1, ?2, 1 FROM packages WHERE ident = 'alice#toolbox'",
            params![alice_id, bob_id],
        )
        .unwrap();
    }
    let accept = sign_accept_for_test(&store, "alice#toolbox", &bob, "bob");
    let err = store
        .accept_transfer("alice#toolbox", "bob", &accept)
        .unwrap_err();
    assert!(err.contains("predates signed transfers"), "{err}");
    assert_eq!(
        store
            .package_owner("alice#toolbox")
            .unwrap()
            .unwrap()
            .owner_display,
        "alice"
    );
}

fn history_summary(store: &Store, ident: &str) -> Vec<(i64, String, String, bool)> {
    store
        .transfer_history(ident)
        .unwrap()
        .into_iter()
        .map(|record| {
            let signed = record.offer_signature.is_some()
                || record.accept_signature.is_some()
                || record.release_digest.is_some()
                || record.from_ident_key.is_some()
                || record.to_ident_key.is_some();
            (record.sequence, record.from_owner, record.to_owner, signed)
        })
        .collect()
}

fn epochs(store: &Store, ident: &str) -> Vec<(String, i64)> {
    store
        .list_package_versions(ident)
        .unwrap()
        .into_iter()
        .map(|row| (row.version, row.transfer_epoch))
        .collect()
}

/// A database written before plan-168: no history table, no offer terms,
/// no publisher columns. Migrating it replays its one logged transfer into
/// an unsigned seq-0 link and backfills the publisher as the prefix owner.
#[test]
fn migrating_a_legacy_database_records_legacy_transfers_and_publishers() {
    let temp = tempfile::tempdir().unwrap();
    let db_path = temp.path().join("legacy.db");
    {
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            r#"
                CREATE TABLE owners (
                    id INTEGER PRIMARY KEY,
                    owner_display TEXT NOT NULL,
                    owner_folded TEXT NOT NULL UNIQUE,
                    created_at INTEGER NOT NULL,
                    status TEXT NOT NULL
                );
                CREATE TABLE packages (
                    id INTEGER PRIMARY KEY,
                    ident TEXT NOT NULL UNIQUE,
                    owner_id INTEGER NOT NULL,
                    created_at INTEGER NOT NULL
                );
                CREATE TABLE package_versions (
                    id INTEGER PRIMARY KEY,
                    package_id INTEGER NOT NULL,
                    version TEXT NOT NULL,
                    hash TEXT NOT NULL,
                    state TEXT NOT NULL,
                    created_at INTEGER NOT NULL,
                    UNIQUE(package_id, version)
                );
                CREATE TABLE transfer_offers (
                    id INTEGER PRIMARY KEY,
                    package_id INTEGER NOT NULL,
                    from_owner_id INTEGER NOT NULL,
                    to_owner_id INTEGER NOT NULL,
                    created_at INTEGER NOT NULL,
                    accepted_at INTEGER NULL,
                    UNIQUE(package_id)
                );
                CREATE TABLE log_entries (
                    idx INTEGER PRIMARY KEY CHECK (idx >= 0),
                    kind TEXT NOT NULL,
                    payload TEXT NOT NULL,
                    leaf_hash BLOB NOT NULL,
                    created_at INTEGER NOT NULL
                );
                INSERT INTO owners VALUES (1, 'Alice', 'alice', 1, 'active');
                INSERT INTO owners VALUES (2, 'bob', 'bob', 1, 'active');
                INSERT INTO packages VALUES (1, 'alice#pkg', 2, 100);
                INSERT INTO package_versions (package_id, version, hash, state, created_at)
                    VALUES (1, '1.0.0', 'legacyhash', 'available', 100);
                INSERT INTO transfer_offers VALUES (1, 1, 1, 2, 150, 160);
                INSERT INTO log_entries VALUES
                    (0, 'publish', '{"ident":"alice#pkg","version":"1.0.0","hash":"legacyhash"}', x'00', 100),
                    (1, 'transfer-offer', '{"ident":"alice#pkg","from":"Alice","to":"bob"}', x'00', 150),
                    (2, 'transfer-accept', '{"ident":"alice#pkg","to":"bob"}', x'00', 160);
                "#,
        )
        .unwrap();
    }

    let opened =
        Store::open_repository(&db_path, &temp.path().join("data")).expect("legacy migration");
    let store = opened.store;
    let history = store.transfer_history("alice#pkg").unwrap();
    assert_eq!(history.len(), 1);
    let link = &history[0];
    assert_eq!(
        (
            link.sequence,
            link.from_owner.as_str(),
            link.to_owner.as_str()
        ),
        (0, "Alice", "bob")
    );
    assert_eq!((link.offered_at, link.accepted_at), (150, 160));
    assert!(link.offer_signature.is_none() && link.accept_signature.is_none());
    assert!(link.from_ident_key.is_none() && link.to_ident_key.is_none());
    assert!(link.release_digest.is_none());

    let versions = store.list_package_versions("alice#pkg").unwrap();
    assert_eq!(versions[0].publisher_display.as_deref(), Some("Alice"));
    assert_eq!(versions[0].publisher_ident_fingerprint, None);
    assert_eq!(versions[0].transfer_epoch, 0);

    // Re-running the migration is a no-op.
    store.migrate().unwrap();
    store.migrate().unwrap();
    assert_eq!(store.transfer_history("alice#pkg").unwrap().len(), 1);
}

#[test]
fn migrating_recovers_an_accepted_transfer_hidden_by_a_later_offer() {
    let (_temp, store) = test_store();
    register_keys(&store, "alice");
    register_keys(&store, "bob");
    register_keys(&store, "carol");
    publish_as(&store, "alice", "alice#toolbox", "1.0.0", None).unwrap();
    legacy_transfer(&store, "alice#toolbox", "alice", "bob");
    // bob's pending offer to carol reset the one `transfer_offers` row, so
    // that table no longer shows the accepted alice -> bob transfer at all.
    legacy_log(
        &store,
        "transfer-offer",
        serde_json::json!({"ident": "alice#toolbox", "from": "bob", "to": "carol"}),
    );
    {
        let conn = store.conn();
        conn.execute(
            "UPDATE transfer_offers SET from_owner_id = to_owner_id, to_owner_id = ?1,
                     accepted_at = NULL",
            params![store_owner_id_unlocked(&conn, "carol")],
        )
        .unwrap();
    }
    store.migrate().unwrap();
    assert_eq!(
        history_summary(&store, "alice#toolbox"),
        vec![(0, "alice".to_string(), "bob".to_string(), false)]
    );
}

fn store_owner_id_unlocked(conn: &Connection, owner: &str) -> i64 {
    conn.query_row(
        "SELECT id FROM owners WHERE owner_folded = ?1",
        params![fold_owner(owner)],
        |row| row.get(0),
    )
    .unwrap()
}

#[test]
fn migrating_replays_a_multi_hop_legacy_chain() {
    let (_temp, store) = test_store();
    register_keys(&store, "alice");
    register_keys(&store, "bob");
    register_keys(&store, "carol");
    publish_as(&store, "alice", "alice#toolbox", "1.0.0", None).unwrap();
    legacy_transfer(&store, "alice#toolbox", "alice", "bob");
    legacy_transfer(&store, "alice#toolbox", "bob", "carol");
    store.migrate().unwrap();
    assert_eq!(
        history_summary(&store, "alice#toolbox"),
        vec![
            (0, "alice".to_string(), "bob".to_string(), false),
            (1, "bob".to_string(), "carol".to_string(), false),
        ]
    );
    // The next signed transfer continues the numbering.
    assert_eq!(
        store
            .transfer_terms("alice#toolbox")
            .unwrap()
            .unwrap()
            .sequence,
        2
    );
}

#[test]
fn migrating_recovers_epochs_after_a_legacy_round_trip() {
    let (_temp, store) = test_store();
    register_keys(&store, "alice");
    register_keys(&store, "bob");
    publish_as(&store, "alice", "alice#toolbox", "1.0.0", None).unwrap();
    legacy_transfer(&store, "alice#toolbox", "alice", "bob");
    legacy_transfer(&store, "alice#toolbox", "bob", "alice");
    publish_as(&store, "alice", "alice#toolbox", "2.0.0", None).unwrap();
    publish_as(&store, "alice", "alice#toolbox", "3.0.0", None).unwrap();
    store.migrate().unwrap();
    assert_eq!(
        epochs(&store, "alice#toolbox"),
        vec![
            ("1.0.0".to_string(), 0),
            ("2.0.0".to_string(), 2),
            ("3.0.0".to_string(), 2),
        ]
    );
    assert_eq!(history_summary(&store, "alice#toolbox").len(), 2);
}

#[test]
fn migrating_refuses_a_log_that_disagrees_with_the_owner() {
    let (_temp, store) = test_store();
    register_keys(&store, "alice");
    register_keys(&store, "bob");
    register_keys(&store, "carol");
    publish_as(&store, "alice", "alice#toolbox", "1.0.0", None).unwrap();
    legacy_transfer(&store, "alice#toolbox", "alice", "bob");
    {
        // The tables say carol owns it; the log never reaches carol.
        let conn = store.conn();
        let carol_id = store_owner_id_unlocked(&conn, "carol");
        conn.execute(
            "UPDATE packages SET owner_id = ?1 WHERE ident = 'alice#toolbox'",
            params![carol_id],
        )
        .unwrap();
    }
    let err = store.migrate().unwrap_err();
    assert_eq!(
        err,
        "transfer log for alice#toolbox ends at bob but the package is owned by carol"
    );
    // Nothing was half-written.
    assert!(store.transfer_history("alice#toolbox").unwrap().is_empty());
}

// --- Migration of a pre-existing database -------------------------------

#[test]
fn migrating_a_legacy_database_adds_the_abi_index_and_kind_columns() {
    // `abi_index` (plan-10-B1) and `package_blobs.kind` (plan-48-A) were
    // added after the tables existed. `CREATE TABLE IF NOT EXISTS` leaves an
    // older table exactly as it was found, so the idempotent ALTER is the
    // only thing that upgrades a database created before those columns — and
    // the documented contract is that every pre-existing blob row migrates to
    // `kind = 'package'`.
    let temp = tempfile::tempdir().unwrap();
    let db_path = temp.path().join("legacy.db");
    {
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            r#"
                CREATE TABLE packages (
                    id INTEGER PRIMARY KEY,
                    ident TEXT NOT NULL UNIQUE,
                    owner_id INTEGER NOT NULL,
                    created_at INTEGER NOT NULL
                );
                CREATE TABLE package_versions (
                    id INTEGER PRIMARY KEY,
                    package_id INTEGER NOT NULL,
                    version TEXT NOT NULL,
                    hash TEXT NOT NULL,
                    state TEXT NOT NULL,
                    created_at INTEGER NOT NULL,
                    UNIQUE(package_id, version)
                );
                CREATE TABLE package_blobs (
                    hash TEXT PRIMARY KEY,
                    path TEXT NOT NULL,
                    created_at INTEGER NOT NULL
                );
                INSERT INTO packages (id, ident, owner_id, created_at)
                    VALUES (1, 'legacy#pkg', 1, 100);
                INSERT INTO package_versions (package_id, version, hash, state, created_at)
                    VALUES (1, '1.0.0', 'legacyhash', 'available', 100);
                INSERT INTO package_blobs (hash, path, created_at)
                    VALUES ('legacyhash', 'legacy/path', 100);
                "#,
        )
        .unwrap();
    }

    let opened =
        Store::open_repository(&db_path, &temp.path().join("data")).expect("legacy migration");
    let store = opened.store;

    // The pre-existing version row survived and picked up the `{}` default.
    let versions = store.list_package_versions("legacy#pkg").unwrap();
    assert_eq!(versions.len(), 1);
    assert_eq!(versions[0].version, "1.0.0");
    assert_eq!(versions[0].abi_index, "{}");
    // The pre-existing blob row migrated to the `package` kind, so
    // `GET /blob/<hash>` still resolves it the way it was stored.
    assert_eq!(
        store.blob_kind("legacyhash").unwrap().as_deref(),
        Some("package")
    );
    // Re-running the migration over the now-current schema is a no-op.
    store.migrate().unwrap();
    assert_eq!(
        store.blob_kind("legacyhash").unwrap().as_deref(),
        Some("package")
    );
}

#[test]
fn migrating_a_legacy_database_adds_the_metadata_columns_and_target_table() {
    // plan-61-A Phase 1. `author`/`url`/`description` land on a table that
    // already exists in a deployed database (repository/DEPLOY.md: a Fly.io
    // volume), so `CREATE TABLE IF NOT EXISTS` cannot add them — only the
    // idempotent ALTER can. The contract is that every pre-existing version
    // row reads back NULL for all three: the server never knew these values,
    // and NULL is how it says so.
    let temp = tempfile::tempdir().unwrap();
    let db_path = temp.path().join("legacy.db");
    {
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            r#"
                CREATE TABLE packages (
                    id INTEGER PRIMARY KEY,
                    ident TEXT NOT NULL UNIQUE,
                    owner_id INTEGER NOT NULL,
                    created_at INTEGER NOT NULL
                );
                CREATE TABLE package_versions (
                    id INTEGER PRIMARY KEY,
                    package_id INTEGER NOT NULL,
                    version TEXT NOT NULL,
                    hash TEXT NOT NULL,
                    state TEXT NOT NULL,
                    created_at INTEGER NOT NULL,
                    UNIQUE(package_id, version)
                );
                INSERT INTO packages (id, ident, owner_id, created_at)
                    VALUES (1, 'legacy#pkg', 1, 100);
                INSERT INTO package_versions (package_id, version, hash, state, created_at)
                    VALUES (1, '1.0.0', 'legacyhash', 'available', 100);
                "#,
        )
        .unwrap();
    }

    let opened =
        Store::open_repository(&db_path, &temp.path().join("data")).expect("legacy migration");
    let store = opened.store;
    let conn = store.conn();

    let (author, url, description): (Option<String>, Option<String>, Option<String>) = conn
        .query_row(
            "SELECT author, url, description FROM package_versions WHERE version = '1.0.0'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("the three metadata columns must exist after migrating");
    assert_eq!(author, None, "a legacy row cannot claim an author");
    assert_eq!(url, None);
    assert_eq!(
        description, None,
        "plan-61-A creates `description` but never writes it; plan-61-E does"
    );

    // The target table and its index came from the CREATE batch, so they
    // appear even though `package_versions` predates this migration.
    let targets: i64 = conn
        .query_row("SELECT count(*) FROM package_version_targets", [], |row| {
            row.get(0)
        })
        .expect("package_version_targets must exist after migrating");
    assert_eq!(targets, 0);
    let index_count: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type = 'index' \
                 AND name = 'package_version_targets_version_idx'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(index_count, 1, "the lookup index must be created too");

    // Re-running the migration over the now-current schema is a no-op.
    drop(conn);
    store.migrate().unwrap();
    store.migrate().unwrap();
}

#[test]
fn add_column_if_missing_distinguishes_a_duplicate_column_from_a_real_failure() {
    // The helper swallows exactly one error — "duplicate column name", which
    // means the migration already ran. Anything else must surface, or a
    // half-applied schema would be reported as a successful migration.
    let temp = tempfile::tempdir().unwrap();
    let conn = Connection::open(temp.path().join("m.db")).unwrap();
    conn.execute_batch("CREATE TABLE t (a TEXT NOT NULL);")
        .unwrap();

    add_column_if_missing(&conn, "t", "b TEXT NOT NULL DEFAULT ''").unwrap();
    // Idempotent: the second call sees "duplicate column name" and succeeds.
    add_column_if_missing(&conn, "t", "b TEXT NOT NULL DEFAULT ''").unwrap();
    conn.execute("INSERT INTO t (a, b) VALUES ('x', 'y')", [])
        .expect("column b must exist after the migration");

    let err = add_column_if_missing(&conn, "no_such_table", "c TEXT").unwrap_err();
    assert!(
        err.contains("failed to add column to no_such_table"),
        "a real ALTER failure must not be swallowed: {err}"
    );
}
