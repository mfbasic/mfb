//! plan-176-L: the registry store's schema — `migrate`, the legacy-transfer backfill and column additions.
//! The `Store` handle and its connection live in `repository/src/store/mod.rs`.

use super::*;

impl Store {
    pub fn migrate(&self) -> Result<(), String> {
        let mut conn = self.conn();
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS owners (
                id INTEGER PRIMARY KEY,
                owner_display TEXT NOT NULL,
                owner_folded TEXT NOT NULL UNIQUE,
                created_at INTEGER NOT NULL,
                status TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS keys (
                id INTEGER PRIMARY KEY,
                owner_id INTEGER NOT NULL REFERENCES owners(id),
                role TEXT NOT NULL,
                public_key BLOB NOT NULL,
                fingerprint TEXT NOT NULL UNIQUE,
                status TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                revoked_at INTEGER NULL
            );

            -- `purpose` (bug-493) binds a challenge to the operation it was
            -- minted for: 'login' (an auth key, `/auth/challenge`) or 'revoke'
            -- (the ident key, `/machines/revoke/challenge`). Without it the two
            -- rows were structurally identical and `/machines/revoke` accepted
            -- a login challenge signed by an ordinary auth key.
            CREATE TABLE IF NOT EXISTS auth_challenges (
                id TEXT PRIMARY KEY,
                owner_id INTEGER NOT NULL REFERENCES owners(id),
                key_id INTEGER NOT NULL REFERENCES keys(id),
                nonce BLOB NOT NULL,
                purpose TEXT NOT NULL DEFAULT 'login',
                created_at INTEGER NOT NULL,
                expires_at INTEGER NOT NULL,
                used_at INTEGER NULL
            );

            CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                owner_id INTEGER NOT NULL REFERENCES owners(id),
                key_id INTEGER NOT NULL REFERENCES keys(id),
                jwt_id TEXT NOT NULL UNIQUE,
                issued_at INTEGER NOT NULL,
                expires_at INTEGER NOT NULL,
                revoked_at INTEGER NULL
            );

            CREATE TABLE IF NOT EXISTS server_secrets (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                secret BLOB NOT NULL,
                created_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS server_keys (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                public_key BLOB NOT NULL,
                private_key BLOB NOT NULL,
                created_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS log_entries (
                idx INTEGER PRIMARY KEY CHECK (idx >= 0),
                kind TEXT NOT NULL,
                payload TEXT NOT NULL,
                leaf_hash BLOB NOT NULL,
                created_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS ident_chain (
                id INTEGER PRIMARY KEY,
                owner_id INTEGER NOT NULL REFERENCES owners(id),
                old_key_id INTEGER NOT NULL REFERENCES keys(id),
                new_key_id INTEGER NOT NULL REFERENCES keys(id),
                signature BLOB NOT NULL,
                created_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS pairing_blobs (
                id INTEGER PRIMARY KEY,
                owner_id INTEGER NOT NULL REFERENCES owners(id),
                lookup TEXT NOT NULL UNIQUE,
                blob BLOB NOT NULL,
                salt BLOB NOT NULL,
                approval_key BLOB NOT NULL DEFAULT x'',
                created_at INTEGER NOT NULL,
                expires_at INTEGER NOT NULL,
                used_at INTEGER NULL
            );

            CREATE TABLE IF NOT EXISTS signing_requests (
                id INTEGER PRIMARY KEY,
                owner_id INTEGER NOT NULL REFERENCES owners(id),
                ident TEXT NOT NULL,
                version TEXT NOT NULL,
                signing_fingerprint TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS packages (
                id INTEGER PRIMARY KEY,
                ident TEXT NOT NULL UNIQUE,
                owner_id INTEGER NOT NULL REFERENCES owners(id),
                created_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS package_versions (
                id INTEGER PRIMARY KEY,
                package_id INTEGER NOT NULL REFERENCES packages(id),
                version TEXT NOT NULL,
                hash TEXT NOT NULL,
                state TEXT NOT NULL,
                abi_index TEXT NOT NULL DEFAULT '{}',
                author TEXT NULL,
                url TEXT NULL,
                description TEXT NULL,
                created_at INTEGER NOT NULL,
                UNIQUE(package_id, version)
            );

            -- plan-61-A §3: the native target matrix, one row per section-10
            -- *locator* — never per distinct blob hash, which under-reports
            -- targets when two platforms ship byte-identical blobs under
            -- different `source` filenames.
            --
            -- `arch` NULL is the any-arch wildcard (a locator whose `arch` is
            -- the empty string), not missing data. `libc` and `lib_type` are
            -- token strings decoded from the wire integers so the database
            -- reads honestly under `sqlite3` and needs no mapping table
            -- downstream. Per §3.1 every row written today has
            -- `lib_type = 'vendor'` and a non-NULL `blob_hash`; both stay
            -- permissive so capturing `system` locators later needs no
            -- migration.
            CREATE TABLE IF NOT EXISTS package_version_targets (
                package_version_id INTEGER NOT NULL REFERENCES package_versions(id),
                blob_hash TEXT NULL,
                os TEXT NOT NULL,
                arch TEXT NULL,
                libc TEXT NULL,
                lib_type TEXT NOT NULL,
                logical TEXT NOT NULL,
                source TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS package_version_targets_version_idx
                ON package_version_targets(package_version_id);

            CREATE TABLE IF NOT EXISTS package_blobs (
                hash TEXT PRIMARY KEY,
                path TEXT NOT NULL,
                kind TEXT NOT NULL DEFAULT 'package',
                created_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS package_version_blobs (
                package_version_id INTEGER NOT NULL REFERENCES package_versions(id),
                hash TEXT NOT NULL REFERENCES package_blobs(hash),
                PRIMARY KEY (package_version_id, hash)
            );

            -- plan-126-E: each version's MFPC section 17 (the `DOC` table), stored
            -- RAW so `mfb_wire::docs` stays the only decoder. A side table rather
            -- than a column on `package_versions`: at 9-28 KB a BLOB column would
            -- bloat every SELECT on a table the search, detail, index and audit
            -- paths all read, and "this version has no documentation" is an
            -- absent row rather than a NULL to interpret. Kept per version, not
            -- latest-only, so a yanked newest release falls back to the older
            -- active one's docs by a row lookup rather than a blob fetch -- which
            -- on S3 would mean the server fetching its own blob over HTTPS.
            CREATE TABLE IF NOT EXISTS package_version_docs (
                package_version_id INTEGER PRIMARY KEY REFERENCES package_versions(id),
                doc_section BLOB NOT NULL
            );

            CREATE TABLE IF NOT EXISTS release_state_changes (
                id INTEGER PRIMARY KEY,
                package_version_id INTEGER NOT NULL REFERENCES package_versions(id),
                state TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS org_members (
                id INTEGER PRIMARY KEY,
                org_id INTEGER NOT NULL REFERENCES owners(id),
                member_id INTEGER NOT NULL REFERENCES owners(id),
                role TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                UNIQUE(org_id, member_id)
            );

            CREATE TABLE IF NOT EXISTS publish_tokens (
                id INTEGER PRIMARY KEY,
                owner_id INTEGER NOT NULL REFERENCES owners(id),
                key_id INTEGER NOT NULL REFERENCES keys(id),
                scope TEXT NOT NULL,
                expires_at INTEGER NOT NULL,
                revoked_at INTEGER NULL,
                created_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS transfer_offers (
                id INTEGER PRIMARY KEY,
                package_id INTEGER NOT NULL REFERENCES packages(id),
                from_owner_id INTEGER NOT NULL REFERENCES owners(id),
                to_owner_id INTEGER NOT NULL REFERENCES owners(id),
                created_at INTEGER NOT NULL,
                accepted_at INTEGER NULL,
                UNIQUE(package_id)
            );

            -- plan-168-A §4.2: the append-only, signed ownership history. One
            -- row per ACCEPTED transfer, numbered from 0 per package. The key,
            -- digest and signature columns are NULL only for a legacy row
            -- replayed from the transparency log (`backfill_legacy_transfers`),
            -- which no client can authenticate and so treats as the end of the
            -- verifiable chain.
            CREATE TABLE IF NOT EXISTS transfer_history (
                id INTEGER PRIMARY KEY,
                package_id INTEGER NOT NULL REFERENCES packages(id),
                sequence INTEGER NOT NULL,
                from_owner_id INTEGER NOT NULL REFERENCES owners(id),
                from_ident_key_id INTEGER NULL REFERENCES keys(id),
                to_owner_id INTEGER NOT NULL REFERENCES owners(id),
                to_ident_key_id INTEGER NULL REFERENCES keys(id),
                release_digest TEXT NULL,
                offer_signature BLOB NULL,
                accept_signature BLOB NULL,
                offered_at INTEGER NOT NULL,
                accepted_at INTEGER NOT NULL,
                UNIQUE(package_id, sequence)
            );
            CREATE TRIGGER IF NOT EXISTS transfer_history_no_update
                BEFORE UPDATE ON transfer_history
                BEGIN SELECT RAISE(ABORT, 'transfer_history is append-only'); END;
            CREATE TRIGGER IF NOT EXISTS transfer_history_no_delete
                BEFORE DELETE ON transfer_history
                BEGIN SELECT RAISE(ABORT, 'transfer_history is append-only'); END;

            CREATE TABLE IF NOT EXISTS registry_config (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                registry_id TEXT NOT NULL,
                root_version INTEGER NOT NULL,
                root_public BLOB NOT NULL,
                root_json TEXT NOT NULL,
                root_signature BLOB NOT NULL,
                snapshot_public BLOB NOT NULL,
                snapshot_private BLOB NOT NULL,
                timestamp_public BLOB NOT NULL,
                timestamp_private BLOB NOT NULL,
                created_at INTEGER NOT NULL
            );
            "#,
        )
        .map_err(|err| format!("failed to migrate database: {err}"))?;
        // Idempotent column additions for databases created before the column
        // existed (plan-10-B1 abi_index). A "duplicate column" error means the
        // migration already ran, so it is ignored.
        add_column_if_missing(
            &conn,
            "package_versions",
            "abi_index TEXT NOT NULL DEFAULT '{}'",
        )?;
        // plan-48-A §4.1: blobs gain a kind so native library blobs are stored
        // and served honestly. The default migrates every pre-existing row to
        // `package`, matching how they were always stored.
        add_column_if_missing(
            &conn,
            "package_blobs",
            "kind TEXT NOT NULL DEFAULT 'package'",
        )?;
        // plan-61-A §1: human-facing metadata the server already receives and
        // parses but discarded until now. All three are NULL-able: a version
        // published before this migration has no recorded value, and NULL says
        // "not known" where '' would claim the publisher set it empty.
        // `description` is created here and stays NULL until plan-61-E.
        add_column_if_missing(&conn, "package_versions", "author TEXT NULL")?;
        add_column_if_missing(&conn, "package_versions", "url TEXT NULL")?;
        add_column_if_missing(&conn, "package_versions", "description TEXT NULL")?;
        // bug-493: a database created before the column existed. Every row
        // already there defaults to 'login', which is what `/auth/challenge`
        // always minted; an in-flight revocation challenge across the upgrade
        // (a five-minute window) simply has to be re-issued.
        add_column_if_missing(
            &conn,
            "auth_challenges",
            "purpose TEXT NOT NULL DEFAULT 'login'",
        )?;
        // bug-583: the code-derived approval public key. Pre-migration rows
        // default to the empty key, which no approval signature can ever
        // verify under, so an in-flight pairing across the upgrade is refused
        // rather than silently accepted on the old lookup-only rule. Pairings
        // live ten minutes, so the window closes on its own.
        add_column_if_missing(
            &conn,
            "pairing_blobs",
            "approval_key BLOB NOT NULL DEFAULT x''",
        )?;
        // plan-168-A §4.2: the signed terms of a pending offer. NULL on a row
        // written before signed transfers, which `accept_transfer` refuses (the
        // owner must re-offer under the v2 protocol).
        add_column_if_missing(&conn, "transfer_offers", "sequence INTEGER NULL")?;
        add_column_if_missing(&conn, "transfer_offers", "from_ident_key_id INTEGER NULL")?;
        add_column_if_missing(&conn, "transfer_offers", "release_digest TEXT NULL")?;
        add_column_if_missing(&conn, "transfer_offers", "offer_signature BLOB NULL")?;
        // plan-168-A §4.2: who published each release, under which ident key,
        // and in which ownership epoch (the number of accepted transfers before
        // it). Publisher keys of pre-plan releases are not recorded (NULL): a
        // client verifies the downloaded header key, never this column.
        add_column_if_missing(&conn, "package_versions", "publisher_owner_id INTEGER NULL")?;
        add_column_if_missing(
            &conn,
            "package_versions",
            "publisher_ident_key_id INTEGER NULL",
        )?;
        add_column_if_missing(
            &conn,
            "package_versions",
            "transfer_epoch INTEGER NOT NULL DEFAULT 0",
        )?;
        // Every pre-plan release was published by its ident-prefix owner:
        // `/validate` required the session to fold-equal the prefix, and
        // `publish_package_version` resolves the package row by the session
        // owner. SQLite's ASCII-only `lower()` equals `fold_owner` because owner
        // names are ASCII (`validate_owner_name`). Idempotent: only NULL rows
        // whose prefix owner resolves, and the UPDATE runs only when one exists,
        // so re-opening an up-to-date database never writes (a deliberately
        // read-only `0400` database must still open).
        let publishers_missing: bool = conn
            .query_row(
                "SELECT EXISTS (
                     SELECT 1 FROM package_versions pv
                     JOIN packages p ON p.id = pv.package_id
                     JOIN owners o
                       ON o.owner_folded = lower(substr(p.ident, 1, instr(p.ident, '#') - 1))
                     WHERE pv.publisher_owner_id IS NULL)",
                [],
                |row| row.get(0),
            )
            .map_err(|err| format!("failed to scan release publishers: {err}"))?;
        if publishers_missing {
            conn.execute(
                "UPDATE package_versions SET publisher_owner_id = (
                     SELECT o.id FROM owners o JOIN packages p ON p.id = package_versions.package_id
                     WHERE o.owner_folded = lower(substr(p.ident, 1, instr(p.ident, '#') - 1)))
                 WHERE publisher_owner_id IS NULL",
                [],
            )
            .map_err(|err| format!("failed to backfill release publishers: {err}"))?;
        }
        backfill_legacy_transfers(&mut conn)?;
        Ok(())
    }
}

/// Replay the transparency log into `transfer_history` for every package that
/// has accepted transfers but no history yet (plan-168-A §4.2), and recover
/// each release's ownership epoch.
///
/// `transfer_offers` cannot be the source: it keeps one row per package and a
/// later offer resets `accepted_at`, so an accepted transfer followed by a new
/// offer leaves no trace there. The log is append-only and records every
/// `transfer-offer`, `transfer-accept` and `publish`; walking them in `idx`
/// order from the ident-prefix owner gives each legacy transfer's position,
/// owners and timestamps, and each release's epoch (accepts logged before it).
///
/// Every replayed link is unsigned: no key, digest or signatures existed, so a
/// client cannot authenticate it and treats it as the end of the verifiable
/// chain. The walk must end at the package's stored owner, or `migrate` fails:
/// a log that disagrees with the tables would otherwise be served as a guessed
/// chain. Idempotent: a package with any history row is skipped.
fn backfill_legacy_transfers(conn: &mut Connection) -> Result<(), String> {
    let candidates: Vec<(i64, String, i64)> = {
        let mut statement = conn
            .prepare(
                "SELECT p.id, p.ident, p.owner_id FROM packages p
                 WHERE NOT EXISTS (SELECT 1 FROM transfer_history h WHERE h.package_id = p.id)
                 ORDER BY p.id",
            )
            .map_err(|err| format!("failed to prepare legacy transfer scan: {err}"))?;
        let rows = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .map_err(|err| format!("failed to scan packages for legacy transfers: {err}"))?;
        rows.collect::<rusqlite::Result<_>>()
            .map_err(|err| format!("failed to read package for legacy transfers: {err}"))?
    };
    if candidates.is_empty() {
        return Ok(());
    }
    let mut by_ident: std::collections::HashMap<String, Vec<LegacyLogEvent>> =
        std::collections::HashMap::new();
    {
        let mut statement = conn
            .prepare(
                "SELECT idx, kind, payload, created_at FROM log_entries
                 WHERE kind IN ('transfer-offer', 'transfer-accept', 'publish')
                 ORDER BY idx ASC",
            )
            .map_err(|err| format!("failed to prepare legacy log scan: {err}"))?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })
            .map_err(|err| format!("failed to scan the log for legacy transfers: {err}"))?;
        for row in rows {
            let (idx, kind, payload, created_at) =
                row.map_err(|err| format!("failed to read log entry: {err}"))?;
            let value: serde_json::Value = serde_json::from_str(&payload)
                .map_err(|err| format!("malformed {kind} log payload at index {idx}: {err}"))?;
            let field = |name: &str| -> Result<String, String> {
                value
                    .get(name)
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
                    .ok_or_else(|| format!("{kind} log payload at index {idx} has no {name}"))
            };
            let event = match kind.as_str() {
                "transfer-offer" => LegacyLogEvent::Offer { created_at },
                "transfer-accept" => LegacyLogEvent::Accept {
                    to: field("to")?,
                    created_at,
                },
                _ => LegacyLogEvent::Publish {
                    version: field("version")?,
                    hash: field("hash")?,
                },
            };
            by_ident.entry(field("ident")?).or_default().push(event);
        }
    }
    for (package_id, ident, stored_owner_id) in candidates {
        let Some(events) = by_ident.get(&ident) else {
            continue;
        };
        if !events
            .iter()
            .any(|event| matches!(event, LegacyLogEvent::Accept { .. }))
        {
            continue;
        }
        let tx = conn
            .transaction()
            .map_err(|err| format!("failed to start legacy transfer replay: {err}"))?;
        let owner_by_name = |name: &str| -> Result<Option<(i64, String)>, String> {
            tx.query_row(
                "SELECT id, owner_display FROM owners WHERE owner_folded = ?1",
                params![fold_owner(name)],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|err| format!("failed to load owner: {err}"))
        };
        let prefix = ident.split_once('#').map(|(owner, _)| owner).unwrap_or("");
        let Some((mut owner_id, mut owner_display)) = owner_by_name(prefix)? else {
            return Err(format!(
                "transfer log for {ident} starts at unknown owner {prefix}"
            ));
        };
        let mut sequence: i64 = 0;
        let mut offered_at: Option<i64> = None;
        for event in events {
            match event {
                LegacyLogEvent::Offer { created_at } => offered_at = Some(*created_at),
                LegacyLogEvent::Accept { to, created_at } => {
                    let Some((to_id, to_display)) = owner_by_name(to)? else {
                        return Err(format!("transfer log for {ident} names unknown owner {to}"));
                    };
                    tx.execute(
                        "INSERT INTO transfer_history
                             (package_id, sequence, from_owner_id, to_owner_id,
                              offered_at, accepted_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                        params![
                            package_id,
                            sequence,
                            owner_id,
                            to_id,
                            offered_at.unwrap_or(*created_at),
                            created_at
                        ],
                    )
                    .map_err(|err| format!("failed to record legacy transfer: {err}"))?;
                    owner_id = to_id;
                    owner_display = to_display;
                    sequence += 1;
                    offered_at = None;
                }
                LegacyLogEvent::Publish { version, hash } => {
                    tx.execute(
                        "UPDATE package_versions SET transfer_epoch = ?1
                         WHERE package_id = ?2 AND version = ?3 AND hash = ?4",
                        params![sequence, package_id, version, hash],
                    )
                    .map_err(|err| format!("failed to record legacy release epoch: {err}"))?;
                }
            }
        }
        if owner_id != stored_owner_id {
            let stored_display: String = tx
                .query_row(
                    "SELECT owner_display FROM owners WHERE id = ?1",
                    params![stored_owner_id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|err| format!("failed to load owner: {err}"))?
                .unwrap_or_else(|| format!("unknown owner id {stored_owner_id}"));
            return Err(format!(
                "transfer log for {ident} ends at {owner_display} but the package is owned by \
                 {stored_display}"
            ));
        }
        tx.commit()
            .map_err(|err| format!("failed to commit legacy transfer replay: {err}"))?;
    }
    Ok(())
}

/// One transparency-log entry the legacy transfer replay reads.
enum LegacyLogEvent {
    Offer { created_at: i64 },
    Accept { to: String, created_at: i64 },
    Publish { version: String, hash: String },
}

pub(super) fn json_value(value: &str) -> String {
    serde_json::to_string(value).expect("JSON string encoding cannot fail")
}

/// Add a column to a table if it is not already present (idempotent
/// migration). SQLite has no `ADD COLUMN IF NOT EXISTS`, so a "duplicate
/// column name" error is treated as success.
pub(super) fn add_column_if_missing(
    conn: &Connection,
    table: &str,
    column_def: &str,
) -> Result<(), String> {
    match conn.execute(&format!("ALTER TABLE {table} ADD COLUMN {column_def}"), []) {
        Ok(_) => Ok(()),
        Err(err) if err.to_string().contains("duplicate column name") => Ok(()),
        Err(err) => Err(format!("failed to add column to {table}: {err}")),
    }
}
