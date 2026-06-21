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
    pub fn open_repository(path: &Path) -> Result<OpenedRepository, String> {
        if path.exists() && !path.is_dir() {
            return Err(format!(
                "repository path '{}' exists but is not a directory",
                path.display()
            ));
        }
        fs::create_dir_all(path).map_err(|err| {
            format!(
                "failed to create repository path '{}': {err}",
                path.display()
            )
        })?;
        let packages_dir = path.join("packages");
        fs::create_dir_all(&packages_dir).map_err(|err| {
            format!(
                "failed to create package directory '{}': {err}",
                packages_dir.display()
            )
        })?;
        let db_path = path.join("meta.db");
        let conn = Connection::open(&db_path)
            .map_err(|err| format!("failed to open '{}': {err}", db_path.display()))?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(|err| format!("failed to enable foreign keys: {err}"))?;
        let store = Store {
            conn: Arc::new(Mutex::new(conn)),
        };
        store.migrate()?;
        store.ensure_server_secret()?;
        Ok(OpenedRepository {
            store,
            packages_dir,
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

    pub fn register_owner(
        &self,
        owner: &str,
        public_key: &[u8],
        proof: &[u8],
    ) -> Result<(OwnerRecord, KeyRecord), String> {
        validate_owner_name(owner)?;
        let message = crypto::registration_message(owner, public_key);
        crypto::verify(public_key, &message, proof)
            .map_err(|_| "invalid proof-of-possession signature".to_string())?;

        let folded = fold_owner(owner);
        let fingerprint = crypto::fingerprint(public_key);
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
            params![owner_id, public_key, fingerprint, now],
        )
        .map_err(|err| format!("failed to register auth key: {err}"))?;
        let key_id = tx.last_insert_rowid();
        tx.commit()
            .map_err(|err| format!("failed to commit registration: {err}"))?;

        Ok((
            OwnerRecord {
                id: owner_id,
                owner_display: owner.to_string(),
            },
            KeyRecord {
                id: key_id,
                public_key: public_key.to_vec(),
                fingerprint,
            },
        ))
    }

    pub fn owner_with_auth_key(&self, owner: &str) -> Result<Option<(OwnerRecord, KeyRecord)>, String> {
        let folded = fold_owner(owner);
        let conn = self.conn.lock().map_err(|_| "database lock poisoned".to_string())?;
        conn.query_row(
            "SELECT o.id, o.owner_display, k.id, k.public_key, k.fingerprint
             FROM owners o
             JOIN keys k ON k.owner_id = o.id
             WHERE o.owner_folded = ?1
               AND o.status = 'active'
               AND k.role = 'auth'
               AND k.status = 'current'",
            params![folded],
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
        let opened = Store::open_repository(temp.path()).unwrap();
        (temp, opened.store)
    }

    fn register(store: &Store, owner: &str) -> (Vec<u8>, Vec<u8>) {
        let (public, private) = crypto::generate_keypair();
        let message = crypto::registration_message(owner, &public);
        let proof = crypto::sign(&private, &message).unwrap();
        store.register_owner(owner, &public, &proof).unwrap();
        (public, private)
    }

    #[test]
    fn startup_creates_database_and_packages_dir() {
        let temp = tempfile::tempdir().unwrap();
        let opened = Store::open_repository(temp.path()).unwrap();
        assert!(temp.path().join("meta.db").is_file());
        assert!(opened.packages_dir.is_dir());
        opened.store.migrate().unwrap();
    }

    #[test]
    fn registration_persists_owner_and_key() {
        let (_temp, store) = test_store();
        let (public, _private) = register(&store, "alice");
        let (owner, key) = store.owner_with_auth_key("alice").unwrap().unwrap();
        assert_eq!(owner.owner_display, "alice");
        assert_eq!(key.public_key, public);
        assert_eq!(store.count_owners().unwrap(), 1);
    }

    #[test]
    fn duplicate_registration_is_case_folded() {
        let (_temp, store) = test_store();
        register(&store, "alice");
        let (public, private) = crypto::generate_keypair();
        let message = crypto::registration_message("Alice", &public);
        let proof = crypto::sign(&private, &message).unwrap();
        let err = store.register_owner("Alice", &public, &proof).unwrap_err();
        assert!(err.contains("already in use"));
        assert_eq!(store.count_owners().unwrap(), 1);
    }

    #[test]
    fn registration_rejects_bad_proof() {
        let (_temp, store) = test_store();
        let (public, _private) = crypto::generate_keypair();
        let (_other_public, other_private) = crypto::generate_keypair();
        let message = crypto::registration_message("alice", &public);
        let proof = crypto::sign(&other_private, &message).unwrap();
        let err = store.register_owner("alice", &public, &proof).unwrap_err();
        assert!(err.contains("invalid proof"));
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
