use crate::crypto;
use crate::validation::{fold_owner, validate_owner_name};
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
        let conn = Connection::open(dbpath)
            .map_err(|err| format!("failed to open '{}': {err}", dbpath.display()))?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(|err| format!("failed to enable foreign keys: {err}"))?;
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

    pub fn migrate(&self) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|_| "database lock poisoned".to_string())?;
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
                created_at INTEGER NOT NULL,
                UNIQUE(package_id, version)
            );

            CREATE TABLE IF NOT EXISTS package_blobs (
                hash TEXT PRIMARY KEY,
                path TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );
            "#,
        )
        .map_err(|err| format!("failed to migrate database: {err}"))
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
        let mut conn = self.conn.lock().map_err(|_| "database lock poisoned".to_string())?;
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

    pub fn owner_with_auth_key(&self, owner: &str) -> Result<Option<(OwnerRecord, KeyRecord)>, String> {
        self.owner_with_key(owner, "auth")
    }

    pub fn owner_with_ident_key(&self, owner: &str) -> Result<Option<(OwnerRecord, KeyRecord)>, String> {
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
        let conn = self.conn.lock().map_err(|_| "database lock poisoned".to_string())?;
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

    fn owner_with_key(&self, owner: &str, role: &str) -> Result<Option<(OwnerRecord, KeyRecord)>, String> {
        let folded = fold_owner(owner);
        let conn = self.conn.lock().map_err(|_| "database lock poisoned".to_string())?;
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
        let mut conn = self.conn.lock().map_err(|_| "database lock poisoned".to_string())?;
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

    fn create_challenge_for_key(&self, owner_id: i64, key_id: i64) -> Result<ChallengeRecord, String> {
        let id = Uuid::new_v4().to_string();
        let mut nonce = vec![0u8; 32];
        rand::thread_rng().fill_bytes(&mut nonce);
        let created_at = now_unix();
        let expires_at = created_at + 300;
        let conn = self.conn.lock().map_err(|_| "database lock poisoned".to_string())?;
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
        let conn = self.conn.lock().map_err(|_| "database lock poisoned".to_string())?;
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
        let mut conn = self.conn.lock().map_err(|_| "database lock poisoned".to_string())?;
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
        let message = crypto::registration_message(crypto::ROLE_AUTH, &owner.owner_display, public_key);
        crypto::verify(public_key, &message, proof)
            .map_err(|_| "invalid auth proof-of-possession signature".to_string())?;
        let fingerprint = crypto::fingerprint(public_key);
        let mut conn = self.conn.lock().map_err(|_| "database lock poisoned".to_string())?;
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
        let chain_message = crypto::ident_rotation_message(
            &owner.owner_display,
            &old_key.fingerprint,
            new_public,
        );
        crypto::verify(&old_key.public_key, &chain_message, chain_signature)
            .map_err(|_| "invalid ident chain signature".to_string())?;
        let possession_message =
            crypto::registration_message(crypto::ROLE_IDENT, &owner.owner_display, new_public);
        crypto::verify(new_public, &possession_message, possession_proof)
            .map_err(|_| "invalid ident proof-of-possession signature".to_string())?;

        let fingerprint = crypto::fingerprint(new_public);
        let now = now_unix();
        let mut conn = self.conn.lock().map_err(|_| "database lock poisoned".to_string())?;
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
        let mut conn = self.conn.lock().map_err(|_| "database lock poisoned".to_string())?;
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
    pub fn ident_chain(&self, owner: &str) -> Result<Vec<(Vec<u8>, Vec<u8>, Vec<u8>, i64)>, String> {
        let folded = fold_owner(owner);
        let conn = self.conn.lock().map_err(|_| "database lock poisoned".to_string())?;
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
        let mut conn = self.conn.lock().map_err(|_| "database lock poisoned".to_string())?;
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

    pub fn complete_challenge(&self, challenge_id: &str, signature: &[u8]) -> Result<(OwnerRecord, KeyRecord), String> {
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
        let mut conn = self.conn.lock().map_err(|_| "database lock poisoned".to_string())?;
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
        let conn = self.conn.lock().map_err(|_| "database lock poisoned".to_string())?;
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
        let conn = self.conn.lock().map_err(|_| "database lock poisoned".to_string())?;
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
        let conn = self.conn.lock().map_err(|_| "database lock poisoned".to_string())?;
        conn.query_row("SELECT secret FROM server_secrets WHERE id = 1", [], |row| row.get(0))
            .map_err(|err| format!("failed to load server signing secret: {err}"))
    }

    pub fn count_owners(&self) -> Result<i64, String> {
        let conn = self.conn.lock().map_err(|_| "database lock poisoned".to_string())?;
        conn.query_row("SELECT COUNT(*) FROM owners", [], |row| row.get(0))
            .map_err(|err| format!("failed to count owners: {err}"))
    }

    /// The published versions of a package, oldest first (plan-10-A `/index`).
    /// Each row carries the version, content hash, publish time, and current
    /// release state; the transparency-log entry is resolved separately.
    pub fn list_package_versions(&self, ident: &str) -> Result<Vec<PackageVersionRow>, String> {
        let conn = self.conn.lock().map_err(|_| "database lock poisoned".to_string())?;
        let mut statement = conn
            .prepare(
                "SELECT pv.version, pv.hash, pv.created_at, pv.state
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
        let conn = self.conn.lock().map_err(|_| "database lock poisoned".to_string())?;
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

    pub fn publish_package_version(
        &self,
        owner_id: i64,
        ident: &str,
        version: &str,
        hash: &str,
        blob_path: &str,
    ) -> Result<PublishedVersion, String> {
        let now = now_unix();
        let mut conn = self.conn.lock().map_err(|_| "database lock poisoned".to_string())?;
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
            "INSERT INTO package_versions (package_id, version, hash, state, created_at)
             VALUES (?1, ?2, ?3, 'available', ?4)",
            params![package_id, version, hash, now],
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

    /// The number of transparency-log entries (the tree size).
    pub fn log_size(&self) -> Result<i64, String> {
        let conn = self.conn.lock().map_err(|_| "database lock poisoned".to_string())?;
        conn.query_row("SELECT COUNT(*) FROM log_entries", [], |row| row.get(0))
            .map_err(|err| format!("failed to size the log: {err}"))
    }

    /// The ordered leaf hashes of the first `size` log entries (the whole
    /// log when `size` is None).
    pub fn log_leaf_hashes(&self, size: Option<i64>) -> Result<Vec<[u8; 32]>, String> {
        let conn = self.conn.lock().map_err(|_| "database lock poisoned".to_string())?;
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
    pub fn publish_log_entry(&self, ident: &str, version: &str) -> Result<Option<LogEntryRef>, String> {
        let payload_ident = json_value(ident);
        let payload_version = json_value(version);
        let conn = self.conn.lock().map_err(|_| "database lock poisoned".to_string())?;
        // publish payloads are canonical (`{"ident":...,"version":...,"hash":...}`),
        // so a prefix match on the two identity fields is exact.
        let prefix = format!("{{\"ident\":{payload_ident},\"version\":{payload_version},");
        conn.query_row(
            "SELECT idx, leaf_hash FROM log_entries
             WHERE kind = 'publish' AND payload LIKE ?1 || '%'
             ORDER BY idx ASC LIMIT 1",
            params![prefix],
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
        let conn = self.conn.lock().map_err(|_| "database lock poisoned".to_string())?;
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
        let conn = self.conn.lock().map_err(|_| "database lock poisoned".to_string())?;
        let exists: Option<i64> = conn
            .query_row("SELECT 1 FROM server_keys WHERE id = 1", [], |row| row.get(0))
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
        let conn = self.conn.lock().map_err(|_| "database lock poisoned".to_string())?;
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
        let conn = self.conn.lock().map_err(|_| "database lock poisoned".to_string())?;
        let exists: Option<i64> = conn
            .query_row("SELECT 1 FROM server_secrets WHERE id = 1", [], |row| row.get(0))
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

/// One published version of a package (plan-10-A `/index`).
#[derive(Debug, Clone)]
pub struct PackageVersionRow {
    pub version: String,
    pub hash: String,
    pub published_at: i64,
    pub state: String,
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
            .register_owner(owner, &auth_public, &auth_proof, &ident_public, &ident_proof)
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
            .register_owner("Alice", &auth_public, &auth_proof, &ident_public, &ident_proof)
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
            .register_owner("alice", &auth_public, &auth_proof, &ident_public, &ident_proof)
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
        let (owner, machine_key) = store.add_auth_key("alice", &machine_public, &proof).unwrap();
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
            .publish_package_version(owner_id, "alice#toolbox", "1.0.0", "hash", "path")
            .unwrap();
        assert_eq!(store.log_size().unwrap(), 3);

        // machine link
        let (machine_public, machine_private) = crypto::generate_keypair();
        let proof = crypto::sign(
            &machine_private,
            &crypto::registration_message(crypto::ROLE_AUTH, "alice", &machine_public),
        )
        .unwrap();
        let (_owner, machine_key) =
            store.add_auth_key("alice", &machine_public, &proof).unwrap();
        assert_eq!(store.log_size().unwrap(), 4);

        // auth revoke
        assert!(store.revoke_auth_key(owner_id, &machine_key.fingerprint).unwrap());
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
    fn challenge_lifecycle_accepts_signature_once() {
        let (_temp, store) = test_store();
        let (_public, private) = register(&store, "alice");
        let challenge = store.create_challenge("alice").unwrap();
        let message = crypto::challenge_message(&challenge.id, &challenge.nonce);
        let signature = crypto::sign(&private, &message).unwrap();
        let (owner, _key) = store.complete_challenge(&challenge.id, &signature).unwrap();
        assert_eq!(owner.owner_display, "alice");
        let err = store.complete_challenge(&challenge.id, &signature).unwrap_err();
        assert!(err.contains("reused challenge"));
    }

    #[test]
    fn challenge_rejects_bad_signature_and_unknown_owner() {
        let (_temp, store) = test_store();
        register(&store, "alice");
        assert!(store.create_challenge("bob").unwrap_err().contains("unknown owner"));
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
        let err = store.complete_challenge(&challenge.id, &signature).unwrap_err();
        assert!(err.contains("expired challenge"));
    }
}
