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

    pub fn public_key_path(&self, owner: &str) -> PathBuf {
        self.keys_dir().join(format!("{owner}.pub"))
    }

    pub fn private_key_path(&self, owner: &str) -> PathBuf {
        self.keys_dir().join(format!("{owner}.prv"))
    }

    pub fn session_path(&self, owner: &str) -> PathBuf {
        self.session_dir().join(format!("{owner}.ses"))
    }
}

pub fn write_keypair(paths: &LocalPaths, owner: &str, public: &[u8], private: &[u8]) -> Result<(), String> {
    create_private_dir(&paths.keys_dir())?;
    let public_path = paths.public_key_path(owner);
    let private_path = paths.private_key_path(owner);
    write_private_file(&public_path, &crypto::encode_bytes(public))?;
    write_private_file(&private_path, &crypto::encode_bytes(private))?;
    Ok(())
}

pub fn remove_keypair(paths: &LocalPaths, owner: &str) {
    let _ = fs::remove_file(paths.public_key_path(owner));
    let _ = fs::remove_file(paths.private_key_path(owner));
}

pub fn read_public_key(paths: &LocalPaths, owner: &str) -> Result<Vec<u8>, String> {
    let path = paths.public_key_path(owner);
    let value = fs::read_to_string(&path)
        .map_err(|err| format!("failed to read local public key '{}': {err}", path.display()))?;
    crypto::decode_bytes(value.trim(), "local public key")
}

pub fn read_private_key(paths: &LocalPaths, owner: &str) -> Result<Vec<u8>, String> {
    let path = paths.private_key_path(owner);
    let value = fs::read_to_string(&path)
        .map_err(|err| format!("missing local private key '{}': {err}", path.display()))?;
    crypto::decode_bytes(value.trim(), "local private key")
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
    fn writes_and_reads_keypair() {
        let temp = tempfile::tempdir().unwrap();
        let paths = LocalPaths::new(temp.path().join(".mfb"));
        let (public, private) = crypto::generate_keypair();
        write_keypair(&paths, "alice", &public, &private).unwrap();

        assert_eq!(read_public_key(&paths, "alice").unwrap(), public);
        assert_eq!(read_private_key(&paths, "alice").unwrap(), private);
        #[cfg(unix)]
        {
            assert_eq!(mode(&paths.keys_dir()), 0o700);
            assert_eq!(mode(&paths.private_key_path("alice")), 0o600);
        }
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
