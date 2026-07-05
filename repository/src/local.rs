use crate::crypto;
use std::fs;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[derive(Debug, Clone)]
pub struct LocalPaths {
    home: PathBuf,
}

impl LocalPaths {
    pub fn new(home: PathBuf) -> Self {
        Self { home }
    }

    pub fn from_env() -> Result<Self, String> {
        if let Ok(home) = std::env::var("MFB_HOME") {
            return Ok(Self::new(PathBuf::from(home)));
        }
        let home = std::env::var("HOME").map_err(|_| "HOME is not set".to_string())?;
        Ok(Self::new(PathBuf::from(home).join(".mfb")))
    }

    pub fn keys_dir(&self) -> PathBuf {
        self.home.join("keys")
    }

    pub fn session_dir(&self) -> PathBuf {
        self.home.join("session")
    }

    /// Per-machine auth keypair (plan-23 §3.1): `<owner>.auth.{pub,prv}`.
    pub fn auth_public_key_path(&self, owner: &str) -> PathBuf {
        self.keys_dir().join(format!("{owner}.auth.pub"))
    }

    pub fn auth_private_key_path(&self, owner: &str) -> PathBuf {
        self.keys_dir().join(format!("{owner}.auth.prv"))
    }

    /// Account ident keypair (plan-23 §3.1): `<owner>.ident.{pub,prv}`.
    /// Present on every linked machine; linking copies it.
    pub fn ident_public_key_path(&self, owner: &str) -> PathBuf {
        self.keys_dir().join(format!("{owner}.ident.pub"))
    }

    pub fn ident_private_key_path(&self, owner: &str) -> PathBuf {
        self.keys_dir().join(format!("{owner}.ident.prv"))
    }

    /// The pinned registry public key (plan-23 index §10.3): fetched from
    /// `GET /ident` on first contact and pinned thereafter.
    pub fn server_key_path(&self) -> PathBuf {
        self.home.join("server.pub")
    }

    pub fn session_path(&self, owner: &str) -> PathBuf {
        self.session_dir().join(format!("{owner}.ses"))
    }

    /// The last-seen transparency-log checkpoint (plan-23-B3):
    /// `<size> <root-hex>`, used to reject registry log rollbacks.
    pub fn checkpoint_path(&self) -> PathBuf {
        self.home.join("checkpoint")
    }
}

/// Persist the last-seen log checkpoint.
pub fn write_checkpoint(paths: &LocalPaths, size: i64, root_hex: &str) -> Result<(), String> {
    create_private_dir(&paths.home)?;
    write_private_file(&paths.checkpoint_path(), &format!("{size} {root_hex}"))
}

/// Read the pinned log checkpoint, if any: `(size, root-hex)`.
pub fn read_checkpoint(paths: &LocalPaths) -> Result<Option<(i64, String)>, String> {
    let path = paths.checkpoint_path();
    if !path.is_file() {
        return Ok(None);
    }
    let value = fs::read_to_string(&path)
        .map_err(|err| format!("failed to read checkpoint '{}': {err}", path.display()))?;
    let mut parts = value.trim().splitn(2, ' ');
    let size = parts
        .next()
        .and_then(|size| size.parse::<i64>().ok())
        .ok_or_else(|| "malformed pinned checkpoint".to_string())?;
    let root = parts
        .next()
        .ok_or_else(|| "malformed pinned checkpoint".to_string())?
        .to_string();
    Ok(Some((size, root)))
}

pub fn write_auth_keypair(paths: &LocalPaths, owner: &str, public: &[u8], private: &[u8]) -> Result<(), String> {
    create_private_dir(&paths.keys_dir())?;
    write_private_file(&paths.auth_public_key_path(owner), &crypto::encode_bytes(public))?;
    write_private_file(&paths.auth_private_key_path(owner), &crypto::encode_bytes(private))?;
    Ok(())
}

pub fn write_ident_keypair(paths: &LocalPaths, owner: &str, public: &[u8], private: &[u8]) -> Result<(), String> {
    create_private_dir(&paths.keys_dir())?;
    write_private_file(&paths.ident_public_key_path(owner), &crypto::encode_bytes(public))?;
    write_private_file(&paths.ident_private_key_path(owner), &crypto::encode_bytes(private))?;
    Ok(())
}

pub fn remove_owner_keys(paths: &LocalPaths, owner: &str) {
    let _ = fs::remove_file(paths.auth_public_key_path(owner));
    let _ = fs::remove_file(paths.auth_private_key_path(owner));
    let _ = fs::remove_file(paths.ident_public_key_path(owner));
    let _ = fs::remove_file(paths.ident_private_key_path(owner));
}

pub fn read_auth_public_key(paths: &LocalPaths, owner: &str) -> Result<Vec<u8>, String> {
    read_key_file(&paths.auth_public_key_path(owner), "local auth public key")
}

pub fn read_auth_private_key(paths: &LocalPaths, owner: &str) -> Result<Vec<u8>, String> {
    let path = paths.auth_private_key_path(owner);
    let value = fs::read_to_string(&path)
        .map_err(|err| format!("missing local private key '{}': {err}", path.display()))?;
    crypto::decode_bytes(value.trim(), "local auth private key")
}

pub fn read_ident_public_key(paths: &LocalPaths, owner: &str) -> Result<Vec<u8>, String> {
    read_key_file(&paths.ident_public_key_path(owner), "local ident public key")
}

pub fn read_ident_private_key(paths: &LocalPaths, owner: &str) -> Result<Vec<u8>, String> {
    let path = paths.ident_private_key_path(owner);
    let value = fs::read_to_string(&path)
        .map_err(|err| format!("missing local ident private key '{}': {err}", path.display()))?;
    crypto::decode_bytes(value.trim(), "local ident private key")
}

fn read_key_file(path: &Path, what: &str) -> Result<Vec<u8>, String> {
    let value = fs::read_to_string(path)
        .map_err(|err| format!("failed to read {what} '{}': {err}", path.display()))?;
    crypto::decode_bytes(value.trim(), what)
}

/// Pin the registry public key on first contact; refuse a key change after.
/// Returns an error naming the pinned file when the fetched key does not
/// match, so a swapped registry key is loud and requires explicit action.
pub fn pin_server_key(paths: &LocalPaths, server_key: &[u8]) -> Result<(), String> {
    let path = paths.server_key_path();
    if path.is_file() {
        let pinned = read_key_file(&path, "pinned server key")?;
        if pinned != server_key {
            return Err(format!(
                "repository server key does not match the pinned key in '{}'; refusing to continue",
                path.display()
            ));
        }
        return Ok(());
    }
    create_private_dir(&paths.home)?;
    write_private_file(&path, &crypto::encode_bytes(server_key))
}

pub fn read_pinned_server_key(paths: &LocalPaths) -> Result<Vec<u8>, String> {
    read_key_file(&paths.server_key_path(), "pinned server key")
}

pub fn write_session(paths: &LocalPaths, owner: &str, jwt: &str) -> Result<(), String> {
    create_private_dir(&paths.session_dir())?;
    write_private_file(&paths.session_path(owner), jwt)
}

pub fn read_session(paths: &LocalPaths, owner: &str) -> Result<String, String> {
    let path = paths.session_path(owner);
    fs::read_to_string(&path)
        .map(|value| value.trim().to_string())
        .map_err(|err| format!("failed to read session '{}': {err}", path.display()))
}

fn create_private_dir(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path)
        .map_err(|err| format!("failed to create directory '{}': {err}", path.display()))?;
    set_permissions(path, 0o700)
}

fn write_private_file(path: &Path, contents: &str) -> Result<(), String> {
    fs::write(path, contents)
        .map_err(|err| format!("failed to write '{}': {err}", path.display()))?;
    set_permissions(path, 0o600)
}

#[cfg(unix)]
fn set_permissions(path: &Path, mode: u32) -> Result<(), String> {
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .map_err(|err| format!("failed to set permissions on '{}': {err}", path.display()))
}

#[cfg(not(unix))]
fn set_permissions(_path: &Path, _mode: u32) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn writes_and_reads_both_keypairs() {
        let temp = tempfile::tempdir().unwrap();
        let paths = LocalPaths::new(temp.path().join(".mfb"));
        let (auth_public, auth_private) = crypto::generate_keypair();
        let (ident_public, ident_private) = crypto::generate_keypair();
        write_auth_keypair(&paths, "alice", &auth_public, &auth_private).unwrap();
        write_ident_keypair(&paths, "alice", &ident_public, &ident_private).unwrap();

        assert_eq!(read_auth_public_key(&paths, "alice").unwrap(), auth_public);
        assert_eq!(read_auth_private_key(&paths, "alice").unwrap(), auth_private);
        assert_eq!(read_ident_public_key(&paths, "alice").unwrap(), ident_public);
        assert_eq!(read_ident_private_key(&paths, "alice").unwrap(), ident_private);
        #[cfg(unix)]
        {
            assert_eq!(mode(&paths.keys_dir()), 0o700);
            assert_eq!(mode(&paths.auth_private_key_path("alice")), 0o600);
            assert_eq!(mode(&paths.ident_private_key_path("alice")), 0o600);
        }

        remove_owner_keys(&paths, "alice");
        assert!(!paths.auth_private_key_path("alice").exists());
        assert!(!paths.ident_private_key_path("alice").exists());
    }

    #[test]
    fn pins_server_key_once_and_rejects_changes() {
        let temp = tempfile::tempdir().unwrap();
        let paths = LocalPaths::new(temp.path().join(".mfb"));
        let (server_key, _private) = crypto::generate_keypair();
        pin_server_key(&paths, &server_key).unwrap();
        assert_eq!(read_pinned_server_key(&paths).unwrap(), server_key);
        // Same key again is fine.
        pin_server_key(&paths, &server_key).unwrap();
        // A different key must be refused.
        let (other_key, _other_private) = crypto::generate_keypair();
        let err = pin_server_key(&paths, &other_key).unwrap_err();
        assert!(err.contains("does not match the pinned key"), "{err}");
        assert_eq!(read_pinned_server_key(&paths).unwrap(), server_key);
    }

    #[test]
    fn writes_owner_scoped_sessions() {
        let temp = tempfile::tempdir().unwrap();
        let paths = LocalPaths::new(temp.path().join(".mfb"));
        write_session(&paths, "alice", "token-a").unwrap();
        write_session(&paths, "bob", "token-b").unwrap();
        write_session(&paths, "alice", "token-a2").unwrap();

        assert_eq!(read_session(&paths, "alice").unwrap(), "token-a2");
        assert_eq!(read_session(&paths, "bob").unwrap(), "token-b");
        #[cfg(unix)]
        {
            assert_eq!(mode(&paths.session_dir()), 0o700);
            assert_eq!(mode(&paths.session_path("alice")), 0o600);
        }
    }
}
