use crate::abi::VendorBlobRef;
use crate::crypto;
use crate::validation::{fold_owner, validate_ident, validate_owner_name, validate_version};
use rand::RngCore;
use rusqlite::{params, Connection, OptionalExtension};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

#[derive(Clone)]
pub struct Store {
    conn: Arc<Mutex<Connection>>,
    /// The signed tree head, memoised by log size (bug-579).
    head_cache: Arc<Mutex<Option<CachedLogHead>>>,
}

/// A signed tree head, valid for exactly the log size it was computed at.
///
/// The transparency log is append-only and dense — nothing in the crate
/// deletes or rewrites a `log_entries` row (the only `UPDATE`/`DROP` against
/// that table are tests that deliberately corrupt it to prove the readers
/// degrade to an error) — so `size` uniquely determines the root, and the size
/// is a sound cache key. `COUNT(*)` on a B-tree is far cheaper than loading and
/// re-hashing every leaf, which is what this avoids.
#[derive(Clone)]
struct CachedLogHead {
    size: i64,
    root: [u8; 32],
    signature: Vec<u8>,
}

/// `auth_challenges.purpose` for a challenge minted by `/auth/challenge` (an
/// auth key, completed by `/auth/login`).
const CHALLENGE_PURPOSE_LOGIN: &str = "login";
/// `auth_challenges.purpose` for a challenge minted by
/// `/machines/revoke/challenge` (the ident key, completed by `/machines/revoke`).
const CHALLENGE_PURPOSE_REVOKE: &str = "revoke";
/// `keys.role` values, as the schema spells them.
const KEY_ROLE_AUTH: &str = "auth";
const KEY_ROLE_IDENT: &str = "ident";

/// The human-facing metadata a publish records alongside the version row
/// (plan-61-A §4). Grouped rather than passed as loose parameters because
/// `publish_package_version` was already at the argument-count lint's limit,
/// and because plan-61-E adds `description` to exactly this set.
///
/// `None` means the publisher provided nothing. An empty string is normalized
/// to `None` by the caller, so "absent" and "present but empty" stay one fact.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PublishMetadata {
    pub author: Option<String>,
    pub url: Option<String>,
    /// The `description` from MFPC section 18 (plan-61-E). `None` for a package
    /// published before plan-61-D, which simply carries no section 18 — that is
    /// a normal outcome, not a failure.
    pub description: Option<String>,
    /// The raw MFPC section-17 bytes (the `DOC` table) to record in
    /// `package_version_docs` (plan-126-E). `None` for an undocumented package,
    /// **and** for one whose section 17 failed to decode: documentation "does not
    /// affect execution or the ABI", so a malformed doc table must never reject a
    /// signed package -- it is simply not recorded. Written inside the same
    /// transaction as the version row.
    pub docs: Option<Vec<u8>>,
    /// The fingerprint of the publisher's ident key the release header names
    /// (plan-168-A §4.3), recorded as `package_versions.publisher_ident_key_id`.
    /// It must be one of the publishing owner's ident keys; an unknown
    /// fingerprint refuses the publish. `None` records no key (a caller that
    /// has no validated header, such as a store-level test).
    pub publisher_ident_fingerprint: Option<String>,
}

/// One `package_version_targets` row as `Store::target_rows_for_test` yields it:
/// `(os, arch, libc, lib_type, source)`. Test-only; see that method.
#[cfg(test)]
pub(crate) type TargetRowForTest = (String, Option<String>, Option<String>, String, String);

#[derive(Debug, Clone)]
pub struct OwnerRecord {
    pub id: i64,
    pub owner_display: String,
}

#[derive(Debug, Clone)]
pub struct KeyRecord {
    pub id: i64,
    pub public_key: Vec<u8>,
    pub fingerprint: String,
}

#[derive(Debug, Clone)]
pub struct PublishedVersion {
    pub ident: String,
    pub version: String,
    pub hash: String,
    pub published_at: i64,
    pub state: String,
    /// The publish's transparency-log entry (plan-23-B3).
    pub log_entry: LogEntryRef,
}

/// One `package_blobs` row, as the garbage collector sees it (plan-49).
///
/// `path` is the `blob_ref` recorded when the row was written — informational
/// only, and a stale one if the datapath moved. The collector addresses the
/// backing object by `hash` + `kind`, exactly the way `GET /blob/<hash>` does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobRow {
    pub hash: String,
    pub path: String,
    pub kind: String,
    pub created_at: i64,
}

impl BlobRow {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(BlobRow {
            hash: row.get(0)?,
            path: row.get(1)?,
            kind: row.get(2)?,
            created_at: row.get(3)?,
        })
    }
}

#[derive(Debug, Clone)]
pub struct ChallengeRecord {
    pub id: String,
    pub owner_id: i64,
    pub key_id: i64,
    pub nonce: Vec<u8>,
    pub expires_at: i64,
    pub used_at: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct NewSession {
    pub owner_id: i64,
    pub key_id: i64,
    pub jwt_id: String,
    pub issued_at: i64,
    pub expires_at: i64,
}

/// The outcome of a machine-link fetch (bug-583). `Unapproved` is separated
/// from `Missing` on purpose: the caller found a live pairing but did not
/// prove possession of the pairing code, and nothing was consumed.
#[derive(Debug, Clone)]
pub enum PairingFetch {
    Relayed { blob: Vec<u8>, salt: Vec<u8> },
    Missing,
    Unapproved,
}

/// Tighten `path` so no group/other bit is set (bug-586).
///
/// The metadata database holds `server_keys.private_key`, the
/// `server_secrets.secret` used to sign sessions, and
/// `registry_config.{snapshot_private,timestamp_private}` — all in plaintext.
/// SQLite creates the database and its `-wal`/`-shm` sidecars with the process
/// umask, so under the ordinary `umask 022` they land at `0644` inside a `0755`
/// directory: any other account on the host or with access to the mounted
/// volume can copy the server's signing key and forge attestations, sessions
/// and signed metadata.
///
/// This only ever REMOVES access. A path that is already private is left
/// untouched (it may legitimately be `0400`, or owned differently), so this can
/// never widen an operator's deliberate hardening. When a path IS exposed and
/// cannot be tightened — typically a volume owned by another UID — the open
/// fails with operator guidance rather than serving with the keys readable,
/// because "start anyway" is exactly the silent widening this must not do.
#[cfg(unix)]
fn make_private(path: &Path, mode: u32, what: &str) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        // Not every sidecar exists at every call; absence is not exposure.
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => {
            return Err(format!(
                "failed to inspect {what} '{}': {err}",
                path.display()
            ))
        }
    };
    let current = metadata.permissions().mode() & 0o777;
    if current & 0o077 == 0 {
        return Ok(());
    }
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).map_err(|err| {
        format!(
            "{what} '{}' is accessible to other accounts (mode {current:o}) and its permissions \
             could not be tightened to {mode:o}: {err}. It holds the server signing key, the \
             session secret and the online metadata keys in plaintext. Repair the volume, e.g. \
             `chown -R mfb:mfb <dir> && chmod 700 <dir> && chmod 600 <dir>/meta.db*`, then restart.",
            path.display()
        )
    })
}

/// The metadata directory, the database, and its SQLite sidecars, all made
/// private to the service account (bug-586).
#[cfg(unix)]
fn harden_private_state(db_dir: Option<&Path>, dbpath: &Path) -> Result<(), String> {
    if let Some(dir) = db_dir {
        make_private(dir, 0o700, "database directory")?;
    }
    make_private(dbpath, 0o600, "database")?;
    // `-wal` and `-shm` are written by SQLite with the same umask and carry the
    // same rows; protecting only `meta.db` would leave the secrets readable in
    // the write-ahead log.
    for suffix in ["-wal", "-shm"] {
        let mut sidecar = dbpath.as_os_str().to_os_string();
        sidecar.push(suffix);
        make_private(Path::new(&sidecar), 0o600, "database journal")?;
    }
    Ok(())
}

/// Non-Unix targets have no POSIX mode to enforce. This is deliberately a
/// no-op rather than a best-effort imitation: claiming enforcement that did not
/// happen is worse than stating plainly that it did not. The deployed server is
/// Linux (`repository/Dockerfile`); see `repository/DEPLOY.md`.
#[cfg(not(unix))]
fn harden_private_state(_db_dir: Option<&Path>, _dbpath: &Path) -> Result<(), String> {
    Ok(())
}

pub struct OpenedRepository {
    pub store: Store,
    pub packages_dir: PathBuf,
}

impl Store {
    pub fn open_repository(dbpath: &Path, datapath: &Path) -> Result<OpenedRepository, String> {
        if dbpath.exists() && !dbpath.is_file() {
            return Err(format!(
                "database path '{}' exists but is not a file",
                dbpath.display()
            ));
        }
        let mut db_dir = None;
        if let Some(parent) = dbpath.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                format!(
                    "failed to create database directory '{}': {err}",
                    parent.display()
                )
            })?;
            if !parent.as_os_str().is_empty() {
                db_dir = Some(parent);
            }
        }
        // bug-586: tighten the directory BEFORE the database is created. The
        // file itself is created by SQLite under the process umask, so there is
        // a window in which it exists at 0644; a 0700 directory means no other
        // account can traverse into it during that window.
        harden_private_state(db_dir, dbpath)?;
        // A remote (`s3://…`) data path has no local directory to create; the
        // blob backend is constructed separately (see `blobstore`). Operator
        // subcommands that only touch the metadata DB still work in S3 mode.
        let is_remote = datapath
            .to_str()
            .is_some_and(|path| path.starts_with("s3://"));
        if !is_remote {
            if datapath.exists() && !datapath.is_dir() {
                return Err(format!(
                    "data path '{}' exists but is not a directory",
                    datapath.display()
                ));
            }
            fs::create_dir_all(datapath).map_err(|err| {
                format!(
                    "failed to create data directory '{}': {err}",
                    datapath.display()
                )
            })?;
        }
        let conn = Connection::open(dbpath)
            .map_err(|err| format!("failed to open '{}': {err}", dbpath.display()))?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(|err| format!("failed to enable foreign keys: {err}"))?;
        // WAL + a busy timeout (plan-10-D2 hardening): readers no longer block
        // on the writer at the SQLite level, and a brief writer contention
        // waits rather than failing, so concurrent publishes/reads do not
        // serialize behind a single global write lock.
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|err| format!("failed to enable WAL: {err}"))?;
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|err| format!("failed to set busy timeout: {err}"))?;
        // bug-586: now that the database and its WAL/SHM sidecars exist, make
        // them private too — before `migrate`/`ensure_server_secret`/
        // `ensure_server_keypair` below write any key material into them.
        harden_private_state(db_dir, dbpath)?;
        let store = Store {
            conn: Arc::new(Mutex::new(conn)),
            head_cache: Arc::new(Mutex::new(None)),
        };
        store.migrate()?;
        store.ensure_server_secret()?;
        store.ensure_server_keypair()?;
        Ok(OpenedRepository {
            store,
            packages_dir: datapath.to_path_buf(),
        })
    }

    /// Acquire the connection guard, recovering from a poisoned lock rather than
    /// failing every subsequent request forever. A Rust panic while the lock was
    /// held would otherwise poison the `Mutex` permanently — a single reachable
    /// panic in any critical section becoming a full-service DoS (bug-264 /
    /// REPO-09). The SQLite connection itself stays usable across a panic:
    /// rusqlite statements are transactional and any in-flight transaction rolls
    /// back when its guard drops, so the correct response to a `PoisonError` is to
    /// take the inner guard and carry on. The failed request has already errored;
    /// other requests continue to serve.
    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Every published version as `(package_version_id, ident, version, hash)`,
    /// for the `backfill-metadata` sweep (plan-61-A Phase 4).
    ///
    /// This is the one read accessor A ships. It is not the sub-plan's "no read
    /// accessors" rule leaking: that rule is about the `package_metadata` /
    /// `package_targets` shapes plan-61-B was expected to consume and does not.
    /// Backfill is a *write* ceremony that has to know what to rewrite.
    pub fn all_package_versions(&self) -> Result<Vec<(i64, String, String, String)>, String> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT pv.id, p.ident, pv.version, pv.hash
                 FROM package_versions pv
                 JOIN packages p ON p.id = pv.package_id
                 ORDER BY pv.id",
            )
            .map_err(|err| format!("failed to enumerate package versions: {err}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .map_err(|err| format!("failed to enumerate package versions: {err}"))?;
        let mut versions = Vec::new();
        for row in rows {
            versions.push(row.map_err(|err| format!("failed to read package version: {err}"))?);
        }
        Ok(versions)
    }

    /// Rewrite one version's captured metadata: replace its target rows and set
    /// its `author`/`url`, in a single transaction (plan-61-A Phase 4).
    ///
    /// Delete-then-insert is what makes the backfill idempotent — a second run
    /// must not double every target row. Scoped to one version so a failure
    /// mid-sweep leaves earlier versions committed and the rest untouched.
    pub fn replace_version_metadata(
        &self,
        package_version_id: i64,
        metadata: &PublishMetadata,
        vendor_blobs: &[VendorBlobRef],
    ) -> Result<(), String> {
        let mut conn = self.conn();
        let tx = conn
            .transaction()
            .map_err(|err| format!("failed to start backfill transaction: {err}"))?;
        tx.execute(
            "UPDATE package_versions SET author = ?2, url = ?3, description = ?4 \
             WHERE id = ?1",
            params![
                package_version_id,
                metadata.author,
                metadata.url,
                metadata.description
            ],
        )
        .map_err(|err| format!("failed to update version metadata: {err}"))?;
        tx.execute(
            "DELETE FROM package_version_targets WHERE package_version_id = ?1",
            params![package_version_id],
        )
        .map_err(|err| format!("failed to clear version targets: {err}"))?;
        insert_version_targets(&tx, package_version_id, vendor_blobs)?;
        tx.commit()
            .map_err(|err| format!("failed to commit backfill: {err}"))
    }

    /// The stored `(author, url)` for one published version. Test-only, for the
    /// same reason as `target_rows_for_test`: the publish-path tests live in
    /// `server.rs` and plan-61-A ships no real read accessors.
    #[cfg(test)]
    pub(crate) fn version_metadata_for_test(
        &self,
        ident: &str,
        version: &str,
    ) -> (Option<String>, Option<String>) {
        self.conn()
            .query_row(
                "SELECT pv.author, pv.url FROM package_versions pv
                 JOIN packages p ON p.id = pv.package_id
                 WHERE p.ident = ?1 AND pv.version = ?2",
                params![ident, version],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap()
    }

    /// The stored `description` for one published version. Test-only.
    #[cfg(test)]
    pub(crate) fn version_description_for_test(
        &self,
        ident: &str,
        version: &str,
    ) -> Option<String> {
        self.conn()
            .query_row(
                "SELECT pv.description FROM package_versions pv
                 JOIN packages p ON p.id = pv.package_id
                 WHERE p.ident = ?1 AND pv.version = ?2",
                params![ident, version],
                |row| row.get(0),
            )
            .unwrap()
    }

    /// Every `package_version_targets` row as
    /// `(os, arch, libc, lib_type, source)`, ordered by insertion.
    ///
    /// Test-only. plan-61-A deliberately ships **no** read accessors — plan-61-B
    /// defines its own `package_detail`/`package_audit` queries over these
    /// tables — but the publish-path tests in `server.rs` live outside this
    /// module and cannot reach the private connection.
    #[cfg(test)]
    pub(crate) fn target_rows_for_test(&self) -> Vec<TargetRowForTest> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT os, arch, libc, lib_type, source
                 FROM package_version_targets ORDER BY rowid",
            )
            .unwrap();
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            })
            .unwrap()
            .map(|row| row.unwrap())
            .collect();
        rows
    }

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

    /// Register an owner with the two client-held public keys (plan-23 §3.1):
    /// a per-machine `auth` key and the account `ident` key. Each key must
    /// carry a proof-of-possession signed over the role-discriminated
    /// registration message, so an auth proof can never be replayed as an
    /// ident proof (or vice versa). The server never sees a private key.
    pub fn register_owner(
        &self,
        owner: &str,
        auth_key: &[u8],
        auth_proof: &[u8],
        ident_key: &[u8],
        ident_proof: &[u8],
    ) -> Result<(OwnerRecord, KeyRecord, KeyRecord), String> {
        validate_owner_name(owner)?;
        let auth_message = crypto::registration_message(crypto::ROLE_AUTH, owner, auth_key);
        crypto::verify(auth_key, &auth_message, auth_proof)
            .map_err(|_| "invalid auth proof-of-possession signature".to_string())?;
        let ident_message = crypto::registration_message(crypto::ROLE_IDENT, owner, ident_key);
        crypto::verify(ident_key, &ident_message, ident_proof)
            .map_err(|_| "invalid ident proof-of-possession signature".to_string())?;

        let folded = fold_owner(owner);
        let auth_fingerprint = crypto::fingerprint(auth_key);
        let ident_fingerprint = crypto::fingerprint(ident_key);
        let now = now_unix();
        let mut conn = self.conn();
        let tx = conn
            .transaction()
            .map_err(|err| format!("failed to start registration transaction: {err}"))?;
        tx.execute(
            "INSERT INTO owners (owner_display, owner_folded, created_at, status)
             VALUES (?1, ?2, ?3, 'active')",
            params![owner, folded, now],
        )
        .map_err(|err| {
            if is_unique_violation(&err) {
                format!("owner name '{owner}' is already in use")
            } else {
                format!("failed to register owner: {err}")
            }
        })?;
        let owner_id = tx.last_insert_rowid();
        let auth_key_id = insert_key_tx(
            &tx,
            owner_id,
            KEY_ROLE_AUTH,
            auth_key,
            &auth_fingerprint,
            now,
            "failed to register auth key",
        )?;
        let ident_key_id = insert_key_tx(
            &tx,
            owner_id,
            KEY_ROLE_IDENT,
            ident_key,
            &ident_fingerprint,
            now,
            "failed to register ident key",
        )?;
        append_log_tx(
            &tx,
            "register",
            &format!(
                "{{\"owner\":{},\"authFingerprint\":{},\"identFingerprint\":{}}}",
                json_value(owner),
                json_value(&auth_fingerprint),
                json_value(&ident_fingerprint),
            ),
        )?;
        tx.commit()
            .map_err(|err| format!("failed to commit registration: {err}"))?;

        Ok((
            OwnerRecord {
                id: owner_id,
                owner_display: owner.to_string(),
            },
            KeyRecord {
                id: auth_key_id,
                public_key: auth_key.to_vec(),
                fingerprint: auth_fingerprint,
            },
            KeyRecord {
                id: ident_key_id,
                public_key: ident_key.to_vec(),
                fingerprint: ident_fingerprint,
            },
        ))
    }

    pub fn owner_with_auth_key(
        &self,
        owner: &str,
    ) -> Result<Option<(OwnerRecord, KeyRecord)>, String> {
        self.owner_with_key(owner, "auth")
    }

    pub fn owner_with_ident_key(
        &self,
        owner: &str,
    ) -> Result<Option<(OwnerRecord, KeyRecord)>, String> {
        self.owner_with_key(owner, "ident")
    }

    /// Look up one of the owner's current auth keys by fingerprint. Machines
    /// are equals (plan-23 §2): an account holds one current auth key per
    /// linked machine, so auth-key resolution is always fingerprint-scoped.
    pub fn owner_auth_key_by_fingerprint(
        &self,
        owner: &str,
        fingerprint: &str,
    ) -> Result<Option<(OwnerRecord, KeyRecord)>, String> {
        let folded = fold_owner(owner);
        let conn = self.conn();
        conn.query_row(
            "SELECT o.id, o.owner_display, k.id, k.public_key, k.fingerprint
             FROM owners o
             JOIN keys k ON k.owner_id = o.id
             WHERE o.owner_folded = ?1
               AND o.status = 'active'
               AND k.role = 'auth'
               AND k.status = 'current'
               AND k.fingerprint = ?2",
            params![folded, fingerprint],
            |row| {
                Ok((
                    OwnerRecord {
                        id: row.get(0)?,
                        owner_display: row.get(1)?,
                    },
                    KeyRecord {
                        id: row.get(2)?,
                        public_key: row.get(3)?,
                        fingerprint: row.get(4)?,
                    },
                ))
            },
        )
        .optional()
        .map_err(|err| format!("failed to load owner: {err}"))
    }

    fn owner_with_key(
        &self,
        owner: &str,
        role: &str,
    ) -> Result<Option<(OwnerRecord, KeyRecord)>, String> {
        let folded = fold_owner(owner);
        let conn = self.conn();
        conn.query_row(
            "SELECT o.id, o.owner_display, k.id, k.public_key, k.fingerprint
             FROM owners o
             JOIN keys k ON k.owner_id = o.id
             WHERE o.owner_folded = ?1
               AND o.status = 'active'
               AND k.role = ?2
               AND k.status = 'current'",
            params![folded, role],
            |row| {
                Ok((
                    OwnerRecord {
                        id: row.get(0)?,
                        owner_display: row.get(1)?,
                    },
                    KeyRecord {
                        id: row.get(2)?,
                        public_key: row.get(3)?,
                        fingerprint: row.get(4)?,
                    },
                ))
            },
        )
        .optional()
        .map_err(|err| format!("failed to load owner: {err}"))
    }

    /// Record an attestation issuance (plan-23 §3.3 step 2): every `/signing`
    /// request is logged before the server signs, so a stolen auth session
    /// requesting attestations always leaves a trace.
    pub fn record_signing_request(
        &self,
        owner_id: i64,
        ident: &str,
        version: &str,
        signing_fingerprint: &str,
    ) -> Result<(), String> {
        let mut conn = self.conn();
        let tx = conn
            .transaction()
            .map_err(|err| format!("failed to start signing transaction: {err}"))?;
        tx.execute(
            "INSERT INTO signing_requests (owner_id, ident, version, signing_fingerprint, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![owner_id, ident, version, signing_fingerprint, now_unix()],
        )
        .map_err(|err| format!("failed to record signing request: {err}"))?;
        let owner_display: String = tx
            .query_row(
                "SELECT owner_display FROM owners WHERE id = ?1",
                params![owner_id],
                |row| row.get(0),
            )
            .map_err(|err| format!("failed to load owner: {err}"))?;
        append_log_tx(
            &tx,
            "attestation",
            &format!(
                "{{\"owner\":{},\"ident\":{},\"version\":{},\"signingFingerprint\":{}}}",
                json_value(&owner_display),
                json_value(ident),
                json_value(version),
                json_value(signing_fingerprint),
            ),
        )?;
        tx.commit()
            .map_err(|err| format!("failed to commit signing request: {err}"))?;
        Ok(())
    }

    /// Legacy single-machine helper kept for tests: challenges the owner's
    /// (sole) current auth key.
    pub fn create_challenge(&self, owner: &str) -> Result<ChallengeRecord, String> {
        validate_owner_name(owner)?;
        let Some((owner, key)) = self.owner_with_auth_key(owner)? else {
            return Err("unknown owner".to_string());
        };
        self.create_challenge_for_key(owner.id, key.id, CHALLENGE_PURPOSE_LOGIN)
    }

    /// Challenge a specific machine's auth key (plan-23-B: an account holds
    /// one current auth key per linked machine).
    pub fn create_auth_challenge(
        &self,
        owner: &str,
        fingerprint: &str,
    ) -> Result<ChallengeRecord, String> {
        validate_owner_name(owner)?;
        let Some((owner, key)) = self.owner_auth_key_by_fingerprint(owner, fingerprint)? else {
            return Err("mismatched local key fingerprint".to_string());
        };
        self.create_challenge_for_key(owner.id, key.id, CHALLENGE_PURPOSE_LOGIN)
    }

    /// Challenge the owner's ident key: proves possession of the account
    /// identity for ident-authorized operations (auth-key revocation). Bound
    /// to the `revoke` purpose, so it can never complete a login (bug-493).
    pub fn create_ident_challenge(&self, owner: &str) -> Result<ChallengeRecord, String> {
        validate_owner_name(owner)?;
        let Some((owner, key)) = self.owner_with_ident_key(owner)? else {
            return Err("unknown owner".to_string());
        };
        self.create_challenge_for_key(owner.id, key.id, CHALLENGE_PURPOSE_REVOKE)
    }

    /// Mint a challenge row for `key_id`, bound to `purpose` (bug-493): the
    /// completer for one purpose refuses a challenge minted for the other.
    fn create_challenge_for_key(
        &self,
        owner_id: i64,
        key_id: i64,
        purpose: &str,
    ) -> Result<ChallengeRecord, String> {
        let id = Uuid::new_v4().to_string();
        let mut nonce = vec![0u8; 32];
        rand::thread_rng().fill_bytes(&mut nonce);
        let created_at = now_unix();
        let expires_at = created_at + 300;
        let conn = self.conn();
        conn.execute(
            "INSERT INTO auth_challenges
             (id, owner_id, key_id, nonce, purpose, created_at, expires_at, used_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL)",
            params![id, owner_id, key_id, nonce, purpose, created_at, expires_at],
        )
        .map_err(|err| format!("failed to create auth challenge: {err}"))?;
        Ok(ChallengeRecord {
            id,
            owner_id,
            key_id,
            nonce,
            expires_at,
            used_at: None,
        })
    }

    /// Store a machine-link relay blob (plan-23 §3.2): a single-use,
    /// short-TTL ciphertext the server cannot read, keyed by the
    /// code-derived lookup. Returns the expiry time.
    pub fn store_pairing_blob(
        &self,
        owner_id: i64,
        lookup: &str,
        blob: &[u8],
        salt: &[u8],
        approval_key: &[u8],
    ) -> Result<i64, String> {
        let now = now_unix();
        let expires_at = now + 600;
        let conn = self.conn();
        // Housekeeping: expired blobs are dead weight; drop them here so no
        // background reaper is needed (full rate-limiting is plan-10-D).
        conn.execute(
            "DELETE FROM pairing_blobs WHERE expires_at <= ?1",
            params![now],
        )
        .map_err(|err| format!("failed to clear expired pairing blobs: {err}"))?;
        conn.execute(
            "INSERT INTO pairing_blobs
                 (owner_id, lookup, blob, salt, approval_key, created_at, expires_at, used_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL)",
            params![owner_id, lookup, blob, salt, approval_key, now, expires_at],
        )
        .map_err(|err| {
            if is_unique_violation(&err) {
                "a pairing with this code is already pending; generate a new code".to_string()
            } else {
                format!("failed to store pairing blob: {err}")
            }
        })?;
        Ok(expires_at)
    }

    /// Fetch-and-consume a pairing blob: single use, refused after expiry.
    /// The stored ciphertext is destroyed as it is handed out.
    ///
    /// bug-583: `approve` is handed the row's code-derived approval public key
    /// and decides, INSIDE the transaction, whether this caller proved
    /// possession of the pairing code. A refusal rolls the transaction back,
    /// so a caller holding only the (server-visible) lookup can neither take
    /// the blob nor burn the honest machine's pending pairing. Consumption is
    /// therefore atomic with authorization.
    pub fn take_pairing_blob<F>(
        &self,
        owner: &str,
        lookup: &str,
        approve: F,
    ) -> Result<PairingFetch, String>
    where
        F: FnOnce(&[u8]) -> bool,
    {
        let folded = fold_owner(owner);
        let now = now_unix();
        let mut conn = self.conn();
        let tx = conn
            .transaction()
            .map_err(|err| format!("failed to start pairing transaction: {err}"))?;
        let row: Option<(i64, Vec<u8>, Vec<u8>, Vec<u8>)> = tx
            .query_row(
                "SELECT p.id, p.blob, p.salt, p.approval_key
                 FROM pairing_blobs p
                 JOIN owners o ON o.id = p.owner_id
                 WHERE p.lookup = ?1
                   AND o.owner_folded = ?2
                   AND o.status = 'active'
                   AND p.used_at IS NULL
                   AND p.expires_at > ?3",
                params![lookup, folded, now],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()
            .map_err(|err| format!("failed to load pairing blob: {err}"))?;
        let Some((id, blob, salt, approval_key)) = row else {
            tx.commit().ok();
            return Ok(PairingFetch::Missing);
        };
        if !approve(&approval_key) {
            tx.rollback().ok();
            return Ok(PairingFetch::Unapproved);
        }
        tx.execute(
            "UPDATE pairing_blobs SET used_at = ?1, blob = x'' WHERE id = ?2",
            params![now, id],
        )
        .map_err(|err| format!("failed to consume pairing blob: {err}"))?;
        tx.commit()
            .map_err(|err| format!("failed to commit pairing fetch: {err}"))?;
        Ok(PairingFetch::Relayed { blob, salt })
    }

    /// Register an additional machine's auth key on an existing account
    /// (plan-23 §3.2 step 1). The proof must be role-separated exactly like
    /// registration.
    pub fn add_auth_key(
        &self,
        owner: &str,
        public_key: &[u8],
        proof: &[u8],
    ) -> Result<(OwnerRecord, KeyRecord), String> {
        validate_owner_name(owner)?;
        let Some((owner, _ident)) = self.owner_with_ident_key(owner)? else {
            return Err("unknown owner".to_string());
        };
        let message =
            crypto::registration_message(crypto::ROLE_AUTH, &owner.owner_display, public_key);
        crypto::verify(public_key, &message, proof)
            .map_err(|_| "invalid auth proof-of-possession signature".to_string())?;
        let fingerprint = crypto::fingerprint(public_key);
        let mut conn = self.conn();
        let tx = conn
            .transaction()
            .map_err(|err| format!("failed to start link transaction: {err}"))?;
        let key_id = insert_key_tx(
            &tx,
            owner.id,
            KEY_ROLE_AUTH,
            public_key,
            &fingerprint,
            now_unix(),
            "failed to register machine auth key",
        )?;
        append_log_tx(
            &tx,
            "link",
            &format!(
                "{{\"owner\":{},\"authFingerprint\":{}}}",
                json_value(&owner.owner_display),
                json_value(&fingerprint),
            ),
        )?;
        tx.commit()
            .map_err(|err| format!("failed to commit link: {err}"))?;
        Ok((
            owner,
            KeyRecord {
                id: key_id,
                public_key: public_key.to_vec(),
                fingerprint,
            },
        ))
    }

    /// Rotate the account ident (plan-23-B2): the OLD ident signs the chain
    /// link naming its successor, and the NEW ident proves possession with a
    /// role-separated registration proof. The old key becomes `past` and the
    /// signed link is recorded so consumers can follow the chain.
    pub fn rotate_ident(
        &self,
        owner: &str,
        new_public: &[u8],
        chain_signature: &[u8],
        possession_proof: &[u8],
    ) -> Result<(OwnerRecord, KeyRecord), String> {
        validate_owner_name(owner)?;
        let Some((owner, old_key)) = self.owner_with_ident_key(owner)? else {
            return Err("unknown owner".to_string());
        };
        let chain_message =
            crypto::ident_rotation_message(&owner.owner_display, &old_key.fingerprint, new_public);
        crypto::verify(&old_key.public_key, &chain_message, chain_signature)
            .map_err(|_| "invalid ident chain signature".to_string())?;
        let possession_message =
            crypto::registration_message(crypto::ROLE_IDENT, &owner.owner_display, new_public);
        crypto::verify(new_public, &possession_message, possession_proof)
            .map_err(|_| "invalid ident proof-of-possession signature".to_string())?;

        let fingerprint = crypto::fingerprint(new_public);
        let now = now_unix();
        let mut conn = self.conn();
        let tx = conn
            .transaction()
            .map_err(|err| format!("failed to start rotation transaction: {err}"))?;
        tx.execute(
            "UPDATE keys SET status = 'past', revoked_at = ?1 WHERE id = ?2",
            params![now, old_key.id],
        )
        .map_err(|err| format!("failed to retire ident key: {err}"))?;
        let new_key_id = insert_key_tx(
            &tx,
            owner.id,
            KEY_ROLE_IDENT,
            new_public,
            &fingerprint,
            now,
            "failed to register rotated ident key",
        )?;
        tx.execute(
            "INSERT INTO ident_chain (owner_id, old_key_id, new_key_id, signature, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![owner.id, old_key.id, new_key_id, chain_signature, now],
        )
        .map_err(|err| format!("failed to record ident chain link: {err}"))?;
        append_log_tx(
            &tx,
            "rotate",
            &format!(
                "{{\"owner\":{},\"oldIdentFingerprint\":{},\"newIdentFingerprint\":{}}}",
                json_value(&owner.owner_display),
                json_value(&old_key.fingerprint),
                json_value(&fingerprint),
            ),
        )?;
        tx.commit()
            .map_err(|err| format!("failed to commit ident rotation: {err}"))?;
        Ok((
            owner,
            KeyRecord {
                id: new_key_id,
                public_key: new_public.to_vec(),
                fingerprint,
            },
        ))
    }

    /// Re-anchor ceremony (plan-23 §3.6, total ident loss): a registry
    /// OPERATOR action, deliberately not an HTTP route — it runs against the
    /// database after out-of-band verification. Binds the name to a fresh
    /// ident with **no chain link**, so clients holding the old pin fail
    /// hard instead of silently following.
    pub fn reanchor_ident(&self, owner: &str, new_public: &[u8]) -> Result<KeyRecord, String> {
        validate_owner_name(owner)?;
        let Some((owner, old_key)) = self.owner_with_ident_key(owner)? else {
            return Err("unknown owner".to_string());
        };
        if new_public.len() != crypto::PUBLIC_KEY_LEN {
            return Err("malformed ident public key".to_string());
        }
        let fingerprint = crypto::fingerprint(new_public);
        let now = now_unix();
        let mut conn = self.conn();
        let tx = conn
            .transaction()
            .map_err(|err| format!("failed to start re-anchor transaction: {err}"))?;
        tx.execute(
            "UPDATE keys SET status = 'past', revoked_at = ?1 WHERE id = ?2",
            params![now, old_key.id],
        )
        .map_err(|err| format!("failed to retire ident key: {err}"))?;
        let key_id = insert_key_tx(
            &tx,
            owner.id,
            KEY_ROLE_IDENT,
            new_public,
            &fingerprint,
            now,
            "failed to register re-anchored ident key",
        )?;
        append_log_tx(
            &tx,
            "reanchor",
            &format!(
                "{{\"owner\":{},\"oldIdentFingerprint\":{},\"newIdentFingerprint\":{}}}",
                json_value(&owner.owner_display),
                json_value(&old_key.fingerprint),
                json_value(&fingerprint),
            ),
        )?;
        tx.commit()
            .map_err(|err| format!("failed to commit re-anchor: {err}"))?;
        Ok(KeyRecord {
            id: key_id,
            public_key: new_public.to_vec(),
            fingerprint,
        })
    }

    /// The owner's ident chain, oldest link first: each entry carries the
    /// old/new public keys and the old key's signature over the rotation
    /// message. An empty chain plus a current key that differs from a
    /// consumer's pin means the ident was re-anchored.
    pub fn ident_chain(
        &self,
        owner: &str,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>, Vec<u8>, i64)>, String> {
        let folded = fold_owner(owner);
        let conn = self.conn();
        let mut statement = conn
            .prepare(
                "SELECT old_keys.public_key, new_keys.public_key, c.signature, c.created_at
                 FROM ident_chain c
                 JOIN owners o ON o.id = c.owner_id
                 JOIN keys old_keys ON old_keys.id = c.old_key_id
                 JOIN keys new_keys ON new_keys.id = c.new_key_id
                 WHERE o.owner_folded = ?1 AND o.status = 'active'
                 ORDER BY c.id ASC",
            )
            .map_err(|err| format!("failed to prepare chain query: {err}"))?;
        let rows = statement
            .query_map(params![folded], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .map_err(|err| format!("failed to load ident chain: {err}"))?;
        let mut chain = Vec::new();
        for row in rows {
            chain.push(row.map_err(|err| format!("failed to read ident chain: {err}"))?);
        }
        Ok(chain)
    }

    /// Revoke a machine's auth key and kill every session opened with it
    /// (plan-23 §3.6). Returns false when no current auth key matches.
    pub fn revoke_auth_key(&self, owner_id: i64, fingerprint: &str) -> Result<bool, String> {
        let now = now_unix();
        let mut conn = self.conn();
        let tx = conn
            .transaction()
            .map_err(|err| format!("failed to start revocation transaction: {err}"))?;
        let key_id: Option<i64> = tx
            .query_row(
                "SELECT id FROM keys
                 WHERE owner_id = ?1 AND role = 'auth' AND status = 'current' AND fingerprint = ?2",
                params![owner_id, fingerprint],
                |row| row.get(0),
            )
            .optional()
            .map_err(|err| format!("failed to load auth key: {err}"))?;
        let Some(key_id) = key_id else {
            tx.commit().ok();
            return Ok(false);
        };
        // bug-493: never revoke the account's last current machine key. No
        // ident-authorized route adds an auth key, so an account with none is
        // unrecoverable short of the operator `reanchor` ceremony. Publish
        // tokens are `auth` rows too but do not count as machines — a token
        // cannot enrol one (bug-492) — so an account left with only a token is
        // just as locked out. A token itself may be revoked here freely; the
        // guard is for machine keys. Checked inside the transaction so two
        // concurrent revocations cannot each see the other key as "remaining".
        let is_token: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM publish_tokens WHERE key_id = ?1)",
                params![key_id],
                |row| row.get(0),
            )
            .map_err(|err| format!("failed to classify auth key: {err}"))?;
        if !is_token {
            let remaining: i64 = tx
                .query_row(
                    "SELECT COUNT(*) FROM keys k
                     WHERE k.owner_id = ?1 AND k.role = 'auth' AND k.status = 'current'
                       AND k.id != ?2
                       AND NOT EXISTS (SELECT 1 FROM publish_tokens t WHERE t.key_id = k.id)",
                    params![owner_id, key_id],
                    |row| row.get(0),
                )
                .map_err(|err| format!("failed to count machine keys: {err}"))?;
            if remaining == 0 {
                return Err(
                    "cannot revoke the account's last auth key; link another machine first"
                        .to_string(),
                );
            }
        }
        tx.execute(
            "UPDATE keys SET status = 'revoked', revoked_at = ?1 WHERE id = ?2",
            params![now, key_id],
        )
        .map_err(|err| format!("failed to revoke auth key: {err}"))?;
        tx.execute(
            "UPDATE sessions SET revoked_at = ?1 WHERE key_id = ?2 AND revoked_at IS NULL",
            params![now, key_id],
        )
        .map_err(|err| format!("failed to revoke sessions: {err}"))?;
        let owner_display: String = tx
            .query_row(
                "SELECT owner_display FROM owners WHERE id = ?1",
                params![owner_id],
                |row| row.get(0),
            )
            .map_err(|err| format!("failed to load owner: {err}"))?;
        append_log_tx(
            &tx,
            "revoke",
            &format!(
                "{{\"owner\":{},\"authFingerprint\":{}}}",
                json_value(&owner_display),
                json_value(fingerprint),
            ),
        )?;
        tx.commit()
            .map_err(|err| format!("failed to commit revocation: {err}"))?;
        Ok(true)
    }

    /// Complete a login challenge: one minted by `/auth/challenge` for an auth
    /// key. A revocation challenge — the ident key's — is refused here even
    /// with a valid ident signature (bug-493).
    pub fn complete_challenge(
        &self,
        challenge_id: &str,
        signature: &[u8],
    ) -> Result<(OwnerRecord, KeyRecord), String> {
        self.complete_challenge_with(
            challenge_id,
            signature,
            CHALLENGE_PURPOSE_LOGIN,
            KEY_ROLE_AUTH,
            crypto::challenge_message,
        )
    }

    /// Complete an ident challenge whose signature covers the revocation
    /// message (challenge + the fingerprint being revoked). Revocation
    /// authority is the ident key alone (plan-23 §3.6): a login challenge for
    /// an auth key is refused on purpose and role, not merely on signature —
    /// the attacker holding that auth key can sign the revocation message
    /// perfectly well (bug-493 / audit-3 REPO-02).
    pub fn complete_revocation_challenge(
        &self,
        challenge_id: &str,
        signature: &[u8],
        fingerprint: &str,
    ) -> Result<(OwnerRecord, KeyRecord), String> {
        self.complete_challenge_with(
            challenge_id,
            signature,
            CHALLENGE_PURPOSE_REVOKE,
            KEY_ROLE_IDENT,
            |id, nonce| crypto::revocation_message(id, nonce, fingerprint),
        )
    }

    /// The one challenge completer. `purpose` must match what the challenge
    /// was minted with and `role` the challenged key's role; both are checked
    /// before the signature and before the row is burned, so a challenge
    /// presented to the wrong completer stays usable for its own.
    fn complete_challenge_with(
        &self,
        challenge_id: &str,
        signature: &[u8],
        purpose: &str,
        role: &str,
        message: impl Fn(&str, &[u8]) -> Vec<u8>,
    ) -> Result<(OwnerRecord, KeyRecord), String> {
        let mut conn = self.conn();
        let tx = conn
            .transaction()
            .map_err(|err| format!("failed to start login transaction: {err}"))?;
        let loaded = tx
            .query_row(
                "SELECT c.id, c.owner_id, c.key_id, c.nonce, c.expires_at, c.used_at,
                        o.owner_display, k.public_key, k.fingerprint, c.purpose, k.role
                 FROM auth_challenges c
                 JOIN owners o ON o.id = c.owner_id
                 JOIN keys k ON k.id = c.key_id
                 WHERE c.id = ?1",
                params![challenge_id],
                |row| {
                    Ok((
                        ChallengeRecord {
                            id: row.get(0)?,
                            owner_id: row.get(1)?,
                            key_id: row.get(2)?,
                            nonce: row.get(3)?,
                            expires_at: row.get(4)?,
                            used_at: row.get(5)?,
                        },
                        OwnerRecord {
                            id: row.get(1)?,
                            owner_display: row.get(6)?,
                        },
                        KeyRecord {
                            id: row.get(2)?,
                            public_key: row.get(7)?,
                            fingerprint: row.get(8)?,
                        },
                        row.get::<_, String>(9)?,
                        row.get::<_, String>(10)?,
                    ))
                },
            )
            .optional()
            .map_err(|err| format!("failed to load auth challenge: {err}"))?;
        let Some((challenge, owner, key, minted_purpose, key_role)) = loaded else {
            return Err("unknown challenge".to_string());
        };
        if minted_purpose != purpose {
            let operation = if purpose == CHALLENGE_PURPOSE_REVOKE {
                "revocation"
            } else {
                purpose
            };
            return Err(format!("challenge was not issued for {operation}"));
        }
        if key_role != role {
            return Err(format!("challenge key is not the account's {role} key"));
        }
        if challenge.used_at.is_some() {
            return Err("reused challenge".to_string());
        }
        if challenge.expires_at <= now_unix() {
            return Err("expired challenge".to_string());
        }
        let message = message(&challenge.id, &challenge.nonce);
        crypto::verify(&key.public_key, &message, signature)?;
        tx.execute(
            "UPDATE auth_challenges SET used_at = ?1 WHERE id = ?2 AND used_at IS NULL",
            params![now_unix(), challenge_id],
        )
        .map_err(|err| format!("failed to mark challenge used: {err}"))?;
        tx.commit()
            .map_err(|err| format!("failed to commit login: {err}"))?;
        Ok((owner, key))
    }

    pub fn insert_session(&self, session: &NewSession) -> Result<String, String> {
        let id = Uuid::new_v4().to_string();
        let conn = self.conn();
        conn.execute(
            "INSERT INTO sessions (id, owner_id, key_id, jwt_id, issued_at, expires_at, revoked_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL)",
            params![
                id,
                session.owner_id,
                session.key_id,
                session.jwt_id,
                session.issued_at,
                session.expires_at
            ],
        )
        .map_err(|err| format!("failed to store session: {err}"))?;
        Ok(id)
    }

    pub fn session_exists(&self, jwt_id: &str) -> Result<bool, String> {
        let conn = self.conn();
        let exists: Option<i64> = conn
            .query_row(
                "SELECT 1 FROM sessions WHERE jwt_id = ?1 AND revoked_at IS NULL",
                params![jwt_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|err| format!("failed to load session: {err}"))?;
        Ok(exists.is_some())
    }

    pub fn server_secret(&self) -> Result<Vec<u8>, String> {
        let conn = self.conn();
        conn.query_row(
            "SELECT secret FROM server_secrets WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .map_err(|err| format!("failed to load server signing secret: {err}"))
    }

    pub fn count_owners(&self) -> Result<i64, String> {
        let conn = self.conn();
        conn.query_row("SELECT COUNT(*) FROM owners", [], |row| row.get(0))
            .map_err(|err| format!("failed to count owners: {err}"))
    }

    /// The published versions of a package, oldest first (plan-10-A `/index`).
    /// Each row carries the version, content hash, publish time, and current
    /// release state; the transparency-log entry is resolved separately.
    pub fn list_package_versions(&self, ident: &str) -> Result<Vec<PackageVersionRow>, String> {
        let conn = self.conn();
        let mut statement = conn
            .prepare(
                "SELECT pv.version, pv.hash, pv.created_at, pv.state, pv.abi_index,
                        publisher.owner_display, publisher_key.fingerprint, pv.transfer_epoch
                 FROM package_versions pv
                 JOIN packages p ON p.id = pv.package_id
                 LEFT JOIN owners publisher ON publisher.id = pv.publisher_owner_id
                 LEFT JOIN keys publisher_key ON publisher_key.id = pv.publisher_ident_key_id
                 WHERE p.ident = ?1
                 ORDER BY pv.created_at ASC, pv.id ASC",
            )
            .map_err(|err| format!("failed to prepare version query: {err}"))?;
        let rows = statement
            .query_map(params![ident], |row| {
                Ok(PackageVersionRow {
                    version: row.get(0)?,
                    hash: row.get(1)?,
                    published_at: row.get(2)?,
                    state: row.get(3)?,
                    abi_index: row.get(4)?,
                    publisher_display: row.get(5)?,
                    publisher_ident_fingerprint: row.get(6)?,
                    transfer_epoch: row.get(7)?,
                })
            })
            .map_err(|err| format!("failed to list package versions: {err}"))?;
        let mut versions = Vec::new();
        for row in rows {
            versions.push(row.map_err(|err| format!("failed to read package version: {err}"))?);
        }
        Ok(versions)
    }

    /// The package's headline "latest" release: newest by `published_at` among
    /// the versions whose state is **active** (plan-126-A).
    ///
    /// Returns `(version, state)`, or `None` when the package has no active
    /// release at all — every published version yanked, blocked or
    /// legal-tombstoned — which is a statement to render, not missing data.
    ///
    /// This is the **only** server-side "latest version" selection. Before it
    /// existed, both the package page and the search results took
    /// `ORDER BY created_at DESC LIMIT 1` with no state predicate, so a package
    /// whose newest release was yanked advertised that release as its headline
    /// version — on the search page with no state badge at all.
    ///
    /// Two properties are load-bearing:
    ///
    /// * **The predicate is an allowlist**, `state IN ('available','deprecated')`,
    ///   matching [`crate::validation::state_is_active`] and through it the
    ///   install client's `state_is_floating_eligible`. Written as
    ///   `state != 'yanked'` it would admit `blocked` and `legal-tombstoned`.
    /// * **Ordering is by publish time**, not semver — `max_by_key(published_at)`
    ///   is how the install client selects, and diverging here would put the two
    ///   surfaces in disagreement about the same package.
    ///
    /// This filters the *selection* only. [`Self::package_detail`] still returns
    /// every version unfiltered; the transparency listing is never narrowed.
    pub fn latest_active_version(&self, ident: &str) -> Result<Option<(String, String)>, String> {
        let conn = self.conn();
        conn.query_row(
            // The two state literals are the SQL spelling of
            // `validation::state_is_active`. Keep them in step: a state added to
            // the active set there must be added here, and the
            // `active_states_are_an_allowlist_matching_the_install_client` test
            // plus this module's yanked/blocked/tombstoned tests are what catch
            // a one-sided edit.
            "SELECT pv.version, pv.state
             FROM package_versions pv
             JOIN packages p ON p.id = pv.package_id
             WHERE p.ident = ?1 AND pv.state IN ('available', 'deprecated')
             ORDER BY pv.created_at DESC, pv.id DESC
             LIMIT 1",
            params![ident],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|err| format!("failed to load latest active version: {err}"))
    }

    /// Record the raw MFPC section-17 bytes for one version (plan-126-E).
    ///
    /// **Raw bytes, not a decoded structure.** `mfb_wire::docs` is the single
    /// decoder; storing decoded rows would create a second representation that
    /// must migrate whenever the wire format is extended. A `BLOB` costs one
    /// decode per page render, on data already in the local database.
    ///
    /// `INSERT OR IGNORE`: a version's documentation is fixed by its signed
    /// payload, so an existing row is never rewritten. That is what keeps the
    /// backfill sweep idempotent -- a second pass over the same blob changes
    /// nothing.
    ///
    /// Returns whether a row was **actually inserted**: `false` when one already
    /// existed. The backfill sweep counts only real inserts, so a second run
    /// reports zero rather than re-claiming the rows the first run wrote.
    pub fn put_version_docs(
        &self,
        package_version_id: i64,
        section: &[u8],
    ) -> Result<bool, String> {
        let conn = self.conn();
        let inserted = conn
            .execute(
                "INSERT OR IGNORE INTO package_version_docs (package_version_id, doc_section)
                 VALUES (?1, ?2)",
                params![package_version_id, section],
            )
            .map_err(|err| format!("failed to record version documentation: {err}"))?;
        Ok(inserted == 1)
    }

    /// The doc section of a package's **latest active** release (plan-126-E), as
    /// `(version, raw section-17 bytes)`.
    ///
    /// Built on [`Self::latest_active_version`], deliberately -- not on its own
    /// `ORDER BY created_at LIMIT 1`. That is what makes a yanked newest release
    /// fall back to the older active one's documentation, and it keeps one
    /// definition of "latest" in the registry instead of two predicates that can
    /// drift (plan-126-A).
    ///
    /// `None` when the package has no active release, **or** when its latest
    /// active release carries no documentation. It never falls back to an older
    /// release's docs: the Docs tab's contract is "the current version's
    /// documentation", and showing an older version's would contradict it
    /// (plan-126-F Open Decision).
    ///
    /// `latest_active_version` is called before this function takes the
    /// connection lock. The lock is not re-entrant, so holding it across that
    /// call would deadlock.
    pub fn latest_active_version_docs(
        &self,
        ident: &str,
    ) -> Result<Option<(String, Vec<u8>)>, String> {
        let Some((version, _state)) = self.latest_active_version(ident)? else {
            return Ok(None);
        };
        let conn = self.conn();
        let section: Option<Vec<u8>> = conn
            .query_row(
                "SELECT d.doc_section
                 FROM package_version_docs d
                 JOIN package_versions pv ON pv.id = d.package_version_id
                 JOIN packages p ON p.id = pv.package_id
                 WHERE p.ident = ?1 AND pv.version = ?2",
                params![ident, version],
                |row| row.get(0),
            )
            .optional()
            .map_err(|err| format!("failed to load version documentation: {err}"))?;
        Ok(section.map(|section| (version, section)))
    }

    /// Everything `GET /packages/:ident` renders: the package's owner, its
    /// recorded metadata, and **every** version with its native target rows
    /// (plan-61-B §3).
    ///
    /// Returns `None` when no package carries this ident.
    ///
    /// No `state` filter, deliberately and permanently. A yanked or superseded
    /// version is still part of the published history, and a view that silently
    /// omits non-current versions reproduces exactly the truncation the open
    /// SUP-03 downgrade attack performs (`plan-61-repo-web.md` §4). `state` is a
    /// field for the caller to render, never a filter this query applies.
    pub fn package_detail(&self, ident: &str) -> Result<Option<PackageDetailRow>, String> {
        let conn = self.conn();
        let owner: Option<String> = conn
            .query_row(
                "SELECT o.owner_display FROM packages p
                 JOIN owners o ON o.id = p.owner_id
                 WHERE p.ident = ?1",
                params![ident],
                |row| row.get(0),
            )
            .optional()
            .map_err(|err| format!("failed to load package: {err}"))?;
        let Some(owner) = owner else {
            return Ok(None);
        };

        let mut statement = conn
            .prepare(
                "SELECT pv.id, pv.version, pv.hash, pv.created_at, pv.state,
                        pv.abi_index, pv.author, pv.url, pv.description
                 FROM package_versions pv
                 JOIN packages p ON p.id = pv.package_id
                 WHERE p.ident = ?1
                 ORDER BY pv.created_at DESC, pv.id DESC",
            )
            .map_err(|err| format!("failed to prepare detail query: {err}"))?;
        let rows = statement
            .query_map(params![ident], |row| {
                Ok(PackageDetailVersion {
                    id: row.get(0)?,
                    version: row.get(1)?,
                    hash: row.get(2)?,
                    published_at: row.get(3)?,
                    state: row.get(4)?,
                    abi_index: row.get(5)?,
                    author: row.get(6)?,
                    url: row.get(7)?,
                    description: row.get(8)?,
                    targets: Vec::new(),
                })
            })
            .map_err(|err| format!("failed to load package detail: {err}"))?;
        let mut versions = Vec::new();
        for row in rows {
            versions.push(row.map_err(|err| format!("failed to read package detail: {err}"))?);
        }

        let mut target_statement = conn
            .prepare(
                "SELECT os, arch, libc, lib_type, logical, source, blob_hash
                 FROM package_version_targets
                 WHERE package_version_id = ?1
                 ORDER BY rowid",
            )
            .map_err(|err| format!("failed to prepare target query: {err}"))?;
        for version in &mut versions {
            let rows = target_statement
                .query_map(params![version.id], |row| {
                    Ok(PackageTargetRow {
                        os: row.get(0)?,
                        arch: row.get(1)?,
                        libc: row.get(2)?,
                        lib_type: row.get(3)?,
                        logical: row.get(4)?,
                        source: row.get(5)?,
                        blob_hash: row.get(6)?,
                    })
                })
                .map_err(|err| format!("failed to load version targets: {err}"))?;
            for row in rows {
                version
                    .targets
                    .push(row.map_err(|err| format!("failed to read version target: {err}"))?);
            }
        }

        Ok(Some(PackageDetailRow { owner, versions }))
    }

    /// The transparency record for one package: its release-state history and
    /// its owner's ident-key rotations (plan-61-B §4). The publish log entries
    /// and inclusion proofs are assembled by the handler from the existing log
    /// API, which already knows how to build them.
    ///
    /// Returns `None` for an unknown ident, so the handler can 404 without a
    /// second existence query.
    pub fn package_audit(&self, ident: &str) -> Result<Option<PackageAuditRow>, String> {
        let conn = self.conn();
        let owner: Option<String> = conn
            .query_row(
                "SELECT o.owner_display FROM packages p
                 JOIN owners o ON o.id = p.owner_id
                 WHERE p.ident = ?1",
                params![ident],
                |row| row.get(0),
            )
            .optional()
            .map_err(|err| format!("failed to load package: {err}"))?;
        let Some(owner) = owner else {
            return Ok(None);
        };

        let mut statement = conn
            .prepare(
                "SELECT pv.version, rsc.state, rsc.created_at
                 FROM release_state_changes rsc
                 JOIN package_versions pv ON pv.id = rsc.package_version_id
                 JOIN packages p ON p.id = pv.package_id
                 WHERE p.ident = ?1
                 ORDER BY rsc.id ASC",
            )
            .map_err(|err| format!("failed to prepare state-change query: {err}"))?;
        let rows = statement
            .query_map(params![ident], |row| {
                Ok(ReleaseStateChangeRow {
                    version: row.get(0)?,
                    state: row.get(1)?,
                    at: row.get(2)?,
                })
            })
            .map_err(|err| format!("failed to load state changes: {err}"))?;
        let mut state_changes = Vec::new();
        for row in rows {
            state_changes.push(row.map_err(|err| format!("failed to read state change: {err}"))?);
        }

        // Query the versions on the connection already held. Calling
        // `list_package_versions` here would re-enter `self.conn()` while this
        // function still owns the guard, and the mutex is not reentrant — that
        // deadlocks the handler rather than failing it.
        let mut version_statement = conn
            .prepare(
                "SELECT pv.version FROM package_versions pv
                 JOIN packages p ON p.id = pv.package_id
                 WHERE p.ident = ?1
                 ORDER BY pv.created_at ASC, pv.id ASC",
            )
            .map_err(|err| format!("failed to prepare version query: {err}"))?;
        let rows = version_statement
            .query_map(params![ident], |row| row.get::<_, String>(0))
            .map_err(|err| format!("failed to list package versions: {err}"))?;
        let mut versions = Vec::new();
        for row in rows {
            versions.push(row.map_err(|err| format!("failed to read package version: {err}"))?);
        }
        drop(version_statement);
        drop(statement);
        drop(conn);

        Ok(Some(PackageAuditRow {
            owner,
            versions,
            state_changes,
        }))
    }

    /// Ranked package search for `GET /search` (plan-61-B §3).
    ///
    /// Ranking is exact ident, then ident prefix, then ident substring, then
    /// owner-name match, then an edit-distance-1 fuzzy tail. Ties break on
    /// ident so results are stable between requests.
    ///
    /// **The fuzzy tail runs only when the first four find nothing.** Phase 2's
    /// measurement task: `typosquat_candidates` is `SELECT ident FROM packages`
    /// — an unindexed full table scan with per-row Levenshtein in Rust
    /// (`store.rs:2438-2454`). It was written for one call per publish, not for
    /// a per-keystroke anonymous search route, so it is gated rather than
    /// wired into the common path.
    ///
    /// `query` is matched with `LIKE ... ESCAPE '\'` and the user's `%`, `_`
    /// and `\` are escaped, so a query of `%` matches a literal percent sign
    /// rather than every package.
    pub fn search_packages(
        &self,
        query: &str,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<SearchResultRow>, String> {
        let query = query.trim();
        // An empty query returns nothing rather than the whole table: this is
        // an anonymous route, and "no filter" must not mean "enumerate
        // everything".
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let folded = query.to_lowercase();
        let pattern = escape_like(&folded);
        let conn = self.conn();

        // Rank against the **package component** as well as the whole ident.
        // A user searching `sql` means the package named `sql`, not the literal
        // string `alice#sql` — ranking only on the full ident put every result
        // in the substring bucket and reduced the order to alphabetical.
        // `substr(ident, instr(ident, '#') + 1)` is the package half.
        let mut statement = conn
            .prepare(
                "SELECT p.ident, o.owner_display,
                        CASE
                            WHEN lower(p.ident) = ?1
                              OR lower(substr(p.ident, instr(p.ident, '#') + 1)) = ?1 THEN 0
                            WHEN lower(p.ident) LIKE ?2 ESCAPE '\\'
                              OR lower(substr(p.ident, instr(p.ident, '#') + 1))
                                 LIKE ?2 ESCAPE '\\' THEN 1
                            WHEN lower(p.ident) LIKE ?3 ESCAPE '\\' THEN 2
                            WHEN lower(o.owner_display) LIKE ?3 ESCAPE '\\' THEN 3
                            ELSE 4
                        END AS rank
                 FROM packages p
                 JOIN owners o ON o.id = p.owner_id
                 WHERE lower(p.ident) = ?1
                    OR lower(p.ident) LIKE ?3 ESCAPE '\\'
                    OR lower(o.owner_display) LIKE ?3 ESCAPE '\\'
                    -- plan-61-E: description text, ranked *below* ident and
                    -- owner. Scoped to the newest version so an old version's
                    -- description cannot keep a package findable by a word its
                    -- current release no longer contains.
                    OR EXISTS (
                        SELECT 1 FROM package_versions pv
                        WHERE pv.package_id = p.id
                          AND pv.id = (
                              SELECT id FROM package_versions
                              WHERE package_id = p.id
                              ORDER BY created_at DESC, id DESC LIMIT 1
                          )
                          AND lower(pv.description) LIKE ?3 ESCAPE '\\'
                    )
                 ORDER BY rank ASC, p.ident ASC
                 LIMIT ?4 OFFSET ?5",
            )
            .map_err(|err| format!("failed to prepare search query: {err}"))?;
        let rows = statement
            .query_map(
                params![
                    folded,
                    format!("{pattern}%"),
                    format!("%{pattern}%"),
                    limit,
                    offset
                ],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .map_err(|err| format!("failed to search packages: {err}"))?;
        let mut idents = Vec::new();
        for row in rows {
            idents.push(row.map_err(|err| format!("failed to read search result: {err}"))?);
        }
        drop(statement);

        // Fuzzy tail: only reached when nothing above matched, and only on the
        // first page — an offset past the end of an empty result set is still
        // empty.
        if idents.is_empty() && offset == 0 {
            let mut fuzzy = Vec::new();
            for (ident, owner) in fuzzy_search_candidates(&conn, folded.chars().count())? {
                if within_edit_distance_one(&ident.to_lowercase(), &folded) {
                    fuzzy.push((ident, owner));
                }
            }
            fuzzy.sort();
            fuzzy.truncate(limit.max(0) as usize);
            idents = fuzzy;
        }

        // Attach each package's newest **active** release. Done per package
        // rather than in the ranking query so the rank expression stays
        // readable; the page is already capped by `limit`.
        //
        // plan-126-A: the state predicate is the same allowlist
        // `Store::latest_active_version` applies, and `pv.state` is selected
        // alongside so the result chip can be badged. Before this, the search
        // page showed a yanked release as a package's current version with no
        // state marker of any kind — `SearchResultRow` carried no state field,
        // so there was nothing a renderer could have marked it with. That made
        // this the one surface where the missing filter was not merely cosmetic.
        let mut latest_statement = conn
            .prepare(
                "SELECT pv.version, pv.created_at, pv.description, pv.state
                 FROM package_versions pv
                 JOIN packages p ON p.id = pv.package_id
                 WHERE p.ident = ?1 AND pv.state IN ('available', 'deprecated')
                 ORDER BY pv.created_at DESC, pv.id DESC
                 LIMIT 1",
            )
            .map_err(|err| format!("failed to prepare latest-version query: {err}"))?;
        let mut results = Vec::new();
        for (ident, owner) in idents {
            let latest = latest_statement
                .query_row(params![ident], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                })
                .optional()
                .map_err(|err| format!("failed to read latest version: {err}"))?;
            // A match with no active release stays a match: the package exists
            // and the query found it. Only its headline version is absent, and
            // the page states that rather than dropping the row — a search that
            // silently hid a fully-yanked package would be the same truncation
            // the Overview's complete version table exists to prevent.
            let (latest_version, published_at, description, latest_state) = match latest {
                Some((version, at, description, release_state)) => {
                    (Some(version), Some(at), description, Some(release_state))
                }
                None => (None, None, None, None),
            };
            results.push(SearchResultRow {
                ident,
                owner,
                latest_version,
                latest_state,
                published_at,
                description,
            });
        }
        Ok(results)
    }

    pub fn package_version_exists(&self, ident: &str, version: &str) -> Result<bool, String> {
        let conn = self.conn();
        let exists: Option<i64> = conn
            .query_row(
                "SELECT 1
                 FROM package_versions pv
                 JOIN packages p ON p.id = pv.package_id
                 WHERE p.ident = ?1 AND pv.version = ?2",
                params![ident, version],
                |row| row.get(0),
            )
            .optional()
            .map_err(|err| format!("failed to check package version: {err}"))?;
        Ok(exists.is_some())
    }

    /// Total `package_versions` rows across every package this owner owns — the
    /// quantity bounded by the per-owner publish quota (bug-188 / REPO-13).
    pub fn owner_version_count(&self, owner_id: i64) -> Result<i64, String> {
        let conn = self.conn();
        conn.query_row(
            "SELECT COUNT(*) FROM package_versions v
             JOIN packages p ON v.package_id = p.id
             WHERE p.owner_id = ?1",
            params![owner_id],
            |row| row.get(0),
        )
        .map_err(|err| format!("failed to count owner versions: {err}"))
    }

    pub fn publish_package_version(
        &self,
        owner_id: i64,
        ident: &str,
        version: &str,
        hash: &str,
        blob_path: &str,
        abi_index: &str,
        vendor_blobs: &[VendorBlobRef],
        metadata: &PublishMetadata,
    ) -> Result<PublishedVersion, String> {
        // REPO-17: validate the ident's package component and the version against
        // an explicit safe charset/length before either reaches the log payload,
        // the `/index` route, or the REPO-14 log-lookup pattern. The owner half is
        // also re-checked here (validate_ident), the authoritative publish choke
        // point that every publish path funnels through.
        validate_ident(ident)?;
        validate_version(version)?;
        let now = now_unix();
        let mut conn = self.conn();
        let tx = conn
            .transaction()
            .map_err(|err| format!("failed to start publish transaction: {err}"))?;
        tx.execute(
            "INSERT OR IGNORE INTO packages (ident, owner_id, created_at)
             VALUES (?1, ?2, ?3)",
            params![ident, owner_id, now],
        )
        .map_err(|err| format!("failed to create package identity: {err}"))?;
        let package_id: i64 = tx
            .query_row(
                "SELECT id FROM packages WHERE ident = ?1 AND owner_id = ?2",
                params![ident, owner_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|err| format!("failed to load package identity: {err}"))?
            .ok_or_else(|| "package identity is owned by another owner".to_string())?;
        tx.execute(
            "INSERT OR IGNORE INTO package_blobs (hash, path, kind, created_at)
             VALUES (?1, ?2, 'package', ?3)",
            params![hash, blob_path, now],
        )
        .map_err(|err| format!("failed to store package blob metadata: {err}"))?;
        // plan-168-A §4.3: record who published the release, under which of
        // their ident keys, and in which ownership epoch. The key is looked up
        // among the publisher's ident keys of any status: a rotation between
        // `/validate` and this write retires the key but does not unmake the
        // signature the header carries.
        let publisher_ident_key_id: Option<i64> = match &metadata.publisher_ident_fingerprint {
            Some(fingerprint) => Some(
                tx.query_row(
                    "SELECT id FROM keys WHERE owner_id = ?1 AND role = 'ident' AND fingerprint = ?2",
                    params![owner_id, fingerprint],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|err| format!("failed to load publisher ident key: {err}"))?
                .ok_or_else(|| {
                    format!("publisher ident key {fingerprint} is not one of the publisher's keys")
                })?,
            ),
            None => None,
        };
        let transfer_epoch: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM transfer_history WHERE package_id = ?1",
                params![package_id],
                |row| row.get(0),
            )
            .map_err(|err| format!("failed to count transfer history: {err}"))?;
        tx.execute(
            "INSERT INTO package_versions
                 (package_id, version, hash, state, abi_index, author, url, description,
                  created_at, publisher_owner_id, publisher_ident_key_id, transfer_epoch)
             VALUES (?1, ?2, ?3, 'available', ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                package_id,
                version,
                hash,
                abi_index,
                metadata.author,
                metadata.url,
                metadata.description,
                now,
                owner_id,
                publisher_ident_key_id,
                transfer_epoch
            ],
        )
        .map_err(|err| {
            if is_unique_violation(&err) {
                format!("package version {ident}@{version} is already published")
            } else {
                format!("failed to publish package version: {err}")
            }
        })?;
        // plan-48-A §4.5: record the version→native-blob edges so a future GC
        // (plan-49) has the reachability data. The vendor blobs were already
        // uploaded via PUT /blob and their `package_blobs` rows exist; nothing
        // reads these edges in this plan.
        let package_version_id = tx.last_insert_rowid();
        // plan-126-E: the doc row is written in the SAME transaction as the version
        // row. A crash between two separate writes would leave a version whose docs
        // never appear -- and no backfill run could tell that apart from a package
        // that was simply never documented.
        if let Some(section) = &metadata.docs {
            tx.execute(
                "INSERT OR IGNORE INTO package_version_docs (package_version_id, doc_section)
                 VALUES (?1, ?2)",
                params![package_version_id, section],
            )
            .map_err(|err| format!("failed to record version documentation: {err}"))?;
        }
        for vendor in vendor_blobs {
            tx.execute(
                "INSERT OR IGNORE INTO package_version_blobs (package_version_id, hash)
                 VALUES (?1, ?2)",
                params![package_version_id, vendor.hash],
            )
            .map_err(|err| format!("failed to record version blob edge: {err}"))?;
        }
        // plan-61-A §3: the native target matrix, written **per locator**. The
        // `package_version_blobs` loop above collapses duplicate hashes on
        // purpose — a blob edge is about reachability, and probing one blob
        // twice is waste. Targets are the opposite: two locators sharing a hash
        // (two platforms shipping byte-identical builds under different
        // `source` names) are two supported platforms, and deduping them here
        // would silently under-report what the package runs on.
        insert_version_targets(&tx, package_version_id, vendor_blobs)?;
        let log_entry = append_log_tx(
            &tx,
            "publish",
            &format!(
                "{{\"ident\":{},\"version\":{},\"hash\":{}}}",
                json_value(ident),
                json_value(version),
                json_value(hash),
            ),
        )?;
        tx.commit()
            .map_err(|err| format!("failed to commit publish: {err}"))?;
        Ok(PublishedVersion {
            ident: ident.to_string(),
            version: version.to_string(),
            hash: hash.to_string(),
            published_at: now,
            state: "available".to_string(),
            log_entry,
        })
    }

    /// The stored `kind` of a blob (`"package"` or `"native"`), or `None` when
    /// no blob row exists for `hash` (plan-48-A §4.1). Lets `GET /blob/<hash>`
    /// 404 an unknown hash from SQLite and select the right backend name/suffix
    /// without an S3 round trip.
    pub fn blob_kind(&self, hash: &str) -> Result<Option<String>, String> {
        let conn = self.conn();
        conn.query_row(
            "SELECT kind FROM package_blobs WHERE hash = ?1",
            params![hash],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|err| format!("failed to load blob kind: {err}"))
    }

    /// Record a freshly uploaded native-library blob's metadata row
    /// (plan-48-A §4.3). Idempotent: re-uploading an existing blob leaves the
    /// original row untouched.
    /// Returns whether the caller should go on to promote a `<hash>.bin`.
    ///
    /// `package_blobs` is keyed by hash alone and `GET /blob/<hash>` picks the
    /// backend file from that row's `kind`. So when a package blob already exists
    /// with these exact bytes, the `INSERT OR IGNORE` is ignored and the row keeps
    /// `kind='package'` — at which point writing a `.bin` too produced a second
    /// copy that nothing ever reads or collects (bug-276 R5).
    ///
    /// Reporting `false` there is safe precisely because the blob is
    /// content-addressed: the existing object holds byte-identical content and
    /// `GET` already serves it under the recorded kind, so the upload is a
    /// genuine no-op rather than a dropped write.
    pub fn record_native_blob(&self, hash: &str, blob_path: &str) -> Result<bool, String> {
        let now = now_unix();
        let conn = self.conn();
        let existing: Option<String> = conn
            .query_row(
                "SELECT kind FROM package_blobs WHERE hash = ?1",
                params![hash],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|err| format!("failed to load native blob metadata: {err}"))?;
        if let Some(kind) = existing {
            return Ok(kind == "native");
        }
        conn.execute(
            "INSERT INTO package_blobs (hash, path, kind, created_at)
             VALUES (?1, ?2, 'native', ?3)",
            params![hash, blob_path, now],
        )
        .map_err(|err| format!("failed to store native blob metadata: {err}"))?;
        Ok(true)
    }

    // --- Blob garbage collection (plan-49) ---------------------------------

    /// Every blob row **no live package version references**, older than
    /// `grace_seconds` at `now` (plan-49 §4.1/§4.2).
    ///
    /// The reachable set is `package_versions.hash` **∪**
    /// `package_version_blobs.hash`, and *both* halves are load-bearing:
    /// `package_versions.hash` is the `.mfp` artifact itself, which is not a
    /// vendor blob and therefore never appears in `package_version_blobs`.
    /// Dropping the first half would report every published package as garbage.
    ///
    /// Reachability is recomputed from these two tables on every call rather
    /// than tracked in a refcount (plan-49 §3.4): a refcount drifts permanently
    /// on any crash between its two writes, and a drifted one either leaks
    /// forever or deletes a live blob. This is self-correcting instead.
    ///
    /// A **yanked** version is still a live row and so still reachable
    /// (plan-49 §3.2): yanking says "do not resolve this by default", not
    /// "delete it", and lockfiles pinning the hash must keep installing. Only a
    /// version row that no longer exists releases its blobs.
    ///
    /// `now` is a parameter rather than read from the clock so a caller — and a
    /// test — can ask what the candidate set looks like at a given instant.
    pub fn unreachable_blobs(&self, now: i64, grace_seconds: i64) -> Result<Vec<BlobRow>, String> {
        if grace_seconds < 0 {
            return Err("grace period must not be negative".to_string());
        }
        // A grace period large enough to underflow would wrap into the future
        // and make every blob a candidate — refuse it rather than sweep the
        // whole registry.
        let cutoff = now
            .checked_sub(grace_seconds)
            .ok_or_else(|| format!("grace period of {grace_seconds}s overflows the clock"))?;
        let conn = self.conn();
        let mut statement = conn
            .prepare(
                "SELECT hash, path, kind, created_at FROM package_blobs
                 WHERE hash NOT IN (SELECT hash FROM package_versions
                                    UNION
                                    SELECT hash FROM package_version_blobs)
                   AND created_at < ?1
                 ORDER BY created_at, hash",
            )
            .map_err(|err| format!("failed to prepare blob reachability query: {err}"))?;
        let rows = statement
            .query_map(params![cutoff], BlobRow::from_row)
            .map_err(|err| format!("failed to scan blobs: {err}"))?;
        let mut blobs = Vec::new();
        for row in rows {
            blobs.push(row.map_err(|err| format!("failed to read blob row: {err}"))?);
        }
        Ok(blobs)
    }

    /// Every blob row a live package version *does* reference — the complement
    /// of [`Store::unreachable_blobs`] with no grace-period filter, so a `gc`
    /// report can state what fraction of the store is garbage.
    pub fn reachable_blobs(&self) -> Result<Vec<BlobRow>, String> {
        let conn = self.conn();
        let mut statement = conn
            .prepare(
                "SELECT hash, path, kind, created_at FROM package_blobs
                 WHERE hash IN (SELECT hash FROM package_versions
                                UNION
                                SELECT hash FROM package_version_blobs)
                 ORDER BY created_at, hash",
            )
            .map_err(|err| format!("failed to prepare reachable blob query: {err}"))?;
        let rows = statement
            .query_map([], BlobRow::from_row)
            .map_err(|err| format!("failed to scan reachable blobs: {err}"))?;
        let mut blobs = Vec::new();
        for row in rows {
            blobs.push(row.map_err(|err| format!("failed to read blob row: {err}"))?);
        }
        Ok(blobs)
    }

    /// Whether one hash is referenced by a live version right now.
    ///
    /// The sweep re-asks this immediately before deleting each object, rather
    /// than trusting the scan it started with. The grace period makes the scan
    /// safe against an *in-flight* publish (§3.1), but a publisher who uploads a
    /// blob and publishes more than a grace period later is not in flight — they
    /// are slow, and their blob is a legitimate candidate right up until the
    /// publish commits. Re-checking narrows that window from the length of the
    /// whole sweep to the gap between this query and the delete.
    ///
    /// It does not *close* the window; nothing short of a lock would, and the
    /// design deliberately has none. `forget_blob`'s foreign key is the backstop
    /// for the remainder.
    pub fn blob_is_reachable(&self, hash: &str) -> Result<bool, String> {
        let conn = self.conn();
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM package_versions WHERE hash = ?1
                           UNION
                           SELECT 1 FROM package_version_blobs WHERE hash = ?1)",
            params![hash],
            |row| row.get::<_, i64>(0),
        )
        .map(|exists| exists != 0)
        .map_err(|err| format!("failed to re-check blob reachability: {err}"))
    }

    /// Drop a `package_blobs` row after its backing object has been deleted
    /// (plan-49 §4.4). Reports whether a row was actually removed, so a second
    /// `gc --delete` over the same hash is a visible no-op rather than a
    /// silent one.
    ///
    /// `package_version_blobs.hash` is a foreign key onto this row and
    /// `foreign_keys` is `ON`, so if a publish landed an edge between the scan
    /// and this delete, SQLite refuses it and the error surfaces instead of
    /// orphaning a live version's vendor blob.
    pub fn forget_blob(&self, hash: &str) -> Result<bool, String> {
        let conn = self.conn();
        let removed = conn
            .execute("DELETE FROM package_blobs WHERE hash = ?1", params![hash])
            .map_err(|err| format!("failed to delete blob metadata for {hash}: {err}"))?;
        Ok(removed > 0)
    }

    /// Resolve an owner name to its row (any account: user or org).
    fn owner_record(&self, owner: &str) -> Result<Option<OwnerRecord>, String> {
        let folded = fold_owner(owner);
        let conn = self.conn();
        conn.query_row(
            "SELECT id, owner_display FROM owners WHERE owner_folded = ?1 AND status = 'active'",
            params![folded],
            |row| {
                Ok(OwnerRecord {
                    id: row.get(0)?,
                    owner_display: row.get(1)?,
                })
            },
        )
        .optional()
        .map_err(|err| format!("failed to load owner: {err}"))
    }

    // --- Orgs (plan-10-D1) -------------------------------------------------

    /// A member's role in an org (`owner`/`admin`/`publisher`), or None.
    pub fn org_member_role(&self, org: &str, member: &str) -> Result<Option<String>, String> {
        let org_folded = fold_owner(org);
        let member_folded = fold_owner(member);
        let conn = self.conn();
        conn.query_row(
            "SELECT m.role
             FROM org_members m
             JOIN owners o ON o.id = m.org_id
             JOIN owners u ON u.id = m.member_id
             WHERE o.owner_folded = ?1 AND u.owner_folded = ?2",
            params![org_folded, member_folded],
            |row| row.get(0),
        )
        .optional()
        .map_err(|err| format!("failed to load org member role: {err}"))
    }

    /// Grant (or update) a member's org role and log it (plan-10-D1). Caller
    /// verifies the granting member's authority before this runs.
    pub fn grant_org_member(&self, org: &str, member: &str, role: &str) -> Result<(), String> {
        if !matches!(role, "owner" | "admin" | "publisher") {
            return Err("role must be owner, admin, or publisher".to_string());
        }
        let Some(org_record) = self.owner_record(org)? else {
            return Err("unknown org".to_string());
        };
        let Some(member_record) = self.owner_record(member)? else {
            return Err("unknown member account".to_string());
        };
        let now = now_unix();
        let mut conn = self.conn();
        let tx = conn
            .transaction()
            .map_err(|err| format!("failed to start org transaction: {err}"))?;
        tx.execute(
            "INSERT INTO org_members (org_id, member_id, role, created_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(org_id, member_id) DO UPDATE SET role = excluded.role",
            params![org_record.id, member_record.id, role, now],
        )
        .map_err(|err| format!("failed to record org member: {err}"))?;
        append_log_tx(
            &tx,
            "org-role",
            &format!(
                "{{\"org\":{},\"member\":{},\"role\":{}}}",
                json_value(&org_record.owner_display),
                json_value(&member_record.owner_display),
                json_value(role),
            ),
        )?;
        tx.commit()
            .map_err(|err| format!("failed to commit org member grant: {err}"))?;
        Ok(())
    }

    /// Remove a member from an org and log it (plan-10-D1). Returns false when
    /// the member had no role.
    pub fn remove_org_member(&self, org: &str, member: &str) -> Result<bool, String> {
        let Some(org_record) = self.owner_record(org)? else {
            return Err("unknown org".to_string());
        };
        let Some(member_record) = self.owner_record(member)? else {
            return Err("unknown member account".to_string());
        };
        let mut conn = self.conn();
        let tx = conn
            .transaction()
            .map_err(|err| format!("failed to start org transaction: {err}"))?;
        let removed = tx
            .execute(
                "DELETE FROM org_members WHERE org_id = ?1 AND member_id = ?2",
                params![org_record.id, member_record.id],
            )
            .map_err(|err| format!("failed to remove org member: {err}"))?;
        if removed == 0 {
            tx.commit().ok();
            return Ok(false);
        }
        append_log_tx(
            &tx,
            "org-role",
            &format!(
                "{{\"org\":{},\"member\":{},\"role\":{}}}",
                json_value(&org_record.owner_display),
                json_value(&member_record.owner_display),
                json_value("removed"),
            ),
        )?;
        tx.commit()
            .map_err(|err| format!("failed to commit org member removal: {err}"))?;
        Ok(true)
    }

    /// The org's members as `(member_display, role)`, oldest first.
    pub fn list_org_members(&self, org: &str) -> Result<Vec<(String, String)>, String> {
        let org_folded = fold_owner(org);
        let conn = self.conn();
        let mut statement = conn
            .prepare(
                "SELECT u.owner_display, m.role
                 FROM org_members m
                 JOIN owners o ON o.id = m.org_id
                 JOIN owners u ON u.id = m.member_id
                 WHERE o.owner_folded = ?1
                 ORDER BY m.id ASC",
            )
            .map_err(|err| format!("failed to prepare org query: {err}"))?;
        let rows = statement
            .query_map(params![org_folded], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|err| format!("failed to list org members: {err}"))?;
        let mut members = Vec::new();
        for row in rows {
            members.push(row.map_err(|err| format!("failed to read org member: {err}"))?);
        }
        Ok(members)
    }

    // --- Publish tokens (plan-10-D1) --------------------------------------

    /// Issue a scoped, TTL-bounded publish token: register its auth key on the
    /// owner and record the scope/expiry, logged. The token can open sessions
    /// and request attestations only within `scope` and only until `expires_at`
    /// — it never bypasses the ident-proof requirement. Caller verifies the
    /// owner-ident authorization before this runs.
    pub fn issue_publish_token(
        &self,
        owner: &str,
        token_public: &[u8],
        proof: &[u8],
        scope: &str,
        ttl_secs: i64,
    ) -> Result<(OwnerRecord, KeyRecord, i64), String> {
        let Some(owner_record) = self.owner_record(owner)? else {
            return Err("unknown owner".to_string());
        };
        let message = crypto::registration_message(
            crypto::ROLE_AUTH,
            &owner_record.owner_display,
            token_public,
        );
        crypto::verify(token_public, &message, proof)
            .map_err(|_| "invalid token proof-of-possession signature".to_string())?;
        if scope.is_empty() || scope.len() > 255 {
            return Err("token scope must be 1..=255 bytes".to_string());
        }
        if ttl_secs <= 0 || ttl_secs > 365 * 24 * 3600 {
            return Err("token ttl must be 1..=31536000 seconds".to_string());
        }
        let fingerprint = crypto::fingerprint(token_public);
        let now = now_unix();
        let expires_at = now + ttl_secs;
        let mut conn = self.conn();
        let tx = conn
            .transaction()
            .map_err(|err| format!("failed to start token transaction: {err}"))?;
        let key_id = insert_key_tx(
            &tx,
            owner_record.id,
            KEY_ROLE_AUTH,
            token_public,
            &fingerprint,
            now,
            "failed to register token key",
        )?;
        tx.execute(
            "INSERT INTO publish_tokens (owner_id, key_id, scope, expires_at, revoked_at, created_at)
             VALUES (?1, ?2, ?3, ?4, NULL, ?5)",
            params![owner_record.id, key_id, scope, expires_at, now],
        )
        .map_err(|err| format!("failed to record publish token: {err}"))?;
        append_log_tx(
            &tx,
            "token-issue",
            &format!(
                "{{\"owner\":{},\"tokenFingerprint\":{},\"scope\":{}}}",
                json_value(&owner_record.owner_display),
                json_value(&fingerprint),
                json_value(scope),
            ),
        )?;
        tx.commit()
            .map_err(|err| format!("failed to commit token issue: {err}"))?;
        Ok((
            owner_record,
            KeyRecord {
                id: key_id,
                public_key: token_public.to_vec(),
                fingerprint,
            },
            expires_at,
        ))
    }

    /// The publish-token scope/expiry/revocation for an auth key, if it is a
    /// token. Used at `/signing` to bound what a token session may attest.
    pub fn publish_token_for_key(
        &self,
        key_id: i64,
    ) -> Result<Option<(String, i64, Option<i64>)>, String> {
        let conn = self.conn();
        conn.query_row(
            "SELECT scope, expires_at, revoked_at FROM publish_tokens WHERE key_id = ?1",
            params![key_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(|err| format!("failed to load publish token: {err}"))
    }

    /// Revoke a publish token (its auth key and any sessions), logged. Returns
    /// false when no active token matches the fingerprint.
    pub fn revoke_publish_token(&self, owner: &str, fingerprint: &str) -> Result<bool, String> {
        let Some(owner_record) = self.owner_record(owner)? else {
            return Err("unknown owner".to_string());
        };
        let now = now_unix();
        let mut conn = self.conn();
        let tx = conn
            .transaction()
            .map_err(|err| format!("failed to start token revoke transaction: {err}"))?;
        let key_id: Option<i64> = tx
            .query_row(
                "SELECT k.id
                 FROM publish_tokens t
                 JOIN keys k ON k.id = t.key_id
                 WHERE t.owner_id = ?1 AND k.fingerprint = ?2 AND t.revoked_at IS NULL",
                params![owner_record.id, fingerprint],
                |row| row.get(0),
            )
            .optional()
            .map_err(|err| format!("failed to load token: {err}"))?;
        let Some(key_id) = key_id else {
            tx.commit().ok();
            return Ok(false);
        };
        tx.execute(
            "UPDATE publish_tokens SET revoked_at = ?1 WHERE key_id = ?2",
            params![now, key_id],
        )
        .map_err(|err| format!("failed to revoke token: {err}"))?;
        tx.execute(
            "UPDATE keys SET status = 'revoked', revoked_at = ?1 WHERE id = ?2",
            params![now, key_id],
        )
        .map_err(|err| format!("failed to revoke token key: {err}"))?;
        tx.execute(
            "UPDATE sessions SET revoked_at = ?1 WHERE key_id = ?2 AND revoked_at IS NULL",
            params![now, key_id],
        )
        .map_err(|err| format!("failed to close token sessions: {err}"))?;
        append_log_tx(
            &tx,
            "token-revoke",
            &format!(
                "{{\"owner\":{},\"tokenFingerprint\":{}}}",
                json_value(&owner_record.owner_display),
                json_value(fingerprint),
            ),
        )?;
        tx.commit()
            .map_err(|err| format!("failed to commit token revoke: {err}"))?;
        Ok(true)
    }

    // --- Ownership transfer (plan-10-D1) ----------------------------------

    /// The account that currently owns a package (may differ from the ident
    /// string's owner after a transfer).
    pub fn package_owner(&self, ident: &str) -> Result<Option<OwnerRecord>, String> {
        let conn = self.conn();
        conn.query_row(
            "SELECT o.id, o.owner_display
             FROM packages p JOIN owners o ON o.id = p.owner_id
             WHERE p.ident = ?1",
            params![ident],
            |row| {
                Ok(OwnerRecord {
                    id: row.get(0)?,
                    owner_display: row.get(1)?,
                })
            },
        )
        .optional()
        .map_err(|err| format!("failed to load package owner: {err}"))
    }

    /// The terms the next transfer of `ident` must sign (plan-168-A §4.3): the
    /// number of accepted transfers so far, the stored owner and their current
    /// ident key, and the cumulative release digest. `None` for an unknown
    /// package.
    pub fn transfer_terms(&self, ident: &str) -> Result<Option<TransferTerms>, String> {
        let conn = self.conn();
        Ok(transfer_terms_on(&conn, ident)?.map(|(_, terms)| terms))
    }

    /// Record a signed transfer offer: the current owner offers `ident` to
    /// `to_owner` on the terms `(sequence, release_digest)`, signed by the
    /// owner's current ident over `crypto::transfer_offer_message_v2`.
    ///
    /// The terms are recomputed and the signature verified inside the writing
    /// transaction, so an offer whose terms moved under it (a publish or an
    /// accept in between) is refused rather than recorded with a signature
    /// over terms that no longer hold.
    pub fn create_transfer_offer(
        &self,
        ident: &str,
        from_owner: &str,
        to_owner: &str,
        sequence: i64,
        release_digest: &str,
        offer_signature: &[u8],
    ) -> Result<(), String> {
        let Some(package_owner) = self.package_owner(ident)? else {
            return Err("unknown package".to_string());
        };
        if fold_owner(&package_owner.owner_display) != fold_owner(from_owner) {
            return Err("offering owner does not currently own the package".to_string());
        }
        let Some(to_record) = self.owner_record(to_owner)? else {
            return Err("unknown recipient account".to_string());
        };
        if to_record.id == package_owner.id {
            return Err("cannot transfer a package to its current owner".to_string());
        }
        let now = now_unix();
        let mut conn = self.conn();
        let tx = conn
            .transaction()
            .map_err(|err| format!("failed to start transfer transaction: {err}"))?;
        // Re-read the package inside the *writing* transaction and re-verify
        // ownership against it (bug-274). The checks above ran under their own
        // short-lived lock acquisitions, and axum handlers run concurrently over
        // one `Arc<Mutex<Connection>>`, so between them and this commit an accept
        // can land and move the package. The UPSERT below resets `accepted_at` to
        // NULL unconditionally, so without this re-check a still-in-flight offer
        // could overwrite an already-accepted row and re-list itself as pending
        // under the previous owner's now-stale authority.
        //
        // Returning here rolls the transaction back, which is the intended abort.
        let (package_id, current_owner_id): (i64, i64) = tx
            .query_row(
                "SELECT id, owner_id FROM packages WHERE ident = ?1",
                params![ident],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|err| format!("failed to load package: {err}"))?;
        if current_owner_id != package_owner.id {
            return Err("offering owner does not currently own the package".to_string());
        }
        let Some((_, terms)) = transfer_terms_on(&tx, ident)? else {
            return Err("unknown package".to_string());
        };
        if terms.sequence != sequence || terms.release_digest != release_digest {
            return Err(STALE_TRANSFER_TERMS.to_string());
        }
        let message = crypto::transfer_offer_message_v2(crypto::TransferTermsRef {
            ident,
            sequence,
            from_owner: &terms.current_owner.owner_display,
            from_ident_fingerprint: &terms.current_ident.fingerprint,
            to_owner: &to_record.owner_display,
            release_digest,
        });
        crypto::verify(&terms.current_ident.public_key, &message, offer_signature)
            .map_err(|_| "invalid transfer offer ident signature".to_string())?;
        tx.execute(
            "INSERT INTO transfer_offers (package_id, from_owner_id, to_owner_id, created_at,
                 accepted_at, sequence, from_ident_key_id, release_digest, offer_signature)
             VALUES (?1, ?2, ?3, ?4, NULL, ?5, ?6, ?7, ?8)
             ON CONFLICT(package_id) DO UPDATE SET
               from_owner_id = excluded.from_owner_id,
               to_owner_id = excluded.to_owner_id,
               created_at = excluded.created_at,
               accepted_at = NULL,
               sequence = excluded.sequence,
               from_ident_key_id = excluded.from_ident_key_id,
               release_digest = excluded.release_digest,
               offer_signature = excluded.offer_signature",
            params![
                package_id,
                package_owner.id,
                to_record.id,
                now,
                sequence,
                terms.current_ident.id,
                release_digest,
                offer_signature
            ],
        )
        .map_err(|err| format!("failed to record transfer offer: {err}"))?;
        append_log_tx(
            &tx,
            "transfer-offer",
            &format!(
                "{{\"ident\":{},\"from\":{},\"to\":{}}}",
                json_value(ident),
                json_value(&package_owner.owner_display),
                json_value(&to_record.owner_display),
            ),
        )?;
        tx.commit()
            .map_err(|err| format!("failed to commit transfer offer: {err}"))?;
        Ok(())
    }

    /// Accept a pending signed transfer: verify the recipient's signature over
    /// `crypto::transfer_accept_message_v2` on the package's current terms,
    /// require those terms to be exactly the pending offer's, then append the
    /// `transfer_history` link, re-bind the package to `to_owner`, and log it.
    /// Already-published versions keep verifying against the old ident's
    /// proofs (issued facts); new versions publish under the new owner's ident.
    pub fn accept_transfer(
        &self,
        ident: &str,
        to_owner: &str,
        accept_signature: &[u8],
    ) -> Result<(), String> {
        let Some(to_record) = self.owner_record(to_owner)? else {
            return Err("unknown recipient account".to_string());
        };
        let now = now_unix();
        let mut conn = self.conn();
        let tx = conn
            .transaction()
            .map_err(|err| format!("failed to start transfer transaction: {err}"))?;
        let Some((package_id, terms)) = transfer_terms_on(&tx, ident)? else {
            return Err(NO_PENDING_OFFER.to_string());
        };
        let Some(to_ident) = current_ident_key_on(&tx, to_record.id)? else {
            return Err("recipient has no current ident key".to_string());
        };
        let message = crypto::transfer_accept_message_v2(crypto::TransferTermsRef {
            ident,
            sequence: terms.sequence,
            from_owner: &terms.current_owner.owner_display,
            from_ident_fingerprint: &terms.current_ident.fingerprint,
            to_owner: &to_record.owner_display,
            release_digest: &terms.release_digest,
        });
        crypto::verify(&to_ident.public_key, &message, accept_signature)
            .map_err(|_| "invalid transfer accept ident signature".to_string())?;
        let offer: Option<PendingOfferRow> = tx
            .query_row(
                // `o.from_owner_id = p.owner_id` is the stale-offer guard
                // (bug-274): an offer is only acceptable while the account that
                // made it is still the package's owner. Without it, an offer
                // resurrected after ownership moved — or simply left pending
                // across a transfer — could re-bind the package on the authority
                // of an account that no longer holds it.
                "SELECT o.id, o.sequence, o.from_ident_key_id, o.release_digest,
                        o.offer_signature, o.created_at
                 FROM transfer_offers o
                 JOIN packages p ON p.id = o.package_id
                 WHERE p.ident = ?1 AND o.to_owner_id = ?2 AND o.accepted_at IS NULL
                   AND o.from_owner_id = p.owner_id",
                params![ident, to_record.id],
                |row| {
                    Ok(PendingOfferRow {
                        id: row.get(0)?,
                        sequence: row.get(1)?,
                        from_ident_key_id: row.get(2)?,
                        release_digest: row.get(3)?,
                        offer_signature: row.get(4)?,
                        offered_at: row.get(5)?,
                    })
                },
            )
            .optional()
            .map_err(|err| format!("failed to load transfer offer: {err}"))?;
        let Some(offer) = offer else {
            return Err(NO_PENDING_OFFER.to_string());
        };
        let (Some(sequence), Some(from_key_id), Some(digest), Some(offer_signature)) = (
            offer.sequence,
            offer.from_ident_key_id,
            offer.release_digest,
            offer.offer_signature,
        ) else {
            return Err(
                "this offer predates signed transfers; the owner must re-offer".to_string(),
            );
        };
        // The recipient signed the CURRENT terms; the offer must have been made
        // on exactly those, under the offerer's still-current ident key, or the
        // recorded link would pair two signatures over different facts.
        if sequence != terms.sequence
            || digest != terms.release_digest
            || from_key_id != terms.current_ident.id
        {
            return Err(STALE_TRANSFER_TERMS.to_string());
        }
        tx.execute(
            "INSERT INTO transfer_history
                 (package_id, sequence, from_owner_id, from_ident_key_id, to_owner_id,
                  to_ident_key_id, release_digest, offer_signature, accept_signature,
                  offered_at, accepted_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                package_id,
                sequence,
                terms.current_owner.id,
                from_key_id,
                to_record.id,
                to_ident.id,
                digest,
                offer_signature,
                accept_signature,
                offer.offered_at,
                now
            ],
        )
        .map_err(|err| format!("failed to record transfer history: {err}"))?;
        tx.execute(
            "UPDATE packages SET owner_id = ?1 WHERE id = ?2",
            params![to_record.id, package_id],
        )
        .map_err(|err| format!("failed to re-bind package owner: {err}"))?;
        tx.execute(
            "UPDATE transfer_offers SET accepted_at = ?1 WHERE id = ?2",
            params![now, offer.id],
        )
        .map_err(|err| format!("failed to close transfer offer: {err}"))?;
        append_log_tx(
            &tx,
            "transfer-accept",
            &format!(
                "{{\"ident\":{},\"to\":{},\"sequence\":{}}}",
                json_value(ident),
                json_value(&to_record.owner_display),
                sequence,
            ),
        )?;
        tx.commit()
            .map_err(|err| format!("failed to commit transfer accept: {err}"))?;
        Ok(())
    }

    /// Whether the package moved through a transfer accepted before signed
    /// transfers existed (plan-168-B): an unsigned legacy history link, which
    /// no client can authenticate.
    pub fn has_legacy_transfer(&self, ident: &str) -> Result<bool, String> {
        let conn = self.conn();
        conn.query_row(
            "SELECT EXISTS (
                 SELECT 1 FROM transfer_history h JOIN packages p ON p.id = h.package_id
                 WHERE p.ident = ?1 AND h.offer_signature IS NULL)",
            params![ident],
            |row| row.get(0),
        )
        .map_err(|err| format!("failed to read transfer history: {err}"))
    }

    /// The package's accepted transfers, oldest first (plan-168-A §4.3). A
    /// legacy link (replayed from the transparency log) has no keys, digest or
    /// signatures.
    pub fn transfer_history(&self, ident: &str) -> Result<Vec<TransferRecord>, String> {
        let conn = self.conn();
        let mut statement = conn
            .prepare(
                "SELECT h.sequence, from_o.owner_display, from_k.public_key,
                        to_o.owner_display, to_k.public_key, h.release_digest,
                        h.offer_signature, h.accept_signature, h.offered_at, h.accepted_at
                 FROM transfer_history h
                 JOIN packages p ON p.id = h.package_id
                 JOIN owners from_o ON from_o.id = h.from_owner_id
                 JOIN owners to_o ON to_o.id = h.to_owner_id
                 LEFT JOIN keys from_k ON from_k.id = h.from_ident_key_id
                 LEFT JOIN keys to_k ON to_k.id = h.to_ident_key_id
                 WHERE p.ident = ?1
                 ORDER BY h.sequence ASC",
            )
            .map_err(|err| format!("failed to prepare transfer history query: {err}"))?;
        let rows = statement
            .query_map(params![ident], |row| {
                Ok(TransferRecord {
                    sequence: row.get(0)?,
                    from_owner: row.get(1)?,
                    from_ident_key: row.get(2)?,
                    to_owner: row.get(3)?,
                    to_ident_key: row.get(4)?,
                    release_digest: row.get(5)?,
                    offer_signature: row.get(6)?,
                    accept_signature: row.get(7)?,
                    offered_at: row.get(8)?,
                    accepted_at: row.get(9)?,
                })
            })
            .map_err(|err| format!("failed to load transfer history: {err}"))?;
        let mut history = Vec::new();
        for row in rows {
            history.push(row.map_err(|err| format!("failed to read transfer history: {err}"))?);
        }
        Ok(history)
    }

    /// Operator root ceremony (plan-10-C2): generate the offline root key and
    /// the online snapshot/timestamp keys, sign a `root.json` that delegates
    /// the server (attestation), snapshot, and timestamp keys, and persist
    /// everything **except the root private key**, which is returned for the
    /// operator to store offline. The root private key never touches the
    /// serving host's database.
    ///
    /// **One-time** (bug-584): this refuses to run against a configured
    /// registry. It used to mint a fresh offline root key and overwrite the
    /// sole config row on every run, which silently replaced the anchor a
    /// pinned client verifies against — a change indistinguishable, from the
    /// client's side, from registry takeover. Renewing the delegated online
    /// keys and the root expiry is `renew_registry_root`, authenticated by the
    /// offline root key and keeping the anchor; deliberately replacing a lost
    /// anchor is `reanchor_registry_root`.
    pub fn init_registry_root(
        &self,
        registry_id: &str,
        expires_at: i64,
    ) -> Result<Vec<u8>, String> {
        let (root_public, root_private) = crypto::generate_keypair();
        self.write_registry_root(
            registry_id,
            expires_at,
            &root_public,
            &root_private,
            RootCeremony::Init,
        )?;
        Ok(root_private)
    }

    /// Root renewal (bug-584): re-sign `root.json` with a bumped version, a
    /// fresh expiry, and freshly generated online snapshot/timestamp keys —
    /// under the **same** offline root key, which the operator supplies from
    /// offline storage and which is never persisted here.
    ///
    /// This is the authenticated old-to-new transition the pinned anchor
    /// needs: possession of the offline root key is the authentication, the
    /// anchor (`root_public`, hence its fingerprint) is unchanged, so a client
    /// that pinned the registry before the renewal verifies the renewed chain
    /// with no out-of-band step. A key that is not the configured root — and a
    /// registry id that is not the configured one — is refused without
    /// touching any stored field. Returns the new root version.
    pub fn renew_registry_root(
        &self,
        registry_id: &str,
        expires_at: i64,
        root_private: &[u8],
    ) -> Result<i64, String> {
        let root_public = crypto::public_from_private(root_private)
            .map_err(|_| "offline root key is malformed".to_string())?;
        self.write_registry_root(
            registry_id,
            expires_at,
            &root_public,
            root_private,
            RootCeremony::Renew,
        )
    }

    /// Deliberate root re-anchor (bug-584): mint a **new** offline root key and
    /// replace the anchor, for the one case renewal cannot cover — the offline
    /// root key is lost. Mirrors the ident `reanchor` ceremony (plan-23 §3.6):
    /// every client that pinned the old fingerprint fails hard until it
    /// re-pins the new one out of band, which is why it is a separate,
    /// explicitly selected, transparency-logged mode and never something a
    /// repeated `init-root` does by accident. Returns the new root private key
    /// for the operator to store offline.
    pub fn reanchor_registry_root(
        &self,
        registry_id: &str,
        expires_at: i64,
    ) -> Result<Vec<u8>, String> {
        let (root_public, root_private) = crypto::generate_keypair();
        self.write_registry_root(
            registry_id,
            expires_at,
            &root_public,
            &root_private,
            RootCeremony::Reanchor,
        )?;
        Ok(root_private)
    }

    /// The one writer of `registry_config`'s root columns. `ceremony` decides
    /// what it may do to an already-configured root — the whole security
    /// question of bug-584 — and every mode appends a transparency-log entry so
    /// a root change is auditable after the fact.
    fn write_registry_root(
        &self,
        registry_id: &str,
        expires_at: i64,
        root_public: &[u8],
        root_private: &[u8],
        ceremony: RootCeremony,
    ) -> Result<i64, String> {
        if registry_id.is_empty() || registry_id.len() > 255 {
            return Err("registry id must be 1..=255 bytes".to_string());
        }
        let (server_public, _server_private) = self.server_keypair()?;
        let (snapshot_public, snapshot_private) = crypto::generate_keypair();
        let (timestamp_public, timestamp_private) = crypto::generate_keypair();
        let now = now_unix();
        let mut conn = self.conn();
        let tx = conn
            .transaction()
            .map_err(|err| format!("failed to start root ceremony transaction: {err}"))?;
        let existing: Option<(i64, String, Vec<u8>)> = tx
            .query_row(
                "SELECT root_version, registry_id, root_public FROM registry_config WHERE id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(|err| format!("failed to read registry config: {err}"))?;
        let previous_version = match (ceremony, &existing) {
            (RootCeremony::Init, Some(_)) => {
                return Err(
                    "registry root of trust is already initialized; renew it with `mfb-repo \
                     renew-root` (same offline root key, pinned clients keep verifying) or, only \
                     if the offline root key is lost, re-anchor it with `mfb-repo reanchor-root` \
                     (every pinned client must re-pin out of band)"
                        .to_string(),
                )
            }
            (RootCeremony::Init, None) => 0,
            (RootCeremony::Renew | RootCeremony::Reanchor, None) => {
                return Err(
                    "registry root of trust is not initialized; run `mfb-repo init-root` first"
                        .to_string(),
                )
            }
            (RootCeremony::Renew, Some((version, existing_id, existing_public))) => {
                if existing_public.as_slice() != root_public {
                    return Err(
                        "the supplied key is not this registry's offline root key; refusing to \
                         replace the pinned anchor"
                            .to_string(),
                    );
                }
                if existing_id != registry_id {
                    return Err(format!(
                        "registry id `{registry_id}` does not match the configured registry id \
                         `{existing_id}`"
                    ));
                }
                *version
            }
            (RootCeremony::Reanchor, Some((version, _, _))) => *version,
        };
        let version = previous_version + 1;
        let root_json = format!(
            "{{\"type\":\"root\",\"registryId\":{},\"version\":{},\"expires\":{},\"serverKey\":{},\"snapshotKey\":{},\"timestampKey\":{}}}",
            json_value(registry_id),
            version,
            expires_at,
            json_value(&crypto::encode_bytes(&server_public)),
            json_value(&crypto::encode_bytes(&snapshot_public)),
            json_value(&crypto::encode_bytes(&timestamp_public)),
        );
        let root_signature = crypto::sign(
            root_private,
            &crypto::root_signing_input(root_json.as_bytes()),
        )?;
        tx.execute(
            "INSERT INTO registry_config
               (id, registry_id, root_version, root_public, root_json, root_signature,
                snapshot_public, snapshot_private, timestamp_public, timestamp_private, created_at)
             VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(id) DO UPDATE SET
               registry_id = excluded.registry_id,
               root_version = excluded.root_version,
               root_public = excluded.root_public,
               root_json = excluded.root_json,
               root_signature = excluded.root_signature,
               snapshot_public = excluded.snapshot_public,
               snapshot_private = excluded.snapshot_private,
               timestamp_public = excluded.timestamp_public,
               timestamp_private = excluded.timestamp_private",
            params![
                registry_id,
                version,
                root_public,
                root_json,
                root_signature,
                snapshot_public,
                snapshot_private,
                timestamp_public,
                timestamp_private,
                now,
            ],
        )
        .map_err(|err| format!("failed to store registry root: {err}"))?;
        append_log_tx(
            &tx,
            ceremony.log_kind(),
            &format!(
                "{{\"registryId\":{},\"version\":{},\"rootFingerprint\":{}}}",
                json_value(registry_id),
                version,
                json_value(&crypto::fingerprint(root_public)),
            ),
        )?;
        tx.commit()
            .map_err(|err| format!("failed to commit root ceremony: {err}"))?;
        Ok(version)
    }

    /// The signed `root.json` and its delegated online keypairs, if the root
    /// ceremony has been run.
    pub fn registry_config(&self) -> Result<Option<RegistryConfig>, String> {
        let conn = self.conn();
        conn.query_row(
            "SELECT registry_id, root_public, root_json, root_signature,
                    snapshot_public, snapshot_private, timestamp_public, timestamp_private
             FROM registry_config WHERE id = 1",
            [],
            |row| {
                Ok(RegistryConfig {
                    registry_id: row.get(0)?,
                    root_public: row.get(1)?,
                    root_json: row.get(2)?,
                    root_signature: row.get(3)?,
                    snapshot_public: row.get(4)?,
                    snapshot_private: row.get(5)?,
                    timestamp_public: row.get(6)?,
                    timestamp_private: row.get(7)?,
                })
            },
        )
        .optional()
        .map_err(|err| format!("failed to load registry config: {err}"))
    }

    /// A canonical hash of the whole served index — every `(ident, version,
    /// hash, state)` tuple, sorted — so a snapshot can attest to the exact
    /// index state and a mirror serving a stale or partial index is detected.
    pub fn index_canonical_hash(&self) -> Result<String, String> {
        let conn = self.conn();
        let mut statement = conn
            .prepare(
                "SELECT p.ident, pv.version, pv.hash, pv.state
                 FROM package_versions pv
                 JOIN packages p ON p.id = pv.package_id",
            )
            .map_err(|err| format!("failed to prepare index query: {err}"))?;
        let rows = statement
            .query_map([], |row| {
                let ident: String = row.get(0)?;
                let version: String = row.get(1)?;
                let hash: String = row.get(2)?;
                let state: String = row.get(3)?;
                Ok(format!("{ident}\u{0}{version}\u{0}{hash}\u{0}{state}\n"))
            })
            .map_err(|err| format!("failed to read index: {err}"))?;
        let mut lines = Vec::new();
        for row in rows {
            lines.push(row.map_err(|err| format!("failed to read index row: {err}"))?);
        }
        lines.sort();
        let mut bytes = Vec::new();
        for line in lines {
            bytes.extend_from_slice(line.as_bytes());
        }
        Ok(hex::encode(crypto::sha256(&bytes)))
    }

    /// Set a published version's release state (plan-10-C1). Updates the
    /// current state, records the transition with a timestamp, and appends one
    /// transparency-log entry — all in a single transaction. Ident-signature
    /// authorization is checked by the caller before this runs. Returns the
    /// publish/transition log entry reference.
    ///
    /// `owner_id` must be the package's **current** owner (`packages.owner_id`,
    /// which a transfer rewrites): the row is resolved under that predicate, so
    /// the store refuses a former owner even if a handler forgot to (bug-494),
    /// exactly as `publish_package_version` does.
    pub fn set_release_state(
        &self,
        owner_id: i64,
        ident: &str,
        version: &str,
        state: &str,
    ) -> Result<LogEntryRef, String> {
        let now = now_unix();
        let mut conn = self.conn();
        let tx = conn
            .transaction()
            .map_err(|err| format!("failed to start release-state transaction: {err}"))?;
        let package_version_id: Option<i64> = tx
            .query_row(
                "SELECT pv.id
                 FROM package_versions pv
                 JOIN packages p ON p.id = pv.package_id
                 WHERE p.ident = ?1 AND pv.version = ?2 AND p.owner_id = ?3",
                params![ident, version, owner_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|err| format!("failed to load package version: {err}"))?;
        let Some(package_version_id) = package_version_id else {
            return Err(format!(
                "package version {ident}@{version} is not published by this account"
            ));
        };
        tx.execute(
            "UPDATE package_versions SET state = ?1 WHERE id = ?2",
            params![state, package_version_id],
        )
        .map_err(|err| format!("failed to update release state: {err}"))?;
        tx.execute(
            "INSERT INTO release_state_changes (package_version_id, state, created_at)
             VALUES (?1, ?2, ?3)",
            params![package_version_id, state, now],
        )
        .map_err(|err| format!("failed to record release-state change: {err}"))?;
        let log_entry = append_log_tx(
            &tx,
            "release-state",
            &format!(
                "{{\"ident\":{},\"version\":{},\"state\":{}}}",
                json_value(ident),
                json_value(version),
                json_value(state),
            ),
        )?;
        tx.commit()
            .map_err(|err| format!("failed to commit release-state change: {err}"))?;
        Ok(log_entry)
    }

    /// Reap expired challenges, sessions, and pairing blobs (plan-10-D2). Runs
    /// on a timer so stale rows do not accumulate. Returns the number of rows
    /// deleted/closed across the three tables.
    pub fn reap_expired(&self) -> Result<usize, String> {
        let now = now_unix();
        let conn = self.conn();
        let mut total = 0usize;
        total += conn
            .execute(
                "DELETE FROM auth_challenges WHERE expires_at <= ?1",
                params![now],
            )
            .map_err(|err| format!("failed to reap challenges: {err}"))?;
        total += conn
            .execute(
                "UPDATE sessions SET revoked_at = ?1 WHERE revoked_at IS NULL AND expires_at <= ?1",
                params![now],
            )
            .map_err(|err| format!("failed to reap sessions: {err}"))?;
        total += conn
            .execute(
                "DELETE FROM pairing_blobs WHERE expires_at <= ?1",
                params![now],
            )
            .map_err(|err| format!("failed to reap pairing blobs: {err}"))?;
        Ok(total)
    }

    /// Existing package idents within edit distance 1 of `ident` (excluding an
    /// exact match) — the warn-only typosquat check at publish (plan-10-D2).
    pub fn typosquat_candidates(&self, ident: &str) -> Result<Vec<String>, String> {
        let conn = self.conn();
        let mut statement = conn
            .prepare("SELECT ident FROM packages")
            .map_err(|err| format!("failed to prepare typosquat query: {err}"))?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|err| format!("failed to scan packages: {err}"))?;
        let mut candidates = Vec::new();
        for row in rows {
            let existing = row.map_err(|err| format!("failed to read package ident: {err}"))?;
            if existing != ident && within_edit_distance_one(&existing, ident) {
                candidates.push(existing);
            }
        }
        Ok(candidates)
    }

    /// The number of transparency-log entries (the tree size).
    pub fn log_size(&self) -> Result<i64, String> {
        let conn = self.conn();
        conn.query_row("SELECT COUNT(*) FROM log_entries", [], |row| row.get(0))
            .map_err(|err| format!("failed to size the log: {err}"))
    }

    /// The current signed tree head: `(size, root, signature)`, memoised by
    /// log size (bug-579).
    ///
    /// `/log/checkpoint`, `/packages/:ident/audit` and `/snapshot.json` are all
    /// anonymous and each rebuilt this independently: every request loaded every
    /// leaf, recomputed the whole Merkle root and produced a fresh signature. The
    /// cost grew with the log and nothing bounded the repetition, so public
    /// traffic could pin the CPU and serialize unrelated work behind `Store`'s
    /// single connection mutex.
    ///
    /// The memo is deliberately NOT inside `log_leaf_hashes`. That reader has a
    /// documented contract about surfacing a malformed leaf as an error, and a
    /// test corrupts leaves in place — leaving the row COUNT unchanged — to prove
    /// it. A cache keyed on size inside that function would serve the pre-corruption
    /// leaves and silently defeat the check. Here the key is sound because
    /// production only ever appends.
    ///
    /// The signature is recomputed only when the size moves, so a cache hit is
    /// also one fewer Ed25519 signing operation.
    pub fn signed_checkpoint(&self) -> Result<(i64, [u8; 32], Vec<u8>), String> {
        let size = self.log_size()?;
        {
            let cached = self
                .head_cache
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(head) = cached.as_ref() {
                if head.size == size {
                    return Ok((head.size, head.root, head.signature.clone()));
                }
            }
        }
        let leaves = self.log_leaf_hashes(None)?;
        // Re-read rather than trusting the count taken above: an append between
        // the two reads would otherwise cache a root under the wrong size.
        let size = leaves.len() as i64;
        let root = crate::log::root(&leaves);
        let (_public, private) = self.server_keypair()?;
        let signature = crypto::sign(
            &private,
            &crate::log::checkpoint_signing_input(size as u64, &root),
        )?;
        let mut cached = self
            .head_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *cached = Some(CachedLogHead {
            size,
            root,
            signature: signature.clone(),
        });
        Ok((size, root, signature))
    }

    /// The ordered leaf hashes of the first `size` log entries (the whole
    /// log when `size` is None).
    pub fn log_leaf_hashes(&self, size: Option<i64>) -> Result<Vec<[u8; 32]>, String> {
        // A negative size silently selected zero leaves (`WHERE idx < -1`), and
        // `log_consistency_proof` would then compute `to = 0` and hand back a
        // structurally "valid" empty-range proof for a log that is not empty
        // (bug-276 R6). The callers take an unvalidated `Option<i64>` straight off
        // a query string, so a rewriting proxy could feed a client that view and
        // weaken an integrity signal it is trusting. Reject rather than clamp: a
        // negative size is never a meaningful request.
        if let Some(size) = size {
            if size < 0 {
                return Err("log size must not be negative".to_string());
            }
        }
        let conn = self.conn();
        let limit = size.unwrap_or(i64::MAX);
        let mut statement = conn
            .prepare("SELECT leaf_hash FROM log_entries WHERE idx < ?1 ORDER BY idx ASC")
            .map_err(|err| format!("failed to prepare log query: {err}"))?;
        let rows = statement
            .query_map(params![limit], |row| row.get::<_, Vec<u8>>(0))
            .map_err(|err| format!("failed to load log leaves: {err}"))?;
        let mut leaves = Vec::new();
        for row in rows {
            let raw = row.map_err(|err| format!("failed to read log leaf: {err}"))?;
            let mut leaf = [0u8; 32];
            if raw.len() != 32 {
                return Err("malformed log leaf hash".to_string());
            }
            leaf.copy_from_slice(&raw);
            leaves.push(leaf);
        }
        Ok(leaves)
    }

    /// Look up the publish log entry for `ident@version`.
    pub fn publish_log_entry(
        &self,
        ident: &str,
        version: &str,
    ) -> Result<Option<LogEntryRef>, String> {
        let payload_ident = json_value(ident);
        let payload_version = json_value(version);
        let conn = self.conn();
        // publish payloads are canonical (`{"ident":...,"version":...,"hash":...}`),
        // so a prefix match on the two identity fields is exact.
        let prefix = format!("{{\"ident\":{payload_ident},\"version\":{payload_version},");
        // REPO-14: `_` and `%` in the ident/version are SQL `LIKE` wildcards. Owner
        // names admit `_` and package/version are otherwise unconstrained, so an
        // un-escaped prefix could match a *different* package's log entry —
        // corrupting the inclusion-proof mapping a client verifies. Escape the
        // metacharacters (`\` first, then `%`/`_`) and match with an explicit
        // `ESCAPE`, appending the single intended trailing wildcard.
        let escaped = prefix
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        let pattern = format!("{escaped}%");
        conn.query_row(
            "SELECT idx, leaf_hash FROM log_entries
             WHERE kind = 'publish' AND payload LIKE ?1 ESCAPE '\\'
             ORDER BY idx ASC LIMIT 1",
            params![pattern],
            |row| {
                let index: i64 = row.get(0)?;
                let raw: Vec<u8> = row.get(1)?;
                Ok((index, raw))
            },
        )
        .optional()
        .map_err(|err| format!("failed to load publish log entry: {err}"))?
        .map(|(index, raw)| {
            let mut leaf_hash = [0u8; 32];
            if raw.len() != 32 {
                return Err("malformed log leaf hash".to_string());
            }
            leaf_hash.copy_from_slice(&raw);
            Ok(LogEntryRef { index, leaf_hash })
        })
        .transpose()
    }

    #[cfg(test)]
    pub fn force_expire_challenge(&self, challenge_id: &str) -> Result<(), String> {
        let conn = self.conn();
        conn.execute(
            "UPDATE auth_challenges SET expires_at = ?1 WHERE id = ?2",
            params![now_unix() - 1, challenge_id],
        )
        .map(|_| ())
        .map_err(|err| format!("failed to expire challenge: {err}"))
    }

    /// The registry's own Ed25519 keypair (plan-23 §2): the only private key
    /// the server holds. It signs attestations (and, later, log checkpoints);
    /// it can never produce a user proof. Generated once on first run.
    fn ensure_server_keypair(&self) -> Result<(), String> {
        let conn = self.conn();
        let exists: Option<i64> = conn
            .query_row("SELECT 1 FROM server_keys WHERE id = 1", [], |row| {
                row.get(0)
            })
            .optional()
            .map_err(|err| format!("failed to check server keypair: {err}"))?;
        if exists.is_none() {
            let (public, private) = crypto::generate_keypair();
            conn.execute(
                "INSERT INTO server_keys (id, public_key, private_key, created_at) VALUES (1, ?1, ?2, ?3)",
                params![public, private, now_unix()],
            )
            .map_err(|err| format!("failed to create server keypair: {err}"))?;
        }
        Ok(())
    }

    /// The server keypair. The private half must never leave the server
    /// process: it is used only to sign, and no route returns it.
    pub fn server_keypair(&self) -> Result<(Vec<u8>, Vec<u8>), String> {
        let conn = self.conn();
        conn.query_row(
            "SELECT public_key, private_key FROM server_keys WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|err| format!("failed to load server keypair: {err}"))
    }

    pub fn server_public_key(&self) -> Result<Vec<u8>, String> {
        Ok(self.server_keypair()?.0)
    }

    fn ensure_server_secret(&self) -> Result<(), String> {
        let conn = self.conn();
        let exists: Option<i64> = conn
            .query_row("SELECT 1 FROM server_secrets WHERE id = 1", [], |row| {
                row.get(0)
            })
            .optional()
            .map_err(|err| format!("failed to check server signing secret: {err}"))?;
        if exists.is_none() {
            let mut secret = vec![0u8; 32];
            rand::thread_rng().fill_bytes(&mut secret);
            conn.execute(
                "INSERT INTO server_secrets (id, secret, created_at) VALUES (1, ?1, ?2)",
                params![secret, now_unix()],
            )
            .map_err(|err| format!("failed to create server signing secret: {err}"))?;
        }
        Ok(())
    }
}

/// A reference to one transparency-log entry (plan-23-B3), returned by every
/// state-changing operation and surfaced on the wire as `logEntry`.
#[derive(Debug, Clone)]
pub struct LogEntryRef {
    pub index: i64,
    pub leaf_hash: [u8; 32],
}

/// Which root ceremony `Store::write_registry_root` is performing. The three
/// differ only in what they may do to an already-configured root, which is
/// exactly the trust-continuity question bug-584 settles.
#[derive(Clone, Copy)]
enum RootCeremony {
    /// First-time initialization; refuses if a root is already configured.
    Init,
    /// Renewal under the same offline root key; the anchor is unchanged.
    Renew,
    /// Deliberate replacement of a lost anchor; pinned clients must re-pin.
    Reanchor,
}

impl RootCeremony {
    /// The transparency-log kind recorded for this ceremony.
    fn log_kind(self) -> &'static str {
        match self {
            RootCeremony::Init => "root-init",
            RootCeremony::Renew => "root-renew",
            RootCeremony::Reanchor => "root-reanchor",
        }
    }
}

/// The signed-metadata root of trust (plan-10-C2): the root-signed `root.json`
/// plus the online snapshot/timestamp keypairs the server signs metadata with.
#[derive(Debug, Clone)]
pub struct RegistryConfig {
    pub registry_id: String,
    pub root_public: Vec<u8>,
    pub root_json: String,
    pub root_signature: Vec<u8>,
    pub snapshot_public: Vec<u8>,
    pub snapshot_private: Vec<u8>,
    pub timestamp_public: Vec<u8>,
    pub timestamp_private: Vec<u8>,
}

/// One `GET /search` hit (plan-61-B §3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchResultRow {
    pub ident: String,
    pub owner: String,
    /// The newest **active** release (plan-126-A). `None` for a package
    /// identity with no published version yet — and also for one whose every
    /// published version is yanked, blocked or legal-tombstoned.
    pub latest_version: Option<String>,
    /// The release state of `latest_version`, so a `deprecated` headline is
    /// badged rather than reading as current. `None` exactly when
    /// `latest_version` is.
    pub latest_state: Option<String>,
    pub published_at: Option<i64>,
    /// NULL until plan-61-E.
    pub description: Option<String>,
}

/// Escape a user string for use inside a `LIKE` pattern with `ESCAPE '\'`.
///
/// Without this, a query of `%` matches every package and `_` matches any
/// single character — a caller could enumerate the whole registry through a
/// route whose entire point is that an empty query returns nothing. The
/// backslash must be escaped first, or it would double-escape the escapes this
/// function itself inserts.
fn escape_like(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// One package as `GET /packages/:ident` sees it (plan-61-B).
#[derive(Debug, Clone)]
pub struct PackageDetailRow {
    pub owner: String,
    /// Every version, newest first, regardless of release state.
    pub versions: Vec<PackageDetailVersion>,
}

#[derive(Debug, Clone)]
pub struct PackageDetailVersion {
    /// `package_versions.id` — used to attach targets, not rendered.
    pub id: i64,
    pub version: String,
    pub hash: String,
    pub published_at: i64,
    pub state: String,
    pub abi_index: String,
    pub author: Option<String>,
    pub url: Option<String>,
    /// NULL until plan-61-E populates it.
    pub description: Option<String>,
    pub targets: Vec<PackageTargetRow>,
}

/// One row of the native target matrix. `arch = None` is the any-arch
/// wildcard, not missing data (plan-61-A §3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageTargetRow {
    pub os: String,
    pub arch: Option<String>,
    pub libc: Option<String>,
    pub lib_type: String,
    pub logical: String,
    pub source: String,
    pub blob_hash: Option<String>,
}

/// The transparency record for one package (plan-61-B §4).
#[derive(Debug, Clone)]
pub struct PackageAuditRow {
    pub owner: String,
    pub versions: Vec<String>,
    pub state_changes: Vec<ReleaseStateChangeRow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseStateChangeRow {
    pub version: String,
    pub state: String,
    pub at: i64,
}

/// One published version of a package (plan-10-A `/index`).
#[derive(Debug, Clone)]
pub struct PackageVersionRow {
    pub version: String,
    pub hash: String,
    pub published_at: i64,
    pub state: String,
    /// The per-symbol ABI index JSON string (plan-10-B1), `{}` when absent.
    pub abi_index: String,
    /// The publishing owner's display name (plan-168-A). `None` only for a
    /// pre-plan release whose prefix owner no longer resolves.
    pub publisher_display: Option<String>,
    /// The publisher's ident-key fingerprint, `None` for a pre-plan release.
    pub publisher_ident_fingerprint: Option<String>,
    /// The number of accepted transfers before this release was published.
    pub transfer_epoch: i64,
}

/// The terms the next transfer of a package signs (plan-168-A §4.3).
#[derive(Debug, Clone)]
pub struct TransferTerms {
    /// The number of accepted transfers so far: the next link's position.
    pub sequence: i64,
    /// The stored (current) owner, who must sign the offer.
    pub current_owner: OwnerRecord,
    /// That owner's current ident key, which signs the offer.
    pub current_ident: KeyRecord,
    /// `crypto::release_digest` over every version of the package.
    pub release_digest: String,
}

/// One accepted transfer, as `/index` serves it (plan-168-A §4.4). Every
/// `Option` is `None` exactly for a legacy link replayed from the log.
#[derive(Debug, Clone)]
pub struct TransferRecord {
    pub sequence: i64,
    pub from_owner: String,
    pub from_ident_key: Option<Vec<u8>>,
    pub to_owner: String,
    pub to_ident_key: Option<Vec<u8>>,
    pub release_digest: Option<String>,
    pub offer_signature: Option<Vec<u8>>,
    pub accept_signature: Option<Vec<u8>>,
    pub offered_at: i64,
    pub accepted_at: i64,
}

/// A pending `transfer_offers` row as `accept_transfer` reads it. The four
/// term columns are NULL on an offer recorded before signed transfers.
struct PendingOfferRow {
    id: i64,
    sequence: Option<i64>,
    from_ident_key_id: Option<i64>,
    release_digest: Option<String>,
    offer_signature: Option<Vec<u8>>,
    offered_at: i64,
}

const STALE_TRANSFER_TERMS: &str =
    "transfer terms are stale (sequence/releases changed); re-run the offer";
const NO_PENDING_OFFER: &str = "no pending transfer offer for this account";

/// The owner's current ident key, read on the caller's connection or
/// transaction.
fn current_ident_key_on(conn: &Connection, owner_id: i64) -> Result<Option<KeyRecord>, String> {
    conn.query_row(
        "SELECT id, public_key, fingerprint FROM keys
         WHERE owner_id = ?1 AND role = 'ident' AND status = 'current'",
        params![owner_id],
        |row| {
            Ok(KeyRecord {
                id: row.get(0)?,
                public_key: row.get(1)?,
                fingerprint: row.get(2)?,
            })
        },
    )
    .optional()
    .map_err(|err| format!("failed to load ident key: {err}"))
}

/// [`Store::transfer_terms`] on the caller's connection or transaction, plus
/// the package row id. `None` for an unknown package.
fn transfer_terms_on(
    conn: &Connection,
    ident: &str,
) -> Result<Option<(i64, TransferTerms)>, String> {
    let package: Option<(i64, i64, String)> = conn
        .query_row(
            "SELECT p.id, o.id, o.owner_display
             FROM packages p JOIN owners o ON o.id = p.owner_id
             WHERE p.ident = ?1",
            params![ident],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(|err| format!("failed to load package owner: {err}"))?;
    let Some((package_id, owner_id, owner_display)) = package else {
        return Ok(None);
    };
    let sequence: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM transfer_history WHERE package_id = ?1",
            params![package_id],
            |row| row.get(0),
        )
        .map_err(|err| format!("failed to count transfer history: {err}"))?;
    let Some(current_ident) = current_ident_key_on(conn, owner_id)? else {
        return Err("package owner has no current ident key".to_string());
    };
    let mut statement = conn
        .prepare("SELECT version, hash FROM package_versions WHERE package_id = ?1")
        .map_err(|err| format!("failed to prepare release digest query: {err}"))?;
    let rows = statement
        .query_map(params![package_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|err| format!("failed to load releases: {err}"))?;
    let mut releases = Vec::new();
    for row in rows {
        releases.push(row.map_err(|err| format!("failed to read release: {err}"))?);
    }
    let release_digest = crypto::release_digest(
        releases
            .iter()
            .map(|(version, hash)| (version.as_str(), hash.as_str())),
    );
    Ok(Some((
        package_id,
        TransferTerms {
            sequence,
            current_owner: OwnerRecord {
                id: owner_id,
                owner_display,
            },
            current_ident,
            release_digest,
        },
    )))
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

fn json_value(value: &str) -> String {
    serde_json::to_string(value).expect("JSON string encoding cannot fail")
}

/// The ONE place a `keys` row is created (bug-580).
///
/// Role separation is an account-wide invariant, not a per-endpoint check: an
/// account's current ident key and every current auth key must be DIFFERENT
/// keys. Domain-separated proof messages stop a proof being replayed across
/// roles; they do nothing to stop a client deliberately offering the same
/// public key for both, and every insertion path verified its proofs and then
/// inserted without comparing. Theft of a machine auth private key would then
/// also be theft of the identity-signing key, which is the whole separation the
/// design exists to provide.
///
/// Funnelling every insert through here is deliberate. The bug report named two
/// paths; there were five, and the two it missed included `issue_publish_token`
/// — the worst of them, because a publish token is a DELEGATED, exportable
/// credential, so a token equal to the ident key hands the account identity to
/// whatever CI holds the token. A per-site check would have been written from
/// that same incomplete list. `key_insertion_has_exactly_one_writer` keeps it
/// the only writer, so a sixth path cannot be added without meeting this.
///
/// The check runs INSIDE the caller's transaction, so a concurrent rotation
/// cannot slip a colliding key in between the read and the insert. In
/// `register_owner` the auth row is written first, so the ident insert's query
/// sees it and the two-argument case needs no separate comparison.
///
/// Existing accounts that already hold a colliding pair are deliberately left
/// alone: this gates the creation of NEW credentials only, and never revokes
/// live access (the bug's stated non-goal).
fn insert_key_tx(
    tx: &rusqlite::Transaction<'_>,
    owner_id: i64,
    role: &str,
    public_key: &[u8],
    fingerprint: &str,
    now: i64,
    what: &str,
) -> Result<i64, String> {
    let opposite = match role {
        KEY_ROLE_AUTH => KEY_ROLE_IDENT,
        KEY_ROLE_IDENT => KEY_ROLE_AUTH,
        other => return Err(format!("unknown key role '{other}'")),
    };
    let collision: Option<i64> = tx
        .query_row(
            "SELECT id FROM keys
             WHERE owner_id = ?1 AND role = ?2 AND status = 'current' AND fingerprint = ?3",
            params![owner_id, opposite, fingerprint],
            |row| row.get(0),
        )
        .optional()
        // Deliberately the caller's own `what` message, not a message about this
        // query. `losing_the_key_tables_errors_revocation_and_chain_reads`
        // (bug-264 / REPO-09) pins that a broken `keys` table degrades to an
        // error naming the OPERATION — "failed to register token key: …" — and
        // that contract must not depend on which statement happens to touch the
        // table first. Adding a read ahead of the insert would otherwise have
        // silently re-worded five operator-facing failures.
        .map_err(|err| format!("{what}: {err}"))?;
    if collision.is_some() {
        return Err(format!(
            "key role separation: this public key is already the account's current {opposite} \
             key. A machine auth key and the account ident key must be different keys, so that \
             losing one does not lose the other."
        ));
    }
    tx.execute(
        "INSERT INTO keys (owner_id, role, public_key, fingerprint, status, created_at, revoked_at)
         VALUES (?1, ?2, ?3, ?4, 'current', ?5, NULL)",
        params![owner_id, role, public_key, fingerprint, now],
    )
    .map_err(|err| format!("{what}: {err}"))?;
    Ok(tx.last_insert_rowid())
}

/// Append one entry to the transparency log inside an existing transaction.
/// The index is dense and monotonic; the leaf hash is the RFC 6962 leaf hash
/// of the payload bytes.
fn append_log_tx(
    tx: &rusqlite::Transaction<'_>,
    kind: &str,
    payload: &str,
) -> Result<LogEntryRef, String> {
    let index: i64 = tx
        .query_row("SELECT COUNT(*) FROM log_entries", [], |row| row.get(0))
        .map_err(|err| format!("failed to size the log: {err}"))?;
    let leaf_hash = crate::log::leaf_hash(payload.as_bytes());
    tx.execute(
        "INSERT INTO log_entries (idx, kind, payload, leaf_hash, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![index, kind, payload, leaf_hash.to_vec(), now_unix()],
    )
    .map_err(|err| format!("failed to append log entry: {err}"))?;
    Ok(LogEntryRef { index, leaf_hash })
}

pub fn now_unix() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(elapsed) => elapsed.as_secs() as i64,
        // A pre-1970 host clock used to fall through `unwrap_or_default()` to 0,
        // which silently stamps every `created_at`/`expires_at` as the epoch — so
        // freshly issued challenges, sessions and pairing blobs are born already
        // expired and nothing says why (bug-276 R7). Self-inflicted
        // misconfiguration rather than an attack, but it presents as an
        // inexplicable "everything expires instantly" outage, so say so loudly.
        Err(err) => {
            eprintln!(
                "warning: host clock is before the Unix epoch ({err}); timestamps \
                 will be recorded as 0 and time-limited records will appear expired"
            );
            0
        }
    }
}

/// Write the `package_version_targets` rows for one version, one row per
/// section-10 locator (plan-61-A §3).
///
/// Takes a `&Connection` so it serves both the publish transaction and the
/// backfill's per-version transaction. It only inserts — the backfill clears
/// the version's existing rows first, which is what makes a re-run idempotent
/// rather than duplicating every target.
fn insert_version_targets(
    conn: &Connection,
    package_version_id: i64,
    vendor_blobs: &[VendorBlobRef],
) -> Result<(), String> {
    for vendor in vendor_blobs {
        conn.execute(
            "INSERT INTO package_version_targets
                 (package_version_id, blob_hash, os, arch, libc, lib_type, logical, source)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                package_version_id,
                vendor.hash,
                vendor.os,
                vendor.arch,
                vendor.libc,
                vendor.lib_type,
                vendor.logical,
                vendor.source,
            ],
        )
        .map_err(|err| format!("failed to record version target: {err}"))?;
    }
    Ok(())
}

/// Add a column to a table if it is not already present (idempotent
/// migration). SQLite has no `ADD COLUMN IF NOT EXISTS`, so a "duplicate
/// column name" error is treated as success.
fn add_column_if_missing(conn: &Connection, table: &str, column_def: &str) -> Result<(), String> {
    match conn.execute(&format!("ALTER TABLE {table} ADD COLUMN {column_def}"), []) {
        Ok(_) => Ok(()),
        Err(err) if err.to_string().contains("duplicate column name") => Ok(()),
        Err(err) => Err(format!("failed to add column to {table}: {err}")),
    }
}

/// Whether two strings are within Levenshtein edit distance 1 (a single
/// insert, delete, or substitution), used for the typosquat warning.
/// Length-bounded candidate set for the fuzzy search tail (bug-420 item 3).
///
/// `within_edit_distance_one` can only return true when the two strings' lengths
/// differ by at most one, so the fuzzy tail never needs to look at a package
/// whose ident length is outside `[query_len - 1, query_len + 1]`. Pushing that
/// bound into SQL keeps the anonymous no-match search path from pulling every
/// `(ident, owner)` row into Rust and paying a per-row `Vec<char>` allocation +
/// Levenshtein — the O(n) full-scan amplification bug-420 flagged. It is a
/// *candidate* filter only: the exact edit-distance check still runs in Rust, so
/// results are identical to the old unbounded scan. (Idents are validated ASCII,
/// so SQLite's `length()` matches the Rust `chars().count()` the distance check
/// uses.)
fn fuzzy_search_candidates(
    conn: &Connection,
    query_len: usize,
) -> Result<Vec<(String, String)>, String> {
    let low = query_len.saturating_sub(1) as i64;
    let high = query_len.saturating_add(1) as i64;
    let mut statement = conn
        .prepare(
            "SELECT p.ident, o.owner_display FROM packages p
             JOIN owners o ON o.id = p.owner_id
             WHERE length(p.ident) BETWEEN ?1 AND ?2",
        )
        .map_err(|err| format!("failed to prepare fuzzy query: {err}"))?;
    let rows = statement
        .query_map(params![low, high], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|err| format!("failed to scan packages: {err}"))?;
    let mut candidates = Vec::new();
    for row in rows {
        candidates.push(row.map_err(|err| format!("failed to read package: {err}"))?);
    }
    Ok(candidates)
}

fn within_edit_distance_one(a: &str, b: &str) -> bool {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (la, lb) = (a.len(), b.len());
    if la.abs_diff(lb) > 1 {
        return false;
    }
    if la == lb {
        // At most one substitution.
        return a.iter().zip(&b).filter(|(x, y)| x != y).count() <= 1;
    }
    // Lengths differ by exactly one: check for a single insertion/deletion.
    let (short, long) = if la < lb { (&a, &b) } else { (&b, &a) };
    let mut i = 0usize;
    let mut j = 0usize;
    let mut edits = 0usize;
    while i < short.len() && j < long.len() {
        if short[i] == long[j] {
            i += 1;
            j += 1;
        } else {
            edits += 1;
            if edits > 1 {
                return false;
            }
            j += 1; // skip a char in the longer string
        }
    }
    true
}

fn is_unique_violation(err: &rusqlite::Error) -> bool {
    matches!(
        err,
        rusqlite::Error::SqliteFailure(code, _)
            if code.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
                || code.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_PRIMARYKEY
    )
}

// `pub(crate)` so sibling modules' tests can reuse the owner-registration
// helper below instead of re-deriving key material (see `gc::tests`).
#[cfg(test)]
pub(crate) mod tests;
