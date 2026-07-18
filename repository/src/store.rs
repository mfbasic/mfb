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
}

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
        if let Some(parent) = dbpath.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                format!(
                    "failed to create database directory '{}': {err}",
                    parent.display()
                )
            })?;
        }
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
        let store = Store {
            conn: Arc::new(Mutex::new(conn)),
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

            CREATE TABLE IF NOT EXISTS auth_challenges (
                id TEXT PRIMARY KEY,
                owner_id INTEGER NOT NULL REFERENCES owners(id),
                key_id INTEGER NOT NULL REFERENCES keys(id),
                nonce BLOB NOT NULL,
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
                created_at INTEGER NOT NULL,
                UNIQUE(package_id, version)
            );

            CREATE TABLE IF NOT EXISTS package_blobs (
                hash TEXT PRIMARY KEY,
                path TEXT NOT NULL,
                created_at INTEGER NOT NULL
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
        tx.execute(
            "INSERT INTO keys (owner_id, role, public_key, fingerprint, status, created_at, revoked_at)
             VALUES (?1, 'auth', ?2, ?3, 'current', ?4, NULL)",
            params![owner_id, auth_key, auth_fingerprint, now],
        )
        .map_err(|err| format!("failed to register auth key: {err}"))?;
        let auth_key_id = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO keys (owner_id, role, public_key, fingerprint, status, created_at, revoked_at)
             VALUES (?1, 'ident', ?2, ?3, 'current', ?4, NULL)",
            params![owner_id, ident_key, ident_fingerprint, now],
        )
        .map_err(|err| format!("failed to register ident key: {err}"))?;
        let ident_key_id = tx.last_insert_rowid();
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
        self.create_challenge_for_key(owner.id, key.id)
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
        self.create_challenge_for_key(owner.id, key.id)
    }

    /// Challenge the owner's ident key: proves possession of the account
    /// identity for ident-authorized operations (auth-key revocation).
    pub fn create_ident_challenge(&self, owner: &str) -> Result<ChallengeRecord, String> {
        validate_owner_name(owner)?;
        let Some((owner, key)) = self.owner_with_ident_key(owner)? else {
            return Err("unknown owner".to_string());
        };
        self.create_challenge_for_key(owner.id, key.id)
    }

    fn create_challenge_for_key(
        &self,
        owner_id: i64,
        key_id: i64,
    ) -> Result<ChallengeRecord, String> {
        let id = Uuid::new_v4().to_string();
        let mut nonce = vec![0u8; 32];
        rand::thread_rng().fill_bytes(&mut nonce);
        let created_at = now_unix();
        let expires_at = created_at + 300;
        let conn = self.conn();
        conn.execute(
            "INSERT INTO auth_challenges
             (id, owner_id, key_id, nonce, created_at, expires_at, used_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL)",
            params![id, owner_id, key_id, nonce, created_at, expires_at],
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
            "INSERT INTO pairing_blobs (owner_id, lookup, blob, salt, created_at, expires_at, used_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL)",
            params![owner_id, lookup, blob, salt, now, expires_at],
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
    pub fn take_pairing_blob(
        &self,
        owner: &str,
        lookup: &str,
    ) -> Result<Option<(Vec<u8>, Vec<u8>)>, String> {
        let folded = fold_owner(owner);
        let now = now_unix();
        let mut conn = self.conn();
        let tx = conn
            .transaction()
            .map_err(|err| format!("failed to start pairing transaction: {err}"))?;
        let row: Option<(i64, Vec<u8>, Vec<u8>)> = tx
            .query_row(
                "SELECT p.id, p.blob, p.salt
                 FROM pairing_blobs p
                 JOIN owners o ON o.id = p.owner_id
                 WHERE p.lookup = ?1
                   AND o.owner_folded = ?2
                   AND o.status = 'active'
                   AND p.used_at IS NULL
                   AND p.expires_at > ?3",
                params![lookup, folded, now],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(|err| format!("failed to load pairing blob: {err}"))?;
        let Some((id, blob, salt)) = row else {
            tx.commit().ok();
            return Ok(None);
        };
        tx.execute(
            "UPDATE pairing_blobs SET used_at = ?1, blob = x'' WHERE id = ?2",
            params![now, id],
        )
        .map_err(|err| format!("failed to consume pairing blob: {err}"))?;
        tx.commit()
            .map_err(|err| format!("failed to commit pairing fetch: {err}"))?;
        Ok(Some((blob, salt)))
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
        tx.execute(
            "INSERT INTO keys (owner_id, role, public_key, fingerprint, status, created_at, revoked_at)
             VALUES (?1, 'auth', ?2, ?3, 'current', ?4, NULL)",
            params![owner.id, public_key, fingerprint, now_unix()],
        )
        .map_err(|err| format!("failed to register machine auth key: {err}"))?;
        let key_id = tx.last_insert_rowid();
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
        tx.execute(
            "INSERT INTO keys (owner_id, role, public_key, fingerprint, status, created_at, revoked_at)
             VALUES (?1, 'ident', ?2, ?3, 'current', ?4, NULL)",
            params![owner.id, new_public, fingerprint, now],
        )
        .map_err(|err| format!("failed to register rotated ident key: {err}"))?;
        let new_key_id = tx.last_insert_rowid();
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
        tx.execute(
            "INSERT INTO keys (owner_id, role, public_key, fingerprint, status, created_at, revoked_at)
             VALUES (?1, 'ident', ?2, ?3, 'current', ?4, NULL)",
            params![owner.id, new_public, fingerprint, now],
        )
        .map_err(|err| format!("failed to register re-anchored ident key: {err}"))?;
        let key_id = tx.last_insert_rowid();
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

    pub fn complete_challenge(
        &self,
        challenge_id: &str,
        signature: &[u8],
    ) -> Result<(OwnerRecord, KeyRecord), String> {
        self.complete_challenge_with(challenge_id, signature, crypto::challenge_message)
    }

    /// Complete an ident challenge whose signature covers the revocation
    /// message (challenge + the fingerprint being revoked).
    pub fn complete_revocation_challenge(
        &self,
        challenge_id: &str,
        signature: &[u8],
        fingerprint: &str,
    ) -> Result<(OwnerRecord, KeyRecord), String> {
        self.complete_challenge_with(challenge_id, signature, |id, nonce| {
            crypto::revocation_message(id, nonce, fingerprint)
        })
    }

    fn complete_challenge_with(
        &self,
        challenge_id: &str,
        signature: &[u8],
        message: impl Fn(&str, &[u8]) -> Vec<u8>,
    ) -> Result<(OwnerRecord, KeyRecord), String> {
        let mut conn = self.conn();
        let tx = conn
            .transaction()
            .map_err(|err| format!("failed to start login transaction: {err}"))?;
        let loaded = tx
            .query_row(
                "SELECT c.id, c.owner_id, c.key_id, c.nonce, c.expires_at, c.used_at,
                        o.owner_display, k.public_key, k.fingerprint
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
                    ))
                },
            )
            .optional()
            .map_err(|err| format!("failed to load auth challenge: {err}"))?;
        let Some((challenge, owner, key)) = loaded else {
            return Err("unknown challenge".to_string());
        };
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
            "INSERT OR IGNORE INTO package_blobs (hash, path, created_at)
             VALUES (?1, ?2, ?3)",
            params![hash, blob_path, now],
        )
        .map_err(|err| format!("failed to store package blob metadata: {err}"))?;
        tx.execute(
            "INSERT INTO package_versions (package_id, version, hash, state, abi_index, created_at)
             VALUES (?1, ?2, ?3, 'available', ?4, ?5)",
            params![package_id, version, hash, abi_index, now],
        )
        .map_err(|err| {
            if is_unique_violation(&err) {
                format!("package version {ident}@{version} is already published")
            } else {
                format!("failed to publish package version: {err}")
            }
        })?;
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
        tx.execute(
            "INSERT INTO keys (owner_id, role, public_key, fingerprint, status, created_at, revoked_at)
             VALUES (?1, 'auth', ?2, ?3, 'current', ?4, NULL)",
            params![owner_record.id, token_public, fingerprint, now],
        )
        .map_err(|err| format!("failed to register token key: {err}"))?;
        let key_id = tx.last_insert_rowid();
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
        let package_id: i64 = {
            let conn = self.conn();
            conn.query_row(
                "SELECT id FROM packages WHERE ident = ?1",
                params![ident],
                |row| row.get(0),
            )
            .map_err(|err| format!("failed to load package: {err}"))?
        };
        let now = now_unix();
        let mut conn = self.conn();
        let tx = conn
            .transaction()
            .map_err(|err| format!("failed to start transfer transaction: {err}"))?;
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
                "SELECT o.id, o.package_id
                 FROM transfer_offers o
                 JOIN packages p ON p.id = o.package_id
                 WHERE p.ident = ?1 AND o.to_owner_id = ?2 AND o.accepted_at IS NULL",
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
    /// operator to store offline. Re-running bumps the root version and
    /// re-delegates (root-key renewal / delegated-key rotation). The root
    /// private key never touches the serving host's database.
    pub fn init_registry_root(
        &self,
        registry_id: &str,
        expires_at: i64,
    ) -> Result<Vec<u8>, String> {
        if registry_id.is_empty() || registry_id.len() > 255 {
            return Err("registry id must be 1..=255 bytes".to_string());
        }
        let (server_public, _server_private) = self.server_keypair()?;
        let (root_public, root_private) = crypto::generate_keypair();
        let (snapshot_public, snapshot_private) = crypto::generate_keypair();
        let (timestamp_public, timestamp_private) = crypto::generate_keypair();
        let now = now_unix();
        let conn = self.conn();
        let previous_version: i64 = conn
            .query_row(
                "SELECT root_version FROM registry_config WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(|err| format!("failed to read registry config: {err}"))?
            .unwrap_or(0);
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
            &root_private,
            &crypto::root_signing_input(root_json.as_bytes()),
        )?;
        conn.execute(
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
        Ok(root_private)
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
    pub fn set_release_state(
        &self,
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
                 WHERE p.ident = ?1 AND pv.version = ?2",
                params![ident, version],
                |row| row.get(0),
            )
            .optional()
            .map_err(|err| format!("failed to load package version: {err}"))?;
        let Some(package_version_id) = package_version_id else {
            return Err(format!(
                "package version {ident}@{version} is not published"
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

    /// The ordered leaf hashes of the first `size` log entries (the whole
    /// log when `size` is None).
    pub fn log_leaf_hashes(&self, size: Option<i64>) -> Result<Vec<[u8; 32]>, String> {
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
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
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

#[cfg(test)]
mod tests {
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
            .publish_package_version(axb_id, "axb#pkg", "1.0.0", "hash", "path", "{}")
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
                    .publish_package_version(id, ident, "1.0.0", "h", "p", "{}")
                    .is_err(),
                "{ident} should be rejected"
            );
        }
        for version in ["1.0 0", "1.0\"0", "1.0%0", "1/0", "1.0\n0", ""] {
            assert!(
                store
                    .publish_package_version(id, "alice#pkg", version, "h", "p", "{}")
                    .is_err(),
                "{version} should be rejected"
            );
        }
        // A clean publish still works.
        assert!(store
            .publish_package_version(id, "alice#pkg", "1.0.0", "h", "p", "{}")
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

    #[test]
    fn pairing_blob_is_single_use_and_expires() {
        let (_temp, store) = test_store();
        register(&store, "alice");
        let owner_id = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
        let lookup = crypto::pairing_lookup("test-code");
        store
            .store_pairing_blob(owner_id, &lookup, b"ciphertext", b"salt")
            .unwrap();

        // Wrong owner or wrong lookup yields nothing.
        assert!(store.take_pairing_blob("bob", &lookup).unwrap().is_none());
        assert!(store
            .take_pairing_blob("alice", &crypto::pairing_lookup("other"))
            .unwrap()
            .is_none());

        // First fetch succeeds; the second finds the blob consumed.
        let (blob, salt) = store.take_pairing_blob("alice", &lookup).unwrap().unwrap();
        assert_eq!(blob, b"ciphertext");
        assert_eq!(salt, b"salt");
        assert!(store.take_pairing_blob("alice", &lookup).unwrap().is_none());

        // An expired blob is never handed out.
        let expired_lookup = crypto::pairing_lookup("expired-code");
        store
            .store_pairing_blob(owner_id, &expired_lookup, b"ciphertext", b"salt")
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
        assert!(store
            .take_pairing_blob("alice", &expired_lookup)
            .unwrap()
            .is_none());
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
            .publish_package_version(owner_id, "alice#toolbox", "1.0.0", "hash", "path", "{}")
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
            .publish_package_version(owner_id, "alice#toolbox", "1.0.0", "hash", "path", "{}")
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

    #[test]
    fn transfer_offer_and_accept_error_branches() {
        let (_temp, store) = test_store();
        let alice = register_keys(&store, "alice");
        register_keys(&store, "bob");
        let alice_id = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
        store
            .publish_package_version(alice_id, "alice#toolbox", "1.0.0", "hash", "path", "{}")
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

    #[test]
    fn set_release_state_rejects_unpublished_and_updates_published() {
        let (_temp, store) = test_store();
        let keys = register_keys(&store, "alice");
        let owner_id = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
        assert!(store
            .set_release_state("alice#toolbox", "1.0.0", "yanked")
            .unwrap_err()
            .contains("is not published"));
        store
            .publish_package_version(owner_id, "alice#toolbox", "1.0.0", "hash", "path", "{}")
            .unwrap();
        store
            .set_release_state("alice#toolbox", "1.0.0", "deprecated")
            .unwrap();
        let versions = store.list_package_versions("alice#toolbox").unwrap();
        assert_eq!(versions[0].state, "deprecated");
        let _ = keys;
    }

    #[test]
    fn index_canonical_hash_changes_with_index_state() {
        let (_temp, store) = test_store();
        let keys = register_keys(&store, "alice");
        let owner_id = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
        let empty = store.index_canonical_hash().unwrap();
        store
            .publish_package_version(owner_id, "alice#toolbox", "1.0.0", "hash", "path", "{}")
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
        // Re-running bumps the root version (delegation renewal).
        store
            .init_registry_root("reg-1", now_unix() + 7200)
            .unwrap();
        assert!(store.registry_config().unwrap().is_some());
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
            .publish_package_version(owner_id, "alice#toolbox", "1.0.0", "hash", "path", "{}")
            .unwrap();
        assert!(store
            .package_version_exists("alice#toolbox", "1.0.0")
            .unwrap());
        // A duplicate publish is rejected.
        assert!(store
            .publish_package_version(owner_id, "alice#toolbox", "1.0.0", "hash", "path", "{}")
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
            .publish_package_version(alice, "alice#toolbox", "1.0.0", "h1", "p1", "{}")
            .unwrap();
        store
            .publish_package_version(alice, "alice#toolbox", "1.1.0", "h2", "p2", "{}")
            .unwrap();
        store
            .publish_package_version(alice, "alice#widgets", "0.1.0", "h3", "p3", "{}")
            .unwrap();
        store
            .publish_package_version(bob, "bob#thing", "1.0.0", "h4", "p4", "{}")
            .unwrap();
        // Three versions across two of alice's packages; bob's row is not counted.
        assert_eq!(store.owner_version_count(alice).unwrap(), 3);
        assert_eq!(store.owner_version_count(bob).unwrap(), 1);
    }
}
