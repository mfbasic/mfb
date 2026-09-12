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
        let conn = self.conn();
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
                "SELECT pv.version, pv.hash, pv.created_at, pv.state, pv.abi_index
                 FROM package_versions pv
                 JOIN packages p ON p.id = pv.package_id
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
        tx.execute(
            "INSERT INTO package_versions
                 (package_id, version, hash, state, abi_index, author, url, description,
                  created_at)
             VALUES (?1, ?2, ?3, 'available', ?4, ?5, ?6, ?7, ?8)",
            params![
                package_id,
                version,
                hash,
                abi_index,
                metadata.author,
                metadata.url,
                metadata.description,
                now
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

    /// Record a transfer offer: the current owner offers `ident` to `to_owner`.
    /// Caller verifies the current owner's ident authorization first.
    pub fn create_transfer_offer(
        &self,
        ident: &str,
        from_owner: &str,
        to_owner: &str,
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
        tx.execute(
            "INSERT INTO transfer_offers (package_id, from_owner_id, to_owner_id, created_at, accepted_at)
             VALUES (?1, ?2, ?3, ?4, NULL)
             ON CONFLICT(package_id) DO UPDATE SET
               from_owner_id = excluded.from_owner_id,
               to_owner_id = excluded.to_owner_id,
               created_at = excluded.created_at,
               accepted_at = NULL",
            params![package_id, package_owner.id, to_record.id, now],
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

    /// Accept a pending transfer: re-bind the package to `to_owner` and log it.
    /// Already-published versions keep verifying against the old ident's
    /// proofs (issued facts); new versions publish under the new owner's ident.
    pub fn accept_transfer(&self, ident: &str, to_owner: &str) -> Result<(), String> {
        let Some(to_record) = self.owner_record(to_owner)? else {
            return Err("unknown recipient account".to_string());
        };
        let now = now_unix();
        let mut conn = self.conn();
        let tx = conn
            .transaction()
            .map_err(|err| format!("failed to start transfer transaction: {err}"))?;
        let offer: Option<(i64, i64)> = tx
            .query_row(
                // `o.from_owner_id = p.owner_id` is the stale-offer guard
                // (bug-274): an offer is only acceptable while the account that
                // made it is still the package's owner. Without it, an offer
                // resurrected after ownership moved — or simply left pending
                // across a transfer — could re-bind the package on the authority
                // of an account that no longer holds it.
                "SELECT o.id, o.package_id
                 FROM transfer_offers o
                 JOIN packages p ON p.id = o.package_id
                 WHERE p.ident = ?1 AND o.to_owner_id = ?2 AND o.accepted_at IS NULL
                   AND o.from_owner_id = p.owner_id",
                params![ident, to_record.id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|err| format!("failed to load transfer offer: {err}"))?;
        let Some((offer_id, package_id)) = offer else {
            return Err("no pending transfer offer for this account".to_string());
        };
        tx.execute(
            "UPDATE packages SET owner_id = ?1 WHERE id = ?2",
            params![to_record.id, package_id],
        )
        .map_err(|err| format!("failed to re-bind package owner: {err}"))?;
        tx.execute(
            "UPDATE transfer_offers SET accepted_at = ?1 WHERE id = ?2",
            params![now, offer_id],
        )
        .map_err(|err| format!("failed to close transfer offer: {err}"))?;
        append_log_tx(
            &tx,
            "transfer-accept",
            &format!(
                "{{\"ident\":{},\"to\":{}}}",
                json_value(ident),
                json_value(&to_record.owner_display),
            ),
        )?;
        tx.commit()
            .map_err(|err| format!("failed to commit transfer accept: {err}"))?;
        Ok(())
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
pub(crate) mod tests {
    use super::*;

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
        #[allow(dead_code)]
        pub(crate) ident_private: Vec<u8>,
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

    #[test]
    fn startup_creates_database_and_packages_dir() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("meta.db");
        let data_path = temp.path().join("data");
        let opened = Store::open_repository(&db_path, &data_path).unwrap();
        assert!(db_path.is_file());
        assert!(data_path.is_dir());
        opened.store.migrate().unwrap();
    }

    #[test]
    fn registration_persists_owner_and_both_keys() {
        let (_temp, store) = test_store();
        let keys = register_keys(&store, "alice");
        let (owner, auth_key) = store.owner_with_auth_key("alice").unwrap().unwrap();
        assert_eq!(owner.owner_display, "alice");
        assert_eq!(auth_key.public_key, keys.auth_public);
        let (_owner, ident_key) = store.owner_with_ident_key("alice").unwrap().unwrap();
        assert_eq!(ident_key.public_key, keys.ident_public);
        assert_ne!(auth_key.fingerprint, ident_key.fingerprint);
        assert_eq!(store.count_owners().unwrap(), 1);
    }

    #[test]
    fn poisoned_connection_lock_recovers_and_keeps_serving() {
        // bug-264 / REPO-09: a panic while the connection lock is held must not
        // permanently wedge the registry. Poison the mutex, then prove subsequent
        // reads and writes still succeed — the poison is recovered, not propagated
        // as "database lock poisoned" on every following request.
        let (_temp, store) = test_store();
        register_keys(&store, "alice");

        let poisoned = std::panic::catch_unwind(std::panic::AssertUnwindSafe({
            let store = store.clone();
            move || {
                let _guard = store.conn.lock().unwrap();
                panic!("boom while holding the connection lock");
            }
        }));
        assert!(poisoned.is_err());
        assert!(store.conn.is_poisoned());

        // The service continues despite the poisoned lock.
        assert_eq!(store.count_owners().unwrap(), 1);
        register_keys(&store, "bob");
        assert_eq!(store.count_owners().unwrap(), 2);
    }

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
    fn duplicate_registration_is_case_folded() {
        let (_temp, store) = test_store();
        register(&store, "alice");
        let (auth_public, auth_private) = crypto::generate_keypair();
        let (ident_public, ident_private) = crypto::generate_keypair();
        let auth_proof = crypto::sign(
            &auth_private,
            &crypto::registration_message(crypto::ROLE_AUTH, "Alice", &auth_public),
        )
        .unwrap();
        let ident_proof = crypto::sign(
            &ident_private,
            &crypto::registration_message(crypto::ROLE_IDENT, "Alice", &ident_public),
        )
        .unwrap();
        let err = store
            .register_owner(
                "Alice",
                &auth_public,
                &auth_proof,
                &ident_public,
                &ident_proof,
            )
            .unwrap_err();
        assert!(err.contains("already in use"));
        assert_eq!(store.count_owners().unwrap(), 1);
    }

    #[test]
    fn registration_rejects_bad_proof() {
        let (_temp, store) = test_store();
        let (auth_public, _auth_private) = crypto::generate_keypair();
        let (ident_public, ident_private) = crypto::generate_keypair();
        let (_other_public, other_private) = crypto::generate_keypair();
        let auth_proof = crypto::sign(
            &other_private,
            &crypto::registration_message(crypto::ROLE_AUTH, "alice", &auth_public),
        )
        .unwrap();
        let ident_proof = crypto::sign(
            &ident_private,
            &crypto::registration_message(crypto::ROLE_IDENT, "alice", &ident_public),
        )
        .unwrap();
        let err = store
            .register_owner(
                "alice",
                &auth_public,
                &auth_proof,
                &ident_public,
                &ident_proof,
            )
            .unwrap_err();
        assert!(err.contains("invalid auth proof"));
    }

    #[test]
    fn registration_rejects_role_replayed_proofs() {
        // A proof-of-possession signed for one role must not be accepted for
        // the other role, even with the same keypair (plan-23 Phase A1).
        let (_temp, store) = test_store();
        let (auth_public, auth_private) = crypto::generate_keypair();
        let (ident_public, ident_private) = crypto::generate_keypair();
        // Sign the ident proof with the IDENT key but over the AUTH role
        // message: replaying a role-mismatched proof must fail.
        let auth_proof = crypto::sign(
            &auth_private,
            &crypto::registration_message(crypto::ROLE_AUTH, "alice", &auth_public),
        )
        .unwrap();
        let replayed_ident_proof = crypto::sign(
            &ident_private,
            &crypto::registration_message(crypto::ROLE_AUTH, "alice", &ident_public),
        )
        .unwrap();
        let err = store
            .register_owner(
                "alice",
                &auth_public,
                &auth_proof,
                &ident_public,
                &replayed_ident_proof,
            )
            .unwrap_err();
        assert!(err.contains("invalid ident proof"));

        // And the mirror image: an ident-role proof replayed as the auth proof.
        let replayed_auth_proof = crypto::sign(
            &auth_private,
            &crypto::registration_message(crypto::ROLE_IDENT, "alice", &auth_public),
        )
        .unwrap();
        let ident_proof = crypto::sign(
            &ident_private,
            &crypto::registration_message(crypto::ROLE_IDENT, "alice", &ident_public),
        )
        .unwrap();
        let err = store
            .register_owner(
                "alice",
                &auth_public,
                &replayed_auth_proof,
                &ident_public,
                &ident_proof,
            )
            .unwrap_err();
        assert!(err.contains("invalid auth proof"));
    }

    #[test]
    fn server_keypair_is_created_once_and_stable() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("meta.db");
        let data_path = temp.path().join("data");
        let opened = Store::open_repository(&db_path, &data_path).unwrap();
        let (public, private) = opened.store.server_keypair().unwrap();
        assert_eq!(public.len(), crypto::PUBLIC_KEY_LEN);
        assert_eq!(private.len(), crypto::PRIVATE_KEY_LEN);
        assert_eq!(crypto::public_from_private(&private).unwrap(), public);
        drop(opened);
        // Re-opening the repository must keep the same keypair.
        let reopened = Store::open_repository(&db_path, &data_path).unwrap();
        assert_eq!(reopened.store.server_public_key().unwrap(), public);
    }

    /// Helper: the approval predicate every honest fetch passes.
    fn approved(_key: &[u8]) -> bool {
        true
    }

    fn is_missing(fetch: &PairingFetch) -> bool {
        matches!(fetch, PairingFetch::Missing)
    }

    #[test]
    fn pairing_blob_is_single_use_and_expires() {
        let (_temp, store) = test_store();
        register(&store, "alice");
        let owner_id = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
        let lookup = crypto::pairing_lookup("test-code");
        store
            .store_pairing_blob(owner_id, &lookup, b"ciphertext", b"salt", b"approval")
            .unwrap();

        // Wrong owner or wrong lookup yields nothing.
        assert!(is_missing(
            &store.take_pairing_blob("bob", &lookup, approved).unwrap()
        ));
        assert!(is_missing(
            &store
                .take_pairing_blob("alice", &crypto::pairing_lookup("other"), approved)
                .unwrap()
        ));

        // bug-583: a caller that fails the approval predicate is refused AND
        // does not consume the pairing — the honest fetch below still works.
        assert!(matches!(
            store
                .take_pairing_blob("alice", &lookup, |_key| false)
                .unwrap(),
            PairingFetch::Unapproved
        ));

        // First fetch succeeds; the second finds the blob consumed. The stored
        // approval key is exactly what the predicate is handed.
        let PairingFetch::Relayed { blob, salt } = store
            .take_pairing_blob("alice", &lookup, |key| key == b"approval")
            .unwrap()
        else {
            panic!("the approved fetch must relay the blob");
        };
        assert_eq!(blob, b"ciphertext");
        assert_eq!(salt, b"salt");
        assert!(is_missing(
            &store.take_pairing_blob("alice", &lookup, approved).unwrap()
        ));

        // An expired blob is never handed out.
        let expired_lookup = crypto::pairing_lookup("expired-code");
        store
            .store_pairing_blob(
                owner_id,
                &expired_lookup,
                b"ciphertext",
                b"salt",
                b"approval",
            )
            .unwrap();
        store
            .conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE pairing_blobs SET expires_at = ?1 WHERE lookup = ?2",
                params![now_unix() - 1, expired_lookup],
            )
            .unwrap();
        assert!(is_missing(
            &store
                .take_pairing_blob("alice", &expired_lookup, approved)
                .unwrap()
        ));
    }

    #[test]
    fn linked_machine_key_works_and_revocation_kills_sessions() {
        let (_temp, store) = test_store();
        let keys = register_keys(&store, "alice");

        // A second machine registers its own auth key.
        let (machine_public, machine_private) = crypto::generate_keypair();
        let proof = crypto::sign(
            &machine_private,
            &crypto::registration_message(crypto::ROLE_AUTH, "alice", &machine_public),
        )
        .unwrap();
        let (owner, machine_key) = store
            .add_auth_key("alice", &machine_public, &proof)
            .unwrap();
        let machine_fingerprint = machine_key.fingerprint.clone();

        // Both machines' keys resolve by fingerprint; each can open a session.
        let first_fingerprint = crypto::fingerprint(&keys.auth_public);
        assert!(store
            .owner_auth_key_by_fingerprint("alice", &first_fingerprint)
            .unwrap()
            .is_some());
        let challenge = store
            .create_auth_challenge("alice", &machine_fingerprint)
            .unwrap();
        let signature = crypto::sign(
            &machine_private,
            &crypto::challenge_message(&challenge.id, &challenge.nonce),
        )
        .unwrap();
        let (_owner, key) = store.complete_challenge(&challenge.id, &signature).unwrap();
        store
            .insert_session(&NewSession {
                owner_id: owner.id,
                key_id: key.id,
                jwt_id: "machine-session".to_string(),
                issued_at: now_unix(),
                expires_at: now_unix() + 3600,
            })
            .unwrap();
        assert!(store.session_exists("machine-session").unwrap());

        // Revocation flips the key and closes its sessions.
        assert!(store
            .revoke_auth_key(owner.id, &machine_fingerprint)
            .unwrap());
        assert!(store
            .owner_auth_key_by_fingerprint("alice", &machine_fingerprint)
            .unwrap()
            .is_none());
        assert!(!store.session_exists("machine-session").unwrap());
        assert!(store
            .create_auth_challenge("alice", &machine_fingerprint)
            .is_err());
        // Revoking again reports nothing to revoke.
        assert!(!store
            .revoke_auth_key(owner.id, &machine_fingerprint)
            .unwrap());
        // The first machine is untouched.
        assert!(store
            .owner_auth_key_by_fingerprint("alice", &first_fingerprint)
            .unwrap()
            .is_some());

        // bug-493: it is now the account's last current machine key. Revoking
        // it would lock the account out permanently (no ident-authorized route
        // adds an auth key), so the store refuses and the key stays current. A
        // publish token does not count as a machine — it cannot enrol one
        // (bug-492) — so its presence does not unlock the revocation...
        let (token_public, token_private) = crypto::generate_keypair();
        let token_proof = crypto::sign(
            &token_private,
            &crypto::registration_message(crypto::ROLE_AUTH, "alice", &token_public),
        )
        .unwrap();
        store
            .issue_publish_token("alice", &token_public, &token_proof, "alice#toolbox", 3600)
            .unwrap();
        assert_eq!(
            store
                .revoke_auth_key(owner.id, &first_fingerprint)
                .unwrap_err(),
            "cannot revoke the account's last auth key; link another machine first",
        );
        assert!(store
            .owner_auth_key_by_fingerprint("alice", &first_fingerprint)
            .unwrap()
            .is_some());
        // ...while the token itself, not being a machine key, may still be
        // revoked through this path.
        assert!(store
            .revoke_auth_key(owner.id, &crypto::fingerprint(&token_public))
            .unwrap());
    }

    #[test]
    fn revocation_challenge_requires_the_ident_key() {
        let (_temp, store) = test_store();
        let keys = register_keys(&store, "alice");
        let fingerprint = crypto::fingerprint(&keys.auth_public);

        // Signed with the AUTH key (auth session alone): refused.
        let challenge = store.create_ident_challenge("alice").unwrap();
        let nonce = challenge.nonce.clone();
        let bad = crypto::sign(
            &keys.auth_private,
            &crypto::revocation_message(&challenge.id, &nonce, &fingerprint),
        )
        .unwrap();
        assert!(store
            .complete_revocation_challenge(&challenge.id, &bad, &fingerprint)
            .is_err());

        // bug-493: a LOGIN challenge for the auth key, with the revocation
        // message signed by that auth key, is refused on *purpose* — not merely
        // on signature — and the refused attempt does not burn it.
        let login_challenge = store.create_auth_challenge("alice", &fingerprint).unwrap();
        let forged = crypto::sign(
            &keys.auth_private,
            &crypto::revocation_message(&login_challenge.id, &login_challenge.nonce, &fingerprint),
        )
        .unwrap();
        assert_eq!(
            store
                .complete_revocation_challenge(&login_challenge.id, &forged, &fingerprint)
                .unwrap_err(),
            "challenge was not issued for revocation",
        );
        let login = crypto::sign(
            &keys.auth_private,
            &crypto::challenge_message(&login_challenge.id, &login_challenge.nonce),
        )
        .unwrap();
        store
            .complete_challenge(&login_challenge.id, &login)
            .expect("the refused revocation did not burn the login challenge");
        // And the reverse: a revocation (ident) challenge cannot complete a login.
        let ident_challenge = store.create_ident_challenge("alice").unwrap();
        let ident_login = crypto::sign(
            &keys.ident_private,
            &crypto::challenge_message(&ident_challenge.id, &ident_challenge.nonce),
        )
        .unwrap();
        assert_eq!(
            store
                .complete_challenge(&ident_challenge.id, &ident_login)
                .unwrap_err(),
            "challenge was not issued for login",
        );

        // Signed with the ident key over a DIFFERENT fingerprint: refused
        // (the fingerprint is inside the signed bytes).
        let challenge = store.create_ident_challenge("alice").unwrap();
        let nonce = challenge.nonce.clone();
        let redirected = crypto::sign(
            &keys.ident_private,
            &crypto::revocation_message(&challenge.id, &nonce, "someone-else"),
        )
        .unwrap();
        assert!(store
            .complete_revocation_challenge(&challenge.id, &redirected, &fingerprint)
            .is_err());

        // Signed with the ident key over the right fingerprint: accepted.
        let challenge = store.create_ident_challenge("alice").unwrap();
        let nonce = challenge.nonce.clone();
        let good = crypto::sign(
            &keys.ident_private,
            &crypto::revocation_message(&challenge.id, &nonce, &fingerprint),
        )
        .unwrap();
        store
            .complete_revocation_challenge(&challenge.id, &good, &fingerprint)
            .unwrap();
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

    #[test]
    fn challenge_lifecycle_accepts_signature_once() {
        let (_temp, store) = test_store();
        let (_public, private) = register(&store, "alice");
        let challenge = store.create_challenge("alice").unwrap();
        let message = crypto::challenge_message(&challenge.id, &challenge.nonce);
        let signature = crypto::sign(&private, &message).unwrap();
        let (owner, _key) = store.complete_challenge(&challenge.id, &signature).unwrap();
        assert_eq!(owner.owner_display, "alice");
        let err = store
            .complete_challenge(&challenge.id, &signature)
            .unwrap_err();
        assert!(err.contains("reused challenge"));
    }

    #[test]
    fn challenge_rejects_bad_signature_and_unknown_owner() {
        let (_temp, store) = test_store();
        register(&store, "alice");
        assert!(store
            .create_challenge("bob")
            .unwrap_err()
            .contains("unknown owner"));
        let challenge = store.create_challenge("alice").unwrap();
        let (_public, private) = crypto::generate_keypair();
        let message = crypto::challenge_message(&challenge.id, &challenge.nonce);
        let signature = crypto::sign(&private, &message).unwrap();
        assert!(store.complete_challenge(&challenge.id, &signature).is_err());
    }

    #[test]
    fn challenge_rejects_expired_challenge() {
        let (_temp, store) = test_store();
        let (_public, private) = register(&store, "alice");
        let challenge = store.create_challenge("alice").unwrap();
        store.force_expire_challenge(&challenge.id).unwrap();
        let message = crypto::challenge_message(&challenge.id, &challenge.nonce);
        let signature = crypto::sign(&private, &message).unwrap();
        let err = store
            .complete_challenge(&challenge.id, &signature)
            .unwrap_err();
        assert!(err.contains("expired challenge"));
    }

    #[test]
    fn complete_challenge_rejects_unknown_id() {
        let (_temp, store) = test_store();
        assert!(store
            .complete_challenge("no-such-id", &[0u8; 64])
            .unwrap_err()
            .contains("unknown challenge"));
    }

    /// bug-586: the metadata database holds the server signing key, the session
    /// secret and the online snapshot/timestamp keys in plaintext. SQLite
    /// creates it and its `-wal`/`-shm` sidecars with the process umask, so
    /// under the ordinary `umask 022` they land at 0644 in a 0755 directory and
    /// any other local account can copy the credentials.
    ///
    /// The RED case is made deterministic by pre-creating the directory and an
    /// empty database file at exposed modes rather than by setting the umask.
    /// `umask` is process-global and libtest runs in parallel, so a test that
    /// set it would race every other test that creates a file; pre-setting the
    /// modes proves the same defect without that hazard, and additionally
    /// covers the upgrade path — an ALREADY deployed volume whose files are
    /// exposed today.
    #[cfg(unix)]
    #[test]
    fn open_repository_makes_the_key_bearing_database_private() {
        use std::os::unix::fs::PermissionsExt;
        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;

        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("data");
        let dbpath = dir.join("meta.db");
        fs::create_dir(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
        // A zero-length file is a valid empty SQLite database, so this is the
        // real "existing world-readable deployment" shape.
        fs::write(&dbpath, b"").unwrap();
        fs::set_permissions(&dbpath, fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(mode(&dir), 0o755, "precondition: the directory is exposed");
        assert_eq!(
            mode(&dbpath),
            0o644,
            "precondition: the database is exposed"
        );

        let opened = Store::open_repository(&dbpath, &temp.path().join("blobs")).unwrap();

        assert_eq!(mode(&dir), 0o700, "the metadata directory must be private");
        assert_eq!(mode(&dbpath), 0o600, "the database must be private");
        // The sidecars carry the same rows; protecting only meta.db would leave
        // the secrets readable in the write-ahead log.
        for suffix in ["-wal", "-shm"] {
            let sidecar = dir.join(format!("meta.db{suffix}"));
            assert!(sidecar.exists(), "WAL mode must have created {suffix}");
            assert_eq!(
                mode(&sidecar),
                0o600,
                "the {suffix} sidecar must be private"
            );
        }

        // And the keys really are in there, so the modes above are guarding
        // something: this is what makes the bug HIGH-value rather than cosmetic.
        let (_public, private) = opened.store.server_keypair().unwrap();
        assert!(!private.is_empty());
    }

    /// POSITIVE (bug-586): hardening must not break an ordinary open, and must
    /// never WIDEN an operator's deliberate choice.
    ///
    /// Without this, "refuse every open" and "chmod everything to 0600
    /// unconditionally" would both pass the test above. A `0400` database is a
    /// legitimate read-only hardening; tightening is allowed to remove access,
    /// never to grant it.
    #[cfg(unix)]
    #[test]
    fn open_repository_leaves_an_already_private_database_alone_and_still_works() {
        use std::os::unix::fs::PermissionsExt;
        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;

        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("data");
        let dbpath = dir.join("meta.db");
        fs::create_dir(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();

        // A normal open on a fresh path: the store is fully functional.
        let opened = Store::open_repository(&dbpath, &temp.path().join("blobs")).unwrap();
        let store = opened.store;
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
        assert_eq!(
            store.list_package_versions("alice#toolbox").unwrap().len(),
            1
        );
        assert_eq!(mode(&dbpath), 0o600);
        assert_eq!(mode(&dir), 0o700);
        drop(store);

        // An operator who deliberately narrowed further keeps their choice:
        // re-opening must not relax 0400 back up to 0600.
        fs::set_permissions(&dbpath, fs::Permissions::from_mode(0o400)).unwrap();
        let reopened = Store::open_repository(&dbpath, &temp.path().join("blobs"));
        assert!(reopened.is_ok(), "a 0400 database must still open");
        assert_eq!(
            mode(&dbpath),
            0o400,
            "hardening may remove access, never grant it"
        );
        let _ = keys;
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

    /// bug-580: an account's current ident key and every current auth key must
    /// be DIFFERENT keys.
    ///
    /// **This was never exploitable, and the report's premise was wrong.**
    /// Measured on the pre-fix store, all five paths already refused a
    /// role-colliding key — with `UNIQUE constraint failed: keys.fingerprint`.
    /// `keys.fingerprint` is declared `NOT NULL UNIQUE` *globally*, so no two
    /// rows anywhere can share a public key and the separation fell out of that.
    ///
    /// What this test protects is that the guarantee stops being INCIDENTAL. It
    /// came from an index whose purpose is not role separation, and bug-580's own
    /// non-goals ask for that index to be loosened ("do not prohibit two
    /// different accounts from independently choosing the same public key" — which
    /// the global UNIQUE currently does prohibit). Whoever loosens it would remove
    /// the role separation as a side effect, with nothing failing. Now
    /// `insert_key_tx` enforces the account-scoped invariant directly, so the two
    /// properties can move independently.
    ///
    /// Every path is exercised because the point is that the list is complete.
    /// The report named two; there are five, and the two it missed include
    /// `issue_publish_token` — the one that matters most, since a publish token
    /// is a delegated, exportable credential handed to CI.
    #[test]
    fn key_role_separation_is_enforced_at_every_creation_path() {
        let (_temp, store) = test_store();
        let (shared_public, shared_private) = crypto::generate_keypair();
        let proof = |role: &str, owner: &str, public: &[u8], private: &[u8]| {
            crypto::sign(private, &crypto::registration_message(role, owner, public)).unwrap()
        };

        // 1. Registration with one key in both roles.
        let err = store
            .register_owner(
                "alice",
                &shared_public,
                &proof(crypto::ROLE_AUTH, "alice", &shared_public, &shared_private),
                &shared_public,
                &proof(crypto::ROLE_IDENT, "alice", &shared_public, &shared_private),
            )
            .unwrap_err();
        assert!(err.contains("key role separation"), "{err}");
        // Nothing was persisted: the transaction rolled back whole.
        assert!(store.owner_with_ident_key("alice").unwrap().is_none());

        // A proper account, to test the remaining paths against.
        let keys = register_keys(&store, "alice");

        // 2. Linking a machine whose auth key IS the account ident key.
        let err = store
            .add_auth_key(
                "alice",
                &keys.ident_public,
                &proof(
                    crypto::ROLE_AUTH,
                    "alice",
                    &keys.ident_public,
                    &keys.ident_private,
                ),
            )
            .unwrap_err();
        assert!(err.contains("key role separation"), "{err}");

        // 3. A publish token minted on the ident key. The report missed this
        //    one, and it is the most dangerous: the token leaves the machine.
        let err = store
            .issue_publish_token(
                "alice",
                &keys.ident_public,
                &proof(
                    crypto::ROLE_AUTH,
                    "alice",
                    &keys.ident_public,
                    &keys.ident_private,
                ),
                "publish",
                3600,
            )
            .unwrap_err();
        assert!(err.contains("key role separation"), "{err}");

        // 4. Rotating the ident ONTO an existing machine auth key.
        let chain = crypto::sign(
            &keys.ident_private,
            &crypto::ident_rotation_message(
                "alice",
                &crypto::fingerprint(&keys.ident_public),
                &keys.auth_public,
            ),
        )
        .unwrap();
        let err = store
            .rotate_ident(
                "alice",
                &keys.auth_public,
                &chain,
                &proof(
                    crypto::ROLE_IDENT,
                    "alice",
                    &keys.auth_public,
                    &keys.auth_private,
                ),
            )
            .unwrap_err();
        assert!(err.contains("key role separation"), "{err}");

        // 5. Re-anchoring the ident onto an existing machine auth key.
        let err = store
            .reanchor_ident("alice", &keys.auth_public)
            .unwrap_err();
        assert!(err.contains("key role separation"), "{err}");

        // The account is intact after five refusals: each rolled back whole.
        let (_owner, ident) = store.owner_with_ident_key("alice").unwrap().unwrap();
        assert_eq!(ident.public_key, keys.ident_public);
    }

    /// POSITIVE (bug-580): distinct keys must keep working everywhere.
    ///
    /// Without this, "reject every key" passes the test above. It pins the
    /// normal shapes the invariant must NOT disturb — several machines, a
    /// publish token, a rotation, and (the bug's explicit non-goal) two
    /// different accounts independently choosing the same public key.
    #[test]
    fn distinct_keys_still_register_link_tokenize_and_rotate() {
        let (_temp, store) = test_store();
        let alice = register_keys(&store, "alice");
        let proof = |role: &str, owner: &str, public: &[u8], private: &[u8]| {
            crypto::sign(private, &crypto::registration_message(role, owner, public)).unwrap()
        };

        // Several distinct machines link to one account.
        for _ in 0..3 {
            let (machine_public, machine_private) = crypto::generate_keypair();
            store
                .add_auth_key(
                    "alice",
                    &machine_public,
                    &proof(
                        crypto::ROLE_AUTH,
                        "alice",
                        &machine_public,
                        &machine_private,
                    ),
                )
                .expect("a distinct machine key still links");
        }

        // A publish token on its own fresh key.
        let (token_public, token_private) = crypto::generate_keypair();
        store
            .issue_publish_token(
                "alice",
                &token_public,
                &proof(crypto::ROLE_AUTH, "alice", &token_public, &token_private),
                "publish",
                3600,
            )
            .expect("a distinct token key still issues");

        // A rotation onto a fresh ident.
        let (next_public, next_private) = crypto::generate_keypair();
        let chain = crypto::sign(
            &alice.ident_private,
            &crypto::ident_rotation_message(
                "alice",
                &crypto::fingerprint(&alice.ident_public),
                &next_public,
            ),
        )
        .unwrap();
        store
            .rotate_ident(
                "alice",
                &next_public,
                &chain,
                &proof(crypto::ROLE_IDENT, "alice", &next_public, &next_private),
            )
            .expect("a distinct ident still rotates");

        // The new check is per ACCOUNT (`owner_id = ?1`), so it does not itself
        // stop two accounts choosing one public key — bug-580's non-goals ask
        // for that to stay permitted.
        //
        // It is nonetheless refused today, by the GLOBAL `keys.fingerprint`
        // UNIQUE index. That is a pre-existing divergence from the stated
        // non-goal, measured rather than assumed, and it is deliberately left
        // alone here: allowing one key to authenticate as two accounts is a
        // policy decision with its own security argument, not something to
        // change while fixing an unrelated invariant. This asserts the CURRENT
        // behaviour so the divergence is visible and any future change to it is
        // a deliberate edit to this line.
        let (shared_public, shared_private) = crypto::generate_keypair();
        let (carol_ident, carol_ident_private) = crypto::generate_keypair();
        store
            .register_owner(
                "carol",
                &shared_public,
                &proof(crypto::ROLE_AUTH, "carol", &shared_public, &shared_private),
                &carol_ident,
                &proof(
                    crypto::ROLE_IDENT,
                    "carol",
                    &carol_ident,
                    &carol_ident_private,
                ),
            )
            .expect("the first account may use this key");
        let (dave_ident, dave_ident_private) = crypto::generate_keypair();
        let err = store
            .register_owner(
                "dave",
                &shared_public,
                &proof(crypto::ROLE_AUTH, "dave", &shared_public, &shared_private),
                &dave_ident,
                &proof(crypto::ROLE_IDENT, "dave", &dave_ident, &dave_ident_private),
            )
            .unwrap_err();
        assert!(
            err.contains("UNIQUE constraint failed: keys.fingerprint"),
            "a second account sharing a key is refused by the GLOBAL index, not \
             by the account-scoped role check: {err}"
        );
    }

    /// bug-580: `insert_key_tx` must stay the ONLY writer of a `keys` row.
    ///
    /// The role-separation invariant is account-wide, so it has to hold at every
    /// creation path — and the reason it did not before is that the paths were
    /// enumerated by hand and the list was wrong (the report named two of five).
    /// A per-site check would have been written from the same wrong list. This
    /// asserts the structural property instead: one writer, so a new path cannot
    /// be added without going through the check.
    ///
    /// Same shape as `tests/gate_lock_covers_every_writer.rs` — a recogniser and
    /// the thing it recognises are two lists, and they drift.
    #[test]
    fn key_insertion_has_exactly_one_writer() {
        let source = include_str!("store.rs");
        // The needle is the statement's full column list, not the bare table
        // name: a recogniser that matches its own diagnostic text counts itself
        // and is wrong by one before it has read a line of production code.
        let needle = concat!("INSERT INTO keys ", "(owner_id, role, public_key");
        let writers = source.matches(needle).count();
        assert_eq!(
            writers, 1,
            "every `keys` row must be created by `insert_key_tx`, which enforces \
             auth/ident role separation (bug-580); found {writers} insertion sites. \
             Route the new one through `insert_key_tx` rather than adding a second \
             check that will drift from this one."
        );
    }

    #[test]
    fn open_repository_rejects_non_file_db_and_non_dir_data() {
        let temp = tempfile::tempdir().unwrap();
        // A directory where the DB file should be.
        let dir_as_db = temp.path().join("db-is-a-dir");
        fs::create_dir(&dir_as_db).unwrap();
        match Store::open_repository(&dir_as_db, &temp.path().join("data")) {
            Ok(_) => panic!("a directory must not be accepted as the DB file"),
            Err(err) => assert!(err.contains("is not a file"), "{err}"),
        }
        // A file where the data directory should be.
        let file_as_data = temp.path().join("data-is-a-file");
        fs::write(&file_as_data, b"x").unwrap();
        match Store::open_repository(&temp.path().join("meta.db"), &file_as_data) {
            Ok(_) => panic!("a file must not be accepted as the data dir"),
            Err(err) => assert!(err.contains("is not a directory"), "{err}"),
        }
    }

    #[test]
    fn org_member_removal_and_listing() {
        let (_temp, store) = test_store();
        register_keys(&store, "acme");
        register_keys(&store, "alice");
        register_keys(&store, "bob");

        // Grant validation rejects a bad role and unknown accounts.
        assert!(store
            .grant_org_member("acme", "alice", "superuser")
            .is_err());
        assert!(store
            .grant_org_member("nosuchorg", "alice", "admin")
            .is_err());
        assert!(store.grant_org_member("acme", "nosuch", "admin").is_err());

        store.grant_org_member("acme", "alice", "admin").unwrap();
        store.grant_org_member("acme", "bob", "publisher").unwrap();
        // Update an existing member's role (ON CONFLICT path).
        store.grant_org_member("acme", "alice", "owner").unwrap();
        assert_eq!(
            store.org_member_role("acme", "alice").unwrap().as_deref(),
            Some("owner")
        );

        let members = store.list_org_members("acme").unwrap();
        assert_eq!(members.len(), 2);
        assert!(members.contains(&("alice".to_string(), "owner".to_string())));

        // Removal reports true, then false when there is nothing to remove.
        assert!(store.remove_org_member("acme", "bob").unwrap());
        assert!(!store.remove_org_member("acme", "bob").unwrap());
        assert!(store.remove_org_member("nosuchorg", "bob").is_err());
        assert!(store.remove_org_member("acme", "nosuch").is_err());
        assert!(store.org_member_role("acme", "carol").unwrap().is_none());
    }

    #[test]
    fn publish_token_issue_validates_scope_and_ttl() {
        let (_temp, store) = test_store();
        let keys = register_keys(&store, "alice");
        let (token_public, token_private) = crypto::generate_keypair();
        let proof = crypto::sign(
            &token_private,
            &crypto::registration_message(crypto::ROLE_AUTH, "alice", &token_public),
        )
        .unwrap();

        // Unknown owner.
        assert!(store
            .issue_publish_token("nosuch", &token_public, &proof, "nosuch#pkg", 60)
            .unwrap_err()
            .contains("unknown owner"));
        // Bad proof.
        assert!(store
            .issue_publish_token("alice", &token_public, &[0u8; 64], "alice#pkg", 60)
            .unwrap_err()
            .contains("invalid token proof"));
        // Empty and over-long scope.
        assert!(store
            .issue_publish_token("alice", &token_public, &proof, "", 60)
            .unwrap_err()
            .contains("scope"));
        assert!(store
            .issue_publish_token("alice", &token_public, &proof, &"a".repeat(300), 60)
            .unwrap_err()
            .contains("scope"));
        // Bad TTL bounds.
        assert!(store
            .issue_publish_token("alice", &token_public, &proof, "alice#pkg", 0)
            .unwrap_err()
            .contains("ttl"));
        assert!(store
            .issue_publish_token("alice", &token_public, &proof, "alice#pkg", 400 * 24 * 3600)
            .unwrap_err()
            .contains("ttl"));

        // A valid issue registers the token key + returns scope/expiry.
        let (owner, key, expires_at) = store
            .issue_publish_token("alice", &token_public, &proof, "alice#pkg", 60)
            .unwrap();
        assert_eq!(owner.owner_display, "alice");
        assert_eq!(key.fingerprint, crypto::fingerprint(&token_public));
        assert!(expires_at > now_unix());
        // publish_token_for_key reads it back.
        let (scope, exp, revoked) = store.publish_token_for_key(key.id).unwrap().unwrap();
        assert_eq!(scope, "alice#pkg");
        assert_eq!(exp, expires_at);
        assert!(revoked.is_none());
        // A non-token key yields None.
        let auth_key_id = store.owner_with_auth_key("alice").unwrap().unwrap().1.id;
        assert!(store.publish_token_for_key(auth_key_id).unwrap().is_none());

        // Revoke: true, then false; unknown owner errors.
        assert!(store
            .revoke_publish_token("alice", &key.fingerprint)
            .unwrap());
        assert!(!store
            .revoke_publish_token("alice", &key.fingerprint)
            .unwrap());
        assert!(store
            .revoke_publish_token("nosuch", &key.fingerprint)
            .is_err());
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

    /// A native PUT whose bytes are already stored as a package blob must not
    /// write a second, unreferenced `.bin` (bug-276 R5).
    ///
    /// `package_blobs` is keyed by hash alone and `GET /blob/<hash>` picks the
    /// file from that row's `kind`, so the existing object already serves these
    /// exact bytes. `record_native_blob` reports whether a `.bin` promote is
    /// warranted; on a kind collision it is not.
    #[test]
    fn recording_a_native_blob_over_a_package_blob_skips_the_bin_promote() {
        let (_temp, store) = test_store();
        register_keys(&store, "alice");
        let alice_id = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
        store
            .publish_package_version(
                alice_id,
                "alice#toolbox",
                "1.0.0",
                "sharedhash",
                "path",
                "{}",
                &[],
                &PublishMetadata::default(),
            )
            .unwrap();

        // The publish recorded `sharedhash` as a package blob.
        assert_eq!(
            store.blob_kind("sharedhash").unwrap().as_deref(),
            Some("package")
        );
        // A native upload of identical bytes must not promote a `.bin`, and must
        // leave the existing row's kind alone.
        assert!(!store
            .record_native_blob("sharedhash", "blobs/sharedhash.bin")
            .unwrap());
        assert_eq!(
            store.blob_kind("sharedhash").unwrap().as_deref(),
            Some("package"),
            "the existing row's kind must not be rewritten"
        );

        // A genuinely new native hash still records and promotes.
        assert!(store
            .record_native_blob("nativehash", "blobs/nativehash.bin")
            .unwrap());
        assert_eq!(
            store.blob_kind("nativehash").unwrap().as_deref(),
            Some("native")
        );
        // Idempotent re-upload of the same native blob still promotes.
        assert!(store
            .record_native_blob("nativehash", "blobs/nativehash.bin")
            .unwrap());
    }

    // --- Blob reachability (plan-49) ---------------------------------------

    /// The reachable set is `package_versions.hash` **∪**
    /// `package_version_blobs.hash` and **both halves are required**
    /// (plan-49 §4.1).
    ///
    /// A `.mfp` blob is not a vendor blob, so it never appears in
    /// `package_version_blobs`; dropping that first half of the union would
    /// report every published package as collectable garbage. The plan asks for
    /// this sentence to live in a test rather than only in a comment, because
    /// the failure it guards is silent and total.
    #[test]
    fn a_published_mfp_blob_is_never_a_candidate() {
        let (_temp, store) = test_store();
        register_keys(&store, "alice");
        let alice_id = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
        store
            .publish_package_version(
                alice_id,
                "alice#toolbox",
                "1.0.0",
                "mfphash",
                "data/mfphash.mfp",
                "{}",
                &[],
                &PublishMetadata::default(),
            )
            .unwrap();

        // The `.mfp` blob has no `package_version_blobs` edge at all...
        let edges: i64 = store
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM package_version_blobs WHERE hash = 'mfphash'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(edges, 0, "a .mfp is not a vendor blob");
        // ...and is nevertheless reachable, forever, via package_versions.hash.
        let ancient = now_unix() + 3650 * 86_400;
        assert!(store.unreachable_blobs(ancient, 86_400).unwrap().is_empty());
        assert_eq!(store.reachable_blobs().unwrap().len(), 1);
    }

    /// A vendor blob a live version references is never a candidate; an
    /// unreferenced one is — but only once it is older than the grace period
    /// (plan-49 §3.1/§4.2).
    #[test]
    fn candidates_are_unreferenced_blobs_past_the_grace_period() {
        let (_temp, store) = test_store();
        register_keys(&store, "alice");
        let alice_id = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
        store
            .record_native_blob("vendorhash", "data/vendorhash.bin")
            .unwrap();
        store
            .record_native_blob("orphanhash", "data/orphanhash.bin")
            .unwrap();
        store
            .publish_package_version(
                alice_id,
                "alice#toolbox",
                "1.0.0",
                "mfphash",
                "data/mfphash.mfp",
                "{}",
                &[crate::abi::vendor_ref_for_hash("vendorhash")],
                &PublishMetadata::default(),
            )
            .unwrap();

        let now = now_unix();
        // Inside the grace window nothing is a candidate — the orphan may still
        // be an upload whose publish has not landed yet.
        assert!(store.unreachable_blobs(now, 86_400).unwrap().is_empty());
        // Outside it, exactly the unreferenced blob is.
        let candidates = store.unreachable_blobs(now + 2 * 86_400, 86_400).unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].hash, "orphanhash");
        assert_eq!(candidates[0].kind, "native");
        // The `.mfp` and the referenced vendor blob are both reachable.
        let mut reachable: Vec<String> = store
            .reachable_blobs()
            .unwrap()
            .into_iter()
            .map(|row| row.hash)
            .collect();
        reachable.sort();
        assert_eq!(reachable, vec!["mfphash", "vendorhash"]);
    }

    /// plan-61-A Phase 2, gotcha 2 — the regression test the whole schema
    /// decision exists for.
    ///
    /// Two platforms shipping a byte-identical build under different `source`
    /// filenames is legal (`PROJECT_JSON_LIBRARY_SOURCE_CONFLICT` forbids two
    /// vendor locators sharing a *source*, not a *hash*) and collapses to one
    /// entry under any dedupe-by-hash accumulation. A target row is a
    /// *platform*, not a blob: dropping one here silently tells every reader
    /// the package does not run on musl.
    ///
    /// The neighbouring `package_version_blobs` edge is the opposite case and
    /// this test pins both: reachability legitimately dedupes to one row.
    #[test]
    fn two_locators_sharing_one_blob_hash_write_two_target_rows() {
        let (_temp, store) = test_store();
        register_keys(&store, "alice");
        let alice_id = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;

        let shared_hash = "sharedbytes";
        store
            .record_native_blob(shared_hash, "data/sharedbytes.bin")
            .unwrap();
        let glibc = VendorBlobRef {
            logical: "snd".to_string(),
            source: "snd-glibc.a".to_string(),
            hash: shared_hash.to_string(),
            os: "linux".to_string(),
            arch: Some("x86_64".to_string()),
            libc: Some("glibc".to_string()),
            lib_type: "vendor".to_string(),
        };
        let musl = VendorBlobRef {
            source: "snd-musl.a".to_string(),
            libc: Some("musl".to_string()),
            ..glibc.clone()
        };
        store
            .publish_package_version(
                alice_id,
                "alice#toolbox",
                "1.0.0",
                "mfphash",
                "data/mfphash.mfp",
                "{}",
                &[glibc, musl],
                &PublishMetadata::default(),
            )
            .unwrap();

        let conn = store.conn();
        let mut stmt = conn
            .prepare(
                "SELECT source, libc, blob_hash FROM package_version_targets
                 ORDER BY source",
            )
            .unwrap();
        let rows: Vec<(String, Option<String>, Option<String>)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .map(|row| row.unwrap())
            .collect();

        assert_eq!(
            rows.len(),
            2,
            "one blob under two names is two supported platforms, not one"
        );
        assert_eq!(rows[0].0, "snd-glibc.a");
        assert_eq!(rows[0].1.as_deref(), Some("glibc"));
        assert_eq!(rows[1].0, "snd-musl.a");
        assert_eq!(rows[1].1.as_deref(), Some("musl"));
        // Both rows point at the one blob that actually backs them.
        assert_eq!(rows[0].2.as_deref(), Some(shared_hash));
        assert_eq!(rows[1].2.as_deref(), Some(shared_hash));

        // The blob edge, by contrast, is about reachability and correctly
        // collapses: one blob is one thing to keep alive.
        let edges: i64 = conn
            .query_row("SELECT count(*) FROM package_version_blobs", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(edges, 1, "reachability dedupes by hash on purpose");
    }

    /// The any-arch wildcard survives the round trip to SQL as NULL, and stays
    /// distinguishable from a concrete arch (plan-61-A §3, gotcha 1).
    #[test]
    fn a_wildcard_arch_is_stored_as_null_and_stays_distinct() {
        let (_temp, store) = test_store();
        register_keys(&store, "alice");
        let alice_id = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;

        store
            .record_native_blob("machash", "data/machash.bin")
            .unwrap();
        store
            .record_native_blob("linuxhash", "data/linuxhash.bin")
            .unwrap();
        let any_arch = VendorBlobRef {
            logical: "snd".to_string(),
            source: "libsnd.dylib".to_string(),
            hash: "machash".to_string(),
            os: "macos".to_string(),
            arch: None,
            libc: None,
            lib_type: "vendor".to_string(),
        };
        let concrete = VendorBlobRef {
            source: "libsnd.a".to_string(),
            hash: "linuxhash".to_string(),
            os: "linux".to_string(),
            arch: Some("aarch64".to_string()),
            ..any_arch.clone()
        };
        store
            .publish_package_version(
                alice_id,
                "alice#toolbox",
                "1.0.0",
                "mfphash",
                "data/mfphash.mfp",
                "{}",
                &[any_arch, concrete],
                &PublishMetadata::default(),
            )
            .unwrap();

        let conn = store.conn();
        // A NULL arch is queryable as "any", which is the whole point of
        // storing it as NULL rather than ''.
        let wildcard: String = conn
            .query_row(
                "SELECT os FROM package_version_targets WHERE arch IS NULL",
                [],
                |row| row.get(0),
            )
            .expect("the wildcard locator must land as arch IS NULL");
        assert_eq!(wildcard, "macos");
        let concrete_count: i64 = conn
            .query_row(
                "SELECT count(*) FROM package_version_targets WHERE arch IS NOT NULL",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(concrete_count, 1);
        // NULL libc is "no constraint", never the empty string.
        let empty_string_libc: i64 = conn
            .query_row(
                "SELECT count(*) FROM package_version_targets WHERE libc = ''",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(empty_string_libc, 0);
    }

    /// A **yanked** version keeps its blobs (plan-49 §3.2). Yanking is a "do not
    /// resolve this by default" signal, not a deletion: lockfiles pinning the
    /// hash must keep installing. Only a version row that no longer exists
    /// releases its blobs.
    #[test]
    fn a_yanked_versions_blobs_stay_reachable() {
        let (_temp, store) = test_store();
        register_keys(&store, "alice");
        let alice_id = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
        store
            .record_native_blob("vendorhash", "data/vendorhash.bin")
            .unwrap();
        store
            .publish_package_version(
                alice_id,
                "alice#toolbox",
                "1.0.0",
                "mfphash",
                "data/mfphash.mfp",
                "{}",
                &[crate::abi::vendor_ref_for_hash("vendorhash")],
                &PublishMetadata::default(),
            )
            .unwrap();
        store
            .set_release_state(alice_id, "alice#toolbox", "1.0.0", "yanked")
            .unwrap();

        let ancient = now_unix() + 3650 * 86_400;
        assert!(
            store.unreachable_blobs(ancient, 86_400).unwrap().is_empty(),
            "yanking must not release a version's blobs"
        );
        assert_eq!(store.reachable_blobs().unwrap().len(), 2);
    }

    /// A blob shared by two versions survives the removal of one of them — the
    /// case a refcount would get wrong and a recomputed scan gets right
    /// (plan-49 §3.4).
    #[test]
    fn a_shared_blob_survives_removing_one_of_its_versions() {
        let (_temp, store) = test_store();
        register_keys(&store, "alice");
        let alice_id = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
        store
            .record_native_blob("sharedvendor", "data/sharedvendor.bin")
            .unwrap();
        for version in ["1.0.0", "1.1.0"] {
            store
                .publish_package_version(
                    alice_id,
                    "alice#toolbox",
                    version,
                    &format!("mfp-{version}"),
                    "data/mfp.mfp",
                    "{}",
                    &[crate::abi::vendor_ref_for_hash("sharedvendor")],
                    &PublishMetadata::default(),
                )
                .unwrap();
        }

        let ancient = now_unix() + 3650 * 86_400;
        assert!(store.unreachable_blobs(ancient, 86_400).unwrap().is_empty());

        // Remove 1.0.0 the way a future version-deletion feature would. There is
        // no such API today (plan-49 "Open Decisions"), so this reaches into the
        // rows directly to prove the reachability rule, not the deletion path.
        let conn = store.conn();
        conn.execute(
            "DELETE FROM package_version_blobs WHERE package_version_id IN
               (SELECT id FROM package_versions WHERE version = '1.0.0')",
            [],
        )
        .unwrap();
        // plan-61-A added `package_version_targets`, a second child table
        // referencing `package_versions`. A real deletion feature must clear it
        // too, so the simulation does — the FK would otherwise reject the
        // parent delete. The reachability assertion below is unchanged.
        conn.execute(
            "DELETE FROM package_version_targets WHERE package_version_id IN
               (SELECT id FROM package_versions WHERE version = '1.0.0')",
            [],
        )
        .unwrap();
        conn.execute("DELETE FROM package_versions WHERE version = '1.0.0'", [])
            .unwrap();
        drop(conn);

        // The shared vendor blob is still referenced by 1.1.0 and must survive;
        // only 1.0.0's own `.mfp` becomes collectable.
        let candidates: Vec<String> = store
            .unreachable_blobs(ancient, 86_400)
            .unwrap()
            .into_iter()
            .map(|row| row.hash)
            .collect();
        assert_eq!(candidates, vec!["mfp-1.0.0"]);
    }

    /// `forget_blob` reports whether it actually removed a row, so a second
    /// sweep over the same hash is a visible no-op. A row still referenced by a
    /// `package_version_blobs` edge is refused by the foreign key rather than
    /// stranding a live version's vendor blob.
    #[test]
    fn forget_blob_is_idempotent_and_fk_guarded() {
        let (_temp, store) = test_store();
        register_keys(&store, "alice");
        let alice_id = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
        store
            .record_native_blob("vendorhash", "data/vendorhash.bin")
            .unwrap();
        store
            .publish_package_version(
                alice_id,
                "alice#toolbox",
                "1.0.0",
                "mfphash",
                "data/mfphash.mfp",
                "{}",
                &[crate::abi::vendor_ref_for_hash("vendorhash")],
                &PublishMetadata::default(),
            )
            .unwrap();
        store
            .record_native_blob("orphanhash", "data/orphanhash.bin")
            .unwrap();

        assert!(store.forget_blob("orphanhash").unwrap());
        assert!(
            !store.forget_blob("orphanhash").unwrap(),
            "a second delete removes nothing"
        );
        // The referenced vendor blob cannot be deleted out from under its
        // version even if a caller asks.
        assert!(store.forget_blob("vendorhash").is_err());
    }

    /// `blob_is_reachable` answers the same question as the sweep's scan, one
    /// hash at a time — it is what the collector re-asks immediately before each
    /// delete, so a publish that lands mid-sweep spares its own blobs.
    #[test]
    fn blob_is_reachable_tracks_both_halves_of_the_union() {
        let (_temp, store) = test_store();
        register_keys(&store, "alice");
        let alice_id = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
        store
            .record_native_blob("vendorhash", "data/vendorhash.bin")
            .unwrap();
        store
            .record_native_blob("orphanhash", "data/orphanhash.bin")
            .unwrap();

        // Before the publish, neither native blob is reachable.
        assert!(!store.blob_is_reachable("vendorhash").unwrap());
        assert!(!store.blob_is_reachable("orphanhash").unwrap());
        assert!(!store.blob_is_reachable("nosuchhash").unwrap());

        store
            .publish_package_version(
                alice_id,
                "alice#toolbox",
                "1.0.0",
                "mfphash",
                "data/mfphash.mfp",
                "{}",
                &[crate::abi::vendor_ref_for_hash("vendorhash")],
                &PublishMetadata::default(),
            )
            .unwrap();

        // The publish makes the `.mfp` (package_versions.hash) and its vendor
        // blob (package_version_blobs.hash) reachable; the orphan is untouched.
        assert!(store.blob_is_reachable("mfphash").unwrap());
        assert!(store.blob_is_reachable("vendorhash").unwrap());
        assert!(!store.blob_is_reachable("orphanhash").unwrap());

        // Yanking does not release them (§3.2).
        store
            .set_release_state(alice_id, "alice#toolbox", "1.0.0", "yanked")
            .unwrap();
        assert!(store.blob_is_reachable("mfphash").unwrap());
        assert!(store.blob_is_reachable("vendorhash").unwrap());
    }

    /// A negative or overflowing grace period is refused rather than wrapping
    /// into the future and making the whole registry a candidate.
    #[test]
    fn unreachable_blobs_refuses_a_bad_grace_period() {
        let (_temp, store) = test_store();
        assert!(store
            .unreachable_blobs(0, -1)
            .unwrap_err()
            .contains("must not be negative"));
        assert!(store
            .unreachable_blobs(i64::MIN, 1)
            .unwrap_err()
            .contains("overflows the clock"));
    }

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
        register_keys(&store, "alice");
        register_keys(&store, "bob");
        register_keys(&store, "carol");
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
        store
            .create_transfer_offer("alice#toolbox", "alice", "bob")
            .unwrap();
        store.accept_transfer("alice#toolbox", "bob").unwrap();

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
        let err = store
            .accept_transfer("alice#toolbox", "carol")
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
        assert!(store
            .create_transfer_offer("alice#toolbox", "alice", "carol")
            .unwrap_err()
            .contains("does not currently own"));
    }

    #[test]
    fn transfer_offer_and_accept_error_branches() {
        let (_temp, store) = test_store();
        let alice = register_keys(&store, "alice");
        register_keys(&store, "bob");
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

        // Unknown package.
        assert!(store
            .create_transfer_offer("alice#missing", "alice", "bob")
            .unwrap_err()
            .contains("unknown package"));
        // Offering owner mismatch.
        assert!(store
            .create_transfer_offer("alice#toolbox", "bob", "alice")
            .unwrap_err()
            .contains("does not currently own"));
        // Unknown recipient.
        assert!(store
            .create_transfer_offer("alice#toolbox", "alice", "nosuch")
            .unwrap_err()
            .contains("unknown recipient"));
        // Cannot transfer to self.
        assert!(store
            .create_transfer_offer("alice#toolbox", "alice", "alice")
            .unwrap_err()
            .contains("current owner"));

        // Accept with no pending offer for the account.
        assert!(store
            .accept_transfer("alice#toolbox", "bob")
            .unwrap_err()
            .contains("no pending transfer"));
        assert!(store
            .accept_transfer("alice#toolbox", "nosuch")
            .unwrap_err()
            .contains("unknown recipient"));

        // A real offer then acceptance re-binds the package.
        store
            .create_transfer_offer("alice#toolbox", "alice", "bob")
            .unwrap();
        // Re-offering updates the existing row (ON CONFLICT path).
        store
            .create_transfer_offer("alice#toolbox", "alice", "bob")
            .unwrap();
        store.accept_transfer("alice#toolbox", "bob").unwrap();
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
        let (_temp, store) =
            store_with_versions(&[("1.5.0", "available"), ("2.0.0", "deprecated")]);
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
    fn registry_config_absent_then_present_after_init() {
        let (_temp, store) = test_store();
        assert!(store.registry_config().unwrap().is_none());
        assert!(store
            .init_registry_root("", now_unix() + 3600)
            .unwrap_err()
            .contains("registry id"));
        let root_private = store
            .init_registry_root("reg-1", now_unix() + 3600)
            .unwrap();
        let config = store.registry_config().unwrap().unwrap();
        assert_eq!(config.registry_id, "reg-1");
        assert_eq!(
            crypto::public_from_private(&root_private).unwrap(),
            config.root_public
        );
        // Re-running is NOT delegation renewal: it minted a fresh anchor
        // (bug-584, measured). It is refused; renewal is `renew_registry_root`.
        assert!(store
            .init_registry_root("reg-1", now_unix() + 7200)
            .unwrap_err()
            .contains("already initialized"));
        assert!(store.registry_config().unwrap().is_some());
    }

    /// bug-584 (positive pin): an honest renewal — the operator supplies the
    /// offline root key — bumps the version, re-delegates freshly generated
    /// online keys under a new expiry, and leaves the ANCHOR untouched, so a
    /// client pinned to the original fingerprint keeps verifying.
    #[test]
    fn renew_registry_root_keeps_the_anchor_and_rotates_the_delegated_keys() {
        let (_temp, store) = test_store();
        let expires = now_unix() + 3600;
        let root_private = store.init_registry_root("reg-1", expires).unwrap();
        let before = store.registry_config().unwrap().unwrap();

        let renewed_expires = expires + 86_400;
        let version = store
            .renew_registry_root("reg-1", renewed_expires, &root_private)
            .expect("a renewal under the configured root key is accepted");
        assert_eq!(version, 2);
        let after = store.registry_config().unwrap().unwrap();

        // The anchor — the only thing a client pins — is unchanged.
        assert_eq!(before.root_public, after.root_public);
        assert_eq!(
            crypto::fingerprint(&before.root_public),
            crypto::fingerprint(&after.root_public),
        );
        // The online delegated keys are fresh, and the new root.json verifies
        // under the same root key with the bumped version and new expiry.
        assert_ne!(before.snapshot_public, after.snapshot_public);
        assert_ne!(before.timestamp_public, after.timestamp_public);
        assert_ne!(before.root_json, after.root_json);
        crypto::verify(
            &after.root_public,
            &crypto::root_signing_input(after.root_json.as_bytes()),
            &after.root_signature,
        )
        .expect("the renewed root.json verifies under the pinned root key");
        assert!(after
            .root_json
            .contains(&format!("\"version\":2,\"expires\":{renewed_expires}")));
        assert!(after
            .root_json
            .contains(&crypto::encode_bytes(&after.snapshot_public)));
        // ...and the renewal is auditable in the transparency log.
        assert_eq!(log_kinds(&store), vec!["root-init", "root-renew"]);
    }

    /// bug-584: renewal is authenticated by possession of the offline root key.
    /// A stranger's key, a malformed key, and an uninitialized registry are all
    /// refused without touching a stored field.
    #[test]
    fn renew_registry_root_refuses_anything_but_the_configured_root_key() {
        let (_temp, store) = test_store();
        let (_stranger_public, stranger_private) = crypto::generate_keypair();
        assert!(store
            .renew_registry_root("reg-1", now_unix() + 3600, &stranger_private)
            .unwrap_err()
            .contains("not initialized"));

        let root_private = store
            .init_registry_root("reg-1", now_unix() + 3600)
            .unwrap();
        let before = store.registry_config().unwrap().unwrap();

        let err = store
            .renew_registry_root("reg-1", now_unix() + 7200, &stranger_private)
            .unwrap_err();
        assert!(
            err.contains("not this registry's offline root key"),
            "{err}"
        );
        assert!(store
            .renew_registry_root("reg-1", now_unix() + 7200, &[0u8; 3])
            .unwrap_err()
            .contains("malformed"));
        // A renewal aimed at a different registry id is refused too: the
        // registry id is part of what the pinned client checks.
        let err = store
            .renew_registry_root("reg-2", now_unix() + 7200, &root_private)
            .unwrap_err();
        assert!(err.contains("does not match the configured"), "{err}");

        let after = store.registry_config().unwrap().unwrap();
        assert_eq!(before.root_public, after.root_public);
        assert_eq!(before.root_json, after.root_json);
        assert_eq!(before.snapshot_public, after.snapshot_public);
        assert_eq!(before.timestamp_public, after.timestamp_public);
    }

    /// bug-584: replacing the anchor is still possible when the offline root
    /// key is LOST, but only as its own explicitly selected ceremony, and it is
    /// transparency-logged as a re-anchor rather than looking like a renewal.
    #[test]
    fn reanchor_registry_root_replaces_the_anchor_only_when_explicitly_selected() {
        let (_temp, store) = test_store();
        assert!(store
            .reanchor_registry_root("reg-1", now_unix() + 3600)
            .unwrap_err()
            .contains("not initialized"));
        store
            .init_registry_root("reg-1", now_unix() + 3600)
            .unwrap();
        let before = store.registry_config().unwrap().unwrap();

        let new_private = store
            .reanchor_registry_root("reg-1", now_unix() + 7200)
            .unwrap();
        let after = store.registry_config().unwrap().unwrap();
        assert_ne!(before.root_public, after.root_public);
        assert_eq!(
            crypto::public_from_private(&new_private).unwrap(),
            after.root_public
        );
        assert!(after.root_json.contains("\"version\":2"));
        assert_eq!(log_kinds(&store), vec!["root-init", "root-reanchor"]);
    }

    /// bug-584: `init-root` is one-time initialization. A second run used to
    /// mint a brand-new offline root key and overwrite the sole config row,
    /// silently replacing the anchor every pinned client verifies against —
    /// indistinguishable from registry takeover. It must refuse, and must not
    /// touch any configured field while refusing.
    #[test]
    fn init_registry_root_refuses_to_replace_a_configured_root() {
        let (_temp, store) = test_store();
        store
            .init_registry_root("reg-1", now_unix() + 3600)
            .unwrap();
        let before = store.registry_config().unwrap().unwrap();
        let err = store
            .init_registry_root("reg-1", now_unix() + 7200)
            .unwrap_err();
        assert!(err.contains("already initialized"), "{err}");
        let after = store.registry_config().unwrap().unwrap();
        assert_eq!(before.root_public, after.root_public);
        assert_eq!(before.root_json, after.root_json);
        assert_eq!(before.root_signature, after.root_signature);
        assert_eq!(before.snapshot_public, after.snapshot_public);
        assert_eq!(before.timestamp_public, after.timestamp_public);
    }

    #[test]
    fn add_auth_key_and_rotate_reject_unknown_owner_and_bad_signatures() {
        let (_temp, store) = test_store();
        let (public, _private) = crypto::generate_keypair();
        assert!(store.add_auth_key("nosuch", &public, &[0u8; 64]).is_err());
        assert!(store
            .rotate_ident("nosuch", &public, &[0u8; 64], &[0u8; 64])
            .is_err());
        assert!(store.reanchor_ident("nosuch", &public).is_err());

        let keys = register_keys(&store, "alice");
        // A too-short re-anchor key is rejected.
        assert!(store
            .reanchor_ident("alice", &[0u8; 10])
            .unwrap_err()
            .contains("malformed ident public key"));
        // A bad chain signature for rotation is rejected.
        let (new_public, _new_private) = crypto::generate_keypair();
        assert!(store
            .rotate_ident("alice", &new_public, &[0u8; 64], &[0u8; 64])
            .unwrap_err()
            .contains("invalid ident chain"));
        let _ = keys;
    }

    #[test]
    fn ident_chain_is_empty_before_rotation() {
        let (_temp, store) = test_store();
        register_keys(&store, "alice");
        assert!(store.ident_chain("alice").unwrap().is_empty());
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

    #[test]
    fn owner_name_validation_guards_every_key_and_challenge_entry_point() {
        // Every account-scoped entry point re-validates the owner name before it
        // reaches SQL or a signature check. The name is attacker-supplied on each
        // of these routes, so a single unguarded one would let an unvalidated
        // string through to `fold_owner`/the log payload.
        let (_temp, store) = test_store();
        register_keys(&store, "alice");
        let (public, _private) = crypto::generate_keypair();
        let (auth_public, auth_proof, _ident_public, _ident_proof) = registration_payload("alice");

        for bad in ["bad name!", "1leading-digit", ""] {
            let (a_pub, a_proof, i_pub, i_proof) = registration_payload(bad);
            assert!(
                store
                    .register_owner(bad, &a_pub, &a_proof, &i_pub, &i_proof)
                    .is_err(),
                "register_owner accepted {bad:?}"
            );
            assert!(
                store.create_challenge(bad).is_err(),
                "create_challenge accepted {bad:?}"
            );
            assert!(
                store.create_auth_challenge(bad, "fp").is_err(),
                "create_auth_challenge accepted {bad:?}"
            );
            assert!(
                store.create_ident_challenge(bad).is_err(),
                "create_ident_challenge accepted {bad:?}"
            );
            assert!(
                store.add_auth_key(bad, &auth_public, &auth_proof).is_err(),
                "add_auth_key accepted {bad:?}"
            );
            assert!(
                store
                    .rotate_ident(bad, &public, &[0u8; 64], &[0u8; 64])
                    .is_err(),
                "rotate_ident accepted {bad:?}"
            );
            assert!(
                store.reanchor_ident(bad, &public).is_err(),
                "reanchor_ident accepted {bad:?}"
            );
        }

        // The reserved name is refused with its own message, not a charset one.
        assert!(store
            .create_ident_challenge("std")
            .unwrap_err()
            .contains("reserved owner name"));

        // Nothing above wrote a row: alice is still the only account.
        assert_eq!(store.count_owners().unwrap(), 1);
    }

    #[test]
    fn ident_challenge_rejects_an_unknown_owner() {
        // The ident challenge is the gate in front of auth-key revocation, so a
        // name that resolves to no active account must fail closed.
        let (_temp, store) = test_store();
        register_keys(&store, "alice");
        assert_eq!(
            store.create_ident_challenge("nosuch").unwrap_err(),
            "unknown owner"
        );
        // And the valid owner still works, so the rejection is name-specific.
        assert!(store.create_ident_challenge("alice").is_ok());
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

    #[test]
    fn open_repository_reports_directories_it_cannot_create() {
        // Both paths are operator-supplied. When the parent of either is an
        // existing regular file the directory cannot be created, and the message
        // has to name which path failed — the alternative is an opaque io error
        // at first use.
        let temp = tempfile::tempdir().unwrap();
        let blocker = temp.path().join("blocker");
        fs::write(&blocker, b"not a directory").unwrap();

        let err = Store::open_repository(
            &blocker.join("nested").join("meta.db"),
            &temp.path().join("data"),
        )
        .map(|_| ())
        .unwrap_err();
        assert!(err.contains("failed to create database directory"), "{err}");

        let err = Store::open_repository(&temp.path().join("meta.db"), &blocker.join("data"))
            .map(|_| ())
            .unwrap_err();
        assert!(err.contains("failed to create data directory"), "{err}");
    }

    // --- SQL failures must degrade to an error, never a panic ---------------
    //
    // Every store method returns `Result<_, String>` on purpose: this process
    // serves HTTP, and the file's own `conn()` comment records that a single
    // reachable panic in a critical section is a full-service DoS (bug-264 /
    // REPO-09). The tests below remove a table out from under a live `Store` —
    // a botched migration or a restored-from-a-bad-backup database — and assert
    // that each reader/writer returns its documented operator-facing message
    // instead of unwrapping. They are the regression guard against any of these
    // `map_err` chains being "simplified" into an `unwrap`/`expect`.

    /// Drop tables from a live store. Foreign keys are disabled for the drop so
    /// the child rows do not block it, then re-enabled for the assertions.
    /// Every transparency-log entry kind, in log order.
    fn log_kinds(store: &Store) -> Vec<String> {
        let conn = store.conn();
        let mut statement = conn
            .prepare("SELECT kind FROM log_entries ORDER BY idx ASC")
            .unwrap();
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap();
        rows.map(|row| row.unwrap()).collect()
    }

    fn drop_tables(store: &Store, tables: &[&str]) {
        let conn = store.conn();
        conn.pragma_update(None, "foreign_keys", "OFF").unwrap();
        for table in tables {
            conn.execute_batch(&format!("DROP TABLE {table}")).unwrap();
        }
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
    }

    #[test]
    fn losing_the_owners_table_errors_every_account_lookup_instead_of_panicking() {
        let (_temp, store) = test_store();
        register_keys(&store, "alice");
        let (public, _private) = crypto::generate_keypair();
        let (auth_public, auth_proof, ident_public, ident_proof) = registration_payload("carol");
        drop_tables(&store, &["owners"]);

        assert!(store
            .count_owners()
            .unwrap_err()
            .contains("failed to count owners"));
        assert!(store
            .owner_with_auth_key("alice")
            .unwrap_err()
            .contains("failed to load owner"));
        assert!(store
            .owner_with_ident_key("alice")
            .unwrap_err()
            .contains("failed to load owner"));
        assert!(store
            .owner_auth_key_by_fingerprint("alice", "fp")
            .unwrap_err()
            .contains("failed to load owner"));
        assert!(store
            .org_member_role("acme", "alice")
            .unwrap_err()
            .contains("failed to load org member role"));

        // Challenge issuance fails closed rather than minting a challenge.
        assert!(store
            .create_challenge("alice")
            .unwrap_err()
            .contains("failed to load owner"));
        assert!(store
            .create_auth_challenge("alice", "fp")
            .unwrap_err()
            .contains("failed to load owner"));
        assert!(store
            .create_ident_challenge("alice")
            .unwrap_err()
            .contains("failed to load owner"));

        // Key management fails closed.
        assert!(store
            .add_auth_key("alice", &public, &[0u8; 64])
            .unwrap_err()
            .contains("failed to load owner"));
        assert!(store
            .rotate_ident("alice", &public, &[0u8; 64], &[0u8; 64])
            .unwrap_err()
            .contains("failed to load owner"));
        assert!(store
            .reanchor_ident("alice", &public)
            .unwrap_err()
            .contains("failed to load owner"));

        // Org / token / transfer entry points all resolve an owner first.
        assert!(store
            .grant_org_member("acme", "alice", "admin")
            .unwrap_err()
            .contains("failed to load owner"));
        assert!(store
            .remove_org_member("acme", "alice")
            .unwrap_err()
            .contains("failed to load owner"));
        assert!(store
            .issue_publish_token("alice", &public, &[0u8; 64], "alice#pkg", 60)
            .unwrap_err()
            .contains("failed to load owner"));
        assert!(store
            .revoke_publish_token("alice", "fp")
            .unwrap_err()
            .contains("failed to load owner"));
        assert!(store
            .accept_transfer("alice#toolbox", "bob")
            .unwrap_err()
            .contains("failed to load owner"));
        assert!(store
            .package_owner("alice#toolbox")
            .unwrap_err()
            .contains("failed to load package owner"));
        assert!(store
            .create_transfer_offer("alice#toolbox", "alice", "bob")
            .unwrap_err()
            .contains("failed to load package owner"));
        assert!(store
            .take_pairing_blob("alice", "lookup", approved)
            .unwrap_err()
            .contains("failed to load pairing blob"));
        assert!(store
            .record_signing_request(1, "alice#toolbox", "1.0.0", "fp")
            .is_err());

        // A registration failure that is *not* a duplicate name reports the
        // generic message, not "already in use" — misreporting a schema fault as
        // a name collision would send the operator hunting the wrong problem.
        let err = store
            .register_owner(
                "carol",
                &auth_public,
                &auth_proof,
                &ident_public,
                &ident_proof,
            )
            .unwrap_err();
        assert!(err.contains("failed to register owner"), "{err}");
        assert!(!err.contains("already in use"), "{err}");
    }

    #[test]
    fn losing_the_key_tables_errors_revocation_and_chain_reads() {
        let (_temp, store) = test_store();
        register_keys(&store, "alice");
        let owner = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
        let (public, private) = crypto::generate_keypair();
        let proof = crypto::sign(
            &private,
            &crypto::registration_message(crypto::ROLE_AUTH, "alice", &public),
        )
        .unwrap();
        drop_tables(&store, &["ident_chain", "publish_tokens"]);

        assert!(store
            .ident_chain("alice")
            .unwrap_err()
            .contains("failed to prepare chain query"));
        assert!(store
            .publish_token_for_key(1)
            .unwrap_err()
            .contains("failed to load publish token"));
        // Issuing a token registers the key first, then records the token row;
        // losing the token table must fail the whole issuance, not leave a
        // registered auth key with no scope or expiry attached to it.
        assert!(store
            .issue_publish_token("alice", &public, &proof, "alice#pkg", 60)
            .unwrap_err()
            .contains("failed to record publish token"));
        assert!(store
            .revoke_publish_token("alice", "fp")
            .unwrap_err()
            .contains("failed to load token"));

        drop_tables(&store, &["keys"]);
        assert!(store
            .revoke_auth_key(owner, "fp")
            .unwrap_err()
            .contains("failed to load auth key"));
        assert!(store
            .issue_publish_token("alice", &public, &proof, "alice#pkg", 60)
            .unwrap_err()
            .contains("failed to register token key"));
    }

    #[test]
    fn losing_the_package_tables_errors_publish_and_index_reads() {
        let (_temp, store) = test_store();
        register_keys(&store, "alice");
        let owner = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
        drop_tables(&store, &["package_version_blobs", "package_versions"]);

        assert!(store
            .list_package_versions("alice#toolbox")
            .unwrap_err()
            .contains("failed to prepare version query"));
        assert!(store
            .package_version_exists("alice#toolbox", "1.0.0")
            .unwrap_err()
            .contains("failed to check package version"));
        assert!(store
            .owner_version_count(owner)
            .unwrap_err()
            .contains("failed to count owner versions"));
        assert!(store
            .index_canonical_hash()
            .unwrap_err()
            .contains("failed to prepare index query"));
        assert!(store
            .set_release_state(owner, "alice#toolbox", "1.0.0", "yanked")
            .unwrap_err()
            .contains("failed to load package version"));
        // The GC's reachability queries read both halves of the union, so both
        // sides must report rather than treat an unreadable table as "nothing is
        // reachable" — that would make every blob a deletion candidate.
        assert!(store
            .unreachable_blobs(now_unix(), 0)
            .unwrap_err()
            .contains("failed to prepare blob reachability query"));
        assert!(store
            .reachable_blobs()
            .unwrap_err()
            .contains("failed to prepare reachable blob query"));
        assert!(store
            .blob_is_reachable("h1")
            .unwrap_err()
            .contains("failed to re-check blob reachability"));
        // A publish that cannot record its version row reports the generic
        // failure, not the "already published" duplicate message.
        let err = store
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
            .unwrap_err();
        assert!(err.contains("failed to publish package version"), "{err}");
        assert!(!err.contains("already published"), "{err}");

        drop_tables(&store, &["package_blobs"]);
        assert!(store
            .blob_kind("h1")
            .unwrap_err()
            .contains("failed to load blob kind"));
        assert!(store
            .record_native_blob("h1", "p1")
            .unwrap_err()
            .contains("failed to load native blob metadata"));

        drop_tables(&store, &["packages"]);
        assert!(store
            .typosquat_candidates("alice#toolbox")
            .unwrap_err()
            .contains("failed to prepare typosquat query"));
        assert!(store
            .publish_package_version(
                owner,
                "alice#toolbox",
                "1.0.0",
                "h1",
                "p1",
                "{}",
                &[],
                &PublishMetadata::default()
            )
            .unwrap_err()
            .contains("failed to create package identity"));
    }

    #[test]
    fn losing_the_log_table_errors_reads_and_aborts_state_changes() {
        let (_temp, store) = test_store();
        register_keys(&store, "alice");
        let owner = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
        let (auth_public, auth_proof, ident_public, ident_proof) = registration_payload("carol");
        drop_tables(&store, &["log_entries"]);

        assert!(store
            .log_size()
            .unwrap_err()
            .contains("failed to size the log"));
        assert!(store
            .log_leaf_hashes(None)
            .unwrap_err()
            .contains("failed to prepare log query"));
        assert!(store
            .publish_log_entry("alice#toolbox", "1.0.0")
            .unwrap_err()
            .contains("failed to load publish log entry"));

        // Every state change appends its log entry inside the same transaction,
        // so an unappendable log must abort the change rather than commit an
        // unlogged one. `carol` must not exist afterwards.
        assert!(store
            .register_owner(
                "carol",
                &auth_public,
                &auth_proof,
                &ident_public,
                &ident_proof
            )
            .unwrap_err()
            .contains("failed to size the log"));
        assert!(store
            .publish_package_version(
                owner,
                "alice#toolbox",
                "1.0.0",
                "h1",
                "p1",
                "{}",
                &[],
                &PublishMetadata::default()
            )
            .unwrap_err()
            .contains("failed to size the log"));
        assert_eq!(store.count_owners().unwrap(), 1);
        assert!(!store
            .package_version_exists("alice#toolbox", "1.0.0")
            .unwrap());
    }

    #[test]
    fn losing_the_session_and_challenge_tables_errors_the_auth_path() {
        let (_temp, store) = test_store();
        register_keys(&store, "alice");
        let owner = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
        drop_tables(&store, &["sessions"]);

        assert!(store
            .session_exists("jwt-1")
            .unwrap_err()
            .contains("failed to load session"));
        assert!(store
            .insert_session(&NewSession {
                owner_id: owner,
                key_id: 1,
                jwt_id: "jwt-1".to_string(),
                issued_at: now_unix(),
                expires_at: now_unix() + 60,
            })
            .unwrap_err()
            .contains("failed to store session"));
        assert!(store
            .reap_expired()
            .unwrap_err()
            .contains("failed to reap sessions"));

        drop_tables(&store, &["auth_challenges"]);
        assert!(store
            .create_challenge("alice")
            .unwrap_err()
            .contains("failed to create auth challenge"));
        assert!(store
            .complete_challenge("some-id", &[0u8; 64])
            .unwrap_err()
            .contains("failed to load auth challenge"));
        assert!(store
            .complete_revocation_challenge("some-id", &[0u8; 64], "fp")
            .unwrap_err()
            .contains("failed to load auth challenge"));
        assert!(store
            .force_expire_challenge("some-id")
            .unwrap_err()
            .contains("failed to expire challenge"));
        assert!(store
            .reap_expired()
            .unwrap_err()
            .contains("failed to reap challenges"));

        drop_tables(&store, &["pairing_blobs"]);
        assert!(store
            .store_pairing_blob(owner, "lookup", b"blob", b"salt", b"approval")
            .unwrap_err()
            .contains("failed to clear expired pairing blobs"));
    }

    #[test]
    fn losing_the_server_key_and_config_tables_errors_signing_and_metadata() {
        let (_temp, store) = test_store();
        drop_tables(&store, &["registry_config"]);
        assert!(store
            .registry_config()
            .unwrap_err()
            .contains("failed to load registry config"));

        drop_tables(&store, &["server_secrets", "server_keys"]);
        assert!(store
            .server_secret()
            .unwrap_err()
            .contains("failed to load server signing secret"));
        assert!(store
            .server_keypair()
            .unwrap_err()
            .contains("failed to load server keypair"));
        assert!(store
            .server_public_key()
            .unwrap_err()
            .contains("failed to load server keypair"));
        // The root ceremony delegates the server's attestation key, so it must
        // refuse to sign a root that names a key it could not read.
        assert!(store
            .init_registry_root("reg-1", now_unix() + 3600)
            .unwrap_err()
            .contains("failed to load server keypair"));
    }

    #[test]
    fn losing_the_org_and_transfer_tables_errors_membership_and_handover() {
        let (_temp, store) = test_store();
        register_keys(&store, "acme");
        register_keys(&store, "alice");
        register_keys(&store, "bob");
        let alice = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
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
        drop_tables(&store, &["org_members", "transfer_offers"]);

        assert!(store
            .list_org_members("acme")
            .unwrap_err()
            .contains("failed to prepare org query"));
        assert!(store
            .grant_org_member("acme", "alice", "admin")
            .unwrap_err()
            .contains("failed to record org member"));
        assert!(store
            .remove_org_member("acme", "alice")
            .unwrap_err()
            .contains("failed to remove org member"));

        assert!(store
            .create_transfer_offer("alice#toolbox", "alice", "bob")
            .unwrap_err()
            .contains("failed to record transfer offer"));
        assert!(store
            .accept_transfer("alice#toolbox", "bob")
            .unwrap_err()
            .contains("failed to load transfer offer"));
        // Ownership never moved.
        assert_eq!(
            store
                .package_owner("alice#toolbox")
                .unwrap()
                .unwrap()
                .owner_display,
            "alice"
        );

        drop_tables(&store, &["release_state_changes"]);
        let alice_id = store.package_owner("alice#toolbox").unwrap().unwrap().id;
        assert!(store
            .set_release_state(alice_id, "alice#toolbox", "1.0.0", "yanked")
            .unwrap_err()
            .contains("failed to record release-state change"));
        // The aborted transition left the version available.
        assert_eq!(
            store.list_package_versions("alice#toolbox").unwrap()[0].state,
            "available"
        );
    }
}
