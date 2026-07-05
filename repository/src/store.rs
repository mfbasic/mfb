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

    pub fn create_challenge(&self, owner: &str) -> Result<ChallengeRecord, String> {
        validate_owner_name(owner)?;
        let Some((owner, key)) = self.owner_with_auth_key(owner)? else {
            return Err("unknown owner".to_string());
        };
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
            params![id, owner.id, key.id, nonce, created_at, expires_at],
        )
        .map_err(|err| format!("failed to create auth challenge: {err}"))?;
        Ok(ChallengeRecord {
            id,
            owner_id: owner.id,
            key_id: key.id,
            nonce,
            expires_at,
            used_at: None,
        })
    }

    pub fn complete_challenge(&self, challenge_id: &str, signature: &[u8]) -> Result<(OwnerRecord, KeyRecord), String> {
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
        let message = crypto::challenge_message(&challenge.id, &challenge.nonce);
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
        tx.commit()
            .map_err(|err| format!("failed to commit publish: {err}"))?;
        Ok(PublishedVersion {
            ident: ident.to_string(),
            version: version.to_string(),
            hash: hash.to_string(),
            published_at: now,
            state: "available".to_string(),
        })
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
