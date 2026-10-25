use super::*;

mod accounts;
mod blobs;
mod faults_governance;
mod faults_versions;
mod packages;
mod registry_tables;
mod transfers;

fn test_store() -> (tempfile::TempDir, Store) {
    let temp = tempfile::tempdir().unwrap();
    let db_path = temp.path().join("meta.db");
    let data_path = temp.path().join("data");
    let opened = Store::open_repository(&db_path, &data_path).unwrap();
    (temp, opened.store)
}

pub(crate) struct RegisteredKeys {
    pub(crate) auth_public: Vec<u8>,
    pub(crate) auth_private: Vec<u8>,
    pub(crate) ident_public: Vec<u8>,
    pub(crate) ident_private: Vec<u8>,
}

/// Sign a v2 transfer offer of `ident` to `to_owner` on the package's
/// current terms with `from`'s ident key; returns `(sequence, digest,
/// signature)` ready for `create_transfer_offer`.
pub(crate) fn sign_offer_for_test(
    store: &Store,
    ident: &str,
    from: &RegisteredKeys,
    to_owner: &str,
) -> (i64, String, Vec<u8>) {
    let terms = store
        .transfer_terms(ident)
        .unwrap()
        .expect("package exists");
    let to_display = store
        .owner_record(to_owner)
        .unwrap()
        .map(|owner| owner.owner_display)
        .unwrap_or_else(|| to_owner.to_string());
    let message = crypto::transfer_offer_message_v2(crypto::TransferTermsRef {
        ident,
        sequence: terms.sequence,
        from_owner: &terms.current_owner.owner_display,
        from_ident_fingerprint: &crypto::fingerprint(&from.ident_public),
        to_owner: &to_display,
        release_digest: &terms.release_digest,
    });
    let signature = crypto::sign(&from.ident_private, &message).unwrap();
    (terms.sequence, terms.release_digest, signature)
}

/// Sign a v2 transfer acceptance of `ident` by `to_owner` on the package's
/// current terms with `to`'s ident key.
pub(crate) fn sign_accept_for_test(
    store: &Store,
    ident: &str,
    to: &RegisteredKeys,
    to_owner: &str,
) -> Vec<u8> {
    let terms = store
        .transfer_terms(ident)
        .unwrap()
        .expect("package exists");
    let to_display = store
        .owner_record(to_owner)
        .unwrap()
        .map(|owner| owner.owner_display)
        .unwrap_or_else(|| to_owner.to_string());
    let message = crypto::transfer_accept_message_v2(crypto::TransferTermsRef {
        ident,
        sequence: terms.sequence,
        from_owner: &terms.current_owner.owner_display,
        from_ident_fingerprint: &terms.current_ident.fingerprint,
        to_owner: &to_display,
        release_digest: &terms.release_digest,
    });
    crypto::sign(&to.ident_private, &message).unwrap()
}

/// The whole two-sided transfer of `ident` from `from_owner` to `to_owner`,
/// through the store with real v2 signatures.
pub(crate) fn offer_and_accept_for_test(
    store: &Store,
    ident: &str,
    from_owner: &str,
    from: &RegisteredKeys,
    to_owner: &str,
    to: &RegisteredKeys,
) {
    let (sequence, digest, signature) = sign_offer_for_test(store, ident, from, to_owner);
    store
        .create_transfer_offer(ident, from_owner, to_owner, sequence, &digest, &signature)
        .unwrap();
    let accept = sign_accept_for_test(store, ident, to, to_owner);
    store.accept_transfer(ident, to_owner, &accept).unwrap();
}

pub(crate) fn register_keys(store: &Store, owner: &str) -> RegisteredKeys {
    let (auth_public, auth_private) = crypto::generate_keypair();
    let (ident_public, ident_private) = crypto::generate_keypair();
    let auth_message = crypto::registration_message(crypto::ROLE_AUTH, owner, &auth_public);
    let auth_proof = crypto::sign(&auth_private, &auth_message).unwrap();
    let ident_message = crypto::registration_message(crypto::ROLE_IDENT, owner, &ident_public);
    let ident_proof = crypto::sign(&ident_private, &ident_message).unwrap();
    store
        .register_owner(
            owner,
            &auth_public,
            &auth_proof,
            &ident_public,
            &ident_proof,
        )
        .unwrap();
    RegisteredKeys {
        auth_public,
        auth_private,
        ident_public,
        ident_private,
    }
}

fn register(store: &Store, owner: &str) -> (Vec<u8>, Vec<u8>) {
    let keys = register_keys(store, owner);
    (keys.auth_public, keys.auth_private)
}

/// Helper: the approval predicate every honest fetch passes.
fn approved(_key: &[u8]) -> bool {
    true
}

// === plan-168-A: signed transfer history =================================

/// Publish `version` of `alice#toolbox`-style `ident` as `owner`, recording
/// `fingerprint` as the publisher key.
fn publish_as(
    store: &Store,
    owner: &str,
    ident: &str,
    version: &str,
    fingerprint: Option<String>,
) -> Result<PublishedVersion, String> {
    let owner_id = store.owner_record(owner).unwrap().unwrap().id;
    store.publish_package_version(
        owner_id,
        ident,
        version,
        &format!("hash-{version}"),
        &format!("path-{version}"),
        "{}",
        &[],
        &PublishMetadata {
            publisher_ident_fingerprint: fingerprint,
            ..Default::default()
        },
    )
}

/// Append a raw transparency-log entry, as a pre-plan registry wrote it.
pub(crate) fn legacy_log(store: &Store, kind: &str, payload: serde_json::Value) {
    let mut conn = store.conn();
    let tx = conn.transaction().unwrap();
    append_log_tx(&tx, kind, &payload.to_string()).unwrap();
    tx.commit().unwrap();
}

/// A pre-plan transfer: the two v1 log entries, the `owner_id` rewrite, and
/// the single `transfer_offers` row (UPSERTed, as the old code did). No
/// `transfer_history` row — that is what the migration has to recover.
pub(crate) fn legacy_transfer(store: &Store, ident: &str, from: &str, to: &str) {
    legacy_log(
        store,
        "transfer-offer",
        serde_json::json!({"ident": ident, "from": from, "to": to}),
    );
    legacy_log(
        store,
        "transfer-accept",
        serde_json::json!({"ident": ident, "to": to}),
    );
    let from_id = store.owner_record(from).unwrap().unwrap().id;
    let to_id = store.owner_record(to).unwrap().unwrap().id;
    let conn = store.conn();
    conn.execute(
        "UPDATE packages SET owner_id = ?1 WHERE ident = ?2",
        params![to_id, ident],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO transfer_offers (package_id, from_owner_id, to_owner_id, created_at,
                 accepted_at)
             SELECT id, ?1, ?2, 1, 2 FROM packages WHERE ident = ?3
             ON CONFLICT(package_id) DO UPDATE SET from_owner_id = excluded.from_owner_id,
               to_owner_id = excluded.to_owner_id, accepted_at = excluded.accepted_at",
        params![from_id, to_id, ident],
    )
    .unwrap();
}

// --- Owner-name validation at every entry point -------------------------

/// A registration payload whose two proofs are valid for `owner`.
fn registration_payload(owner: &str) -> (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>) {
    let (auth_public, auth_private) = crypto::generate_keypair();
    let (ident_public, ident_private) = crypto::generate_keypair();
    let auth_proof = crypto::sign(
        &auth_private,
        &crypto::registration_message(crypto::ROLE_AUTH, owner, &auth_public),
    )
    .unwrap();
    let ident_proof = crypto::sign(
        &ident_private,
        &crypto::registration_message(crypto::ROLE_IDENT, owner, &ident_public),
    )
    .unwrap();
    (auth_public, auth_proof, ident_public, ident_proof)
}

/// Run raw SQL against a live store's connection, with foreign keys off: the
/// way a test puts the database into a state no store method writes (a row
/// another release of the registry, an operator, or a damaged file left).
pub(crate) fn tamper(store: &Store, sql: &str) {
    let conn = store.conn();
    conn.pragma_update(None, "foreign_keys", "OFF").unwrap();
    conn.execute_batch(sql).unwrap();
    conn.pragma_update(None, "foreign_keys", "ON").unwrap();
}

/// Make every `op` (`INSERT`, `UPDATE` or `DELETE`) on `table` abort with
/// `injected <op> failure`, the way a full disk or a constraint another writer
/// added refuses one statement while the rest of the database still works.
pub(crate) fn refuse_writes(store: &Store, table: &str, op: &str) {
    tamper(
        store,
        &format!(
            "CREATE TRIGGER refuse_{op}_{table} BEFORE {op} ON {table}
             BEGIN SELECT RAISE(ABORT, 'injected {op} failure'); END;"
        ),
    );
}

/// Make any transaction that runs `op` on `table` fail at `COMMIT` and roll
/// back: the trigger records a deferred foreign-key violation, which SQLite
/// only reports when the transaction tries to commit.
pub(crate) fn refuse_commit_after(store: &Store, table: &str, op: &str) {
    tamper(
        store,
        &format!(
            "CREATE TABLE IF NOT EXISTS commit_trap (
                 owner_id INTEGER REFERENCES owners(id) DEFERRABLE INITIALLY DEFERRED
             );
             CREATE TRIGGER trap_{op}_{table} AFTER {op} ON {table}
             BEGIN INSERT INTO commit_trap (owner_id) VALUES (-1); END;"
        ),
    );
}

/// Leave the store's connection inside an open transaction, so the next
/// `Connection::transaction()` fails to begin (`cannot start a transaction
/// within a transaction`) while plain reads still work.
pub(crate) fn wedge_transaction(store: &Store) {
    store.conn().execute_batch("BEGIN").unwrap();
}

/// Lower the connection's SQLITE_LIMIT_LENGTH to `bytes`, so binding any
/// longer text or blob parameter fails with SQLITE_TOOBIG — the one failure a
/// prepared statement's `query`/`query_map` can return.
pub(crate) fn limit_value_length(store: &Store, bytes: i32) {
    store
        .conn()
        .set_limit(rusqlite::limits::Limit::SQLITE_LIMIT_LENGTH, bytes);
}

pub(crate) fn drop_tables(store: &Store, tables: &[&str]) {
    let conn = store.conn();
    conn.pragma_update(None, "foreign_keys", "OFF").unwrap();
    for table in tables {
        conn.execute_batch(&format!("DROP TABLE {table}")).unwrap();
    }
    conn.pragma_update(None, "foreign_keys", "ON").unwrap();
}
