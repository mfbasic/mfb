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

mod accounts;
mod blobs;
mod browse;
mod governance;
mod schema;
mod transparency;
mod versions;
use accounts::*;
pub use blobs::*;
pub use browse::*;
pub use governance::*;
use schema::*;
pub use transparency::*;
pub use versions::*;

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
