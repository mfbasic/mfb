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

    /// Staging paths for an ident rotation in flight (bug-276 R1). The new
    /// keypair is written here *before* `POST /keys/rotate`, so a rotation the
    /// server commits can never leave the client without the key that is now the
    /// account authority.
    pub fn ident_pending_public_key_path(&self, owner: &str) -> PathBuf {
        self.keys_dir().join(format!("{owner}.ident.next.pub"))
    }

    pub fn ident_pending_private_key_path(&self, owner: &str) -> PathBuf {
        self.keys_dir().join(format!("{owner}.ident.next.prv"))
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

    /// The pinned signed-metadata root (plan-10-C2): `<registry-id> <root-fingerprint>`.
    pub fn root_pin_path(&self) -> PathBuf {
        self.home.join("root-pin")
    }

    /// The highest snapshot/timestamp version seen (plan-10-C2 rollback defense).
    pub fn snapshot_version_path(&self) -> PathBuf {
        self.home.join("snapshot-version")
    }
}

/// Pin the registry id + root fingerprint (plan-10-C2). Written once by
/// `mfb repo trust`; every later metadata fetch is checked against it.
pub fn write_root_pin(
    paths: &LocalPaths,
    registry_id: &str,
    root_fingerprint: &str,
) -> Result<(), String> {
    create_private_dir(&paths.home)?;
    write_private_file(
        &paths.root_pin_path(),
        &format!("{registry_id} {root_fingerprint}"),
    )
}

/// Read the pinned `(registry_id, root_fingerprint)`, if any.
pub fn read_root_pin(paths: &LocalPaths) -> Result<Option<(String, String)>, String> {
    let path = paths.root_pin_path();
    if !path.is_file() {
        return Ok(None);
    }
    let value = fs::read_to_string(&path)
        .map_err(|err| format!("failed to read root pin '{}': {err}", path.display()))?;
    let mut parts = value.trim().splitn(2, ' ');
    let registry_id = parts
        .next()
        .ok_or_else(|| "malformed root pin".to_string())?
        .to_string();
    let root_fingerprint = parts
        .next()
        .ok_or_else(|| "malformed root pin".to_string())?
        .to_string();
    Ok(Some((registry_id, root_fingerprint)))
}

pub fn write_snapshot_version(paths: &LocalPaths, version: i64) -> Result<(), String> {
    create_private_dir(&paths.home)?;
    write_private_file(&paths.snapshot_version_path(), &version.to_string())
}

pub fn read_snapshot_version(paths: &LocalPaths) -> Result<Option<i64>, String> {
    let path = paths.snapshot_version_path();
    if !path.is_file() {
        return Ok(None);
    }
    let value = fs::read_to_string(&path).map_err(|err| {
        format!(
            "failed to read snapshot version '{}': {err}",
            path.display()
        )
    })?;
    // Fail closed on a present-but-unparseable file (bug-276 R9). Returning
    // `Ok(None)` here let `verify_pinned_metadata`'s `.unwrap_or(0)` drop the
    // anti-rollback floor to 0 with no error, so a corrupted pin silently
    // disabled rollback protection instead of reporting it. `read_checkpoint`
    // below already fails closed on the same class of corruption; this matches it.
    value
        .trim()
        .parse::<i64>()
        .map(Some)
        .map_err(|_| format!("malformed pinned snapshot version '{}'", path.display()))
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

pub fn write_auth_keypair(
    paths: &LocalPaths,
    owner: &str,
    public: &[u8],
    private: &[u8],
) -> Result<(), String> {
    create_private_dir(&paths.keys_dir())?;
    write_private_file(
        &paths.auth_public_key_path(owner),
        &crypto::encode_bytes(public),
    )?;
    write_private_file(
        &paths.auth_private_key_path(owner),
        &crypto::encode_bytes(private),
    )?;
    Ok(())
}

pub fn write_ident_keypair(
    paths: &LocalPaths,
    owner: &str,
    public: &[u8],
    private: &[u8],
) -> Result<(), String> {
    create_private_dir(&paths.keys_dir())?;
    write_private_file(
        &paths.ident_public_key_path(owner),
        &crypto::encode_bytes(public),
    )?;
    write_private_file(
        &paths.ident_private_key_path(owner),
        &crypto::encode_bytes(private),
    )?;
    Ok(())
}

/// Stage a rotated ident keypair before the server is asked to commit it
/// (bug-276 R1).
pub fn write_pending_ident_keypair(
    paths: &LocalPaths,
    owner: &str,
    public: &[u8],
    private: &[u8],
) -> Result<(), String> {
    create_private_dir(&paths.keys_dir())?;
    write_private_file(
        &paths.ident_pending_public_key_path(owner),
        &crypto::encode_bytes(public),
    )?;
    write_private_file(
        &paths.ident_pending_private_key_path(owner),
        &crypto::encode_bytes(private),
    )?;
    Ok(())
}

/// Read a staged rotation's public key, if one is present.
pub fn read_pending_ident_public_key(
    paths: &LocalPaths,
    owner: &str,
) -> Result<Option<Vec<u8>>, String> {
    let path = paths.ident_pending_public_key_path(owner);
    if !path.is_file() {
        return Ok(None);
    }
    read_key_file(&path, "pending ident public key").map(Some)
}

/// Promote a staged rotation to the live ident keypair.
pub fn promote_pending_ident_keypair(paths: &LocalPaths, owner: &str) -> Result<(), String> {
    for (from, to) in [
        (
            paths.ident_pending_public_key_path(owner),
            paths.ident_public_key_path(owner),
        ),
        (
            paths.ident_pending_private_key_path(owner),
            paths.ident_private_key_path(owner),
        ),
    ] {
        fs::rename(&from, &to).map_err(|err| {
            format!(
                "failed to promote '{}' to '{}': {err}",
                from.display(),
                to.display()
            )
        })?;
    }
    Ok(())
}

/// Discard a staged rotation the server did not accept.
pub fn remove_pending_ident_keypair(paths: &LocalPaths, owner: &str) {
    let _ = fs::remove_file(paths.ident_pending_public_key_path(owner));
    let _ = fs::remove_file(paths.ident_pending_private_key_path(owner));
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
    read_key_file(
        &paths.ident_public_key_path(owner),
        "local ident public key",
    )
}

pub fn read_ident_private_key(paths: &LocalPaths, owner: &str) -> Result<Vec<u8>, String> {
    let path = paths.ident_private_key_path(owner);
    let value = fs::read_to_string(&path).map_err(|err| {
        format!(
            "missing local ident private key '{}': {err}",
            path.display()
        )
    })?;
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
    // First contact (audit-2 SUP-02 / bug-189): nothing has vouched for this key
    // yet, and it is the root of the plan-23 §3.5 signature chain. If the operator
    // supplied an out-of-band fingerprint via MFB_REPO_SERVER_FINGERPRINT, require
    // it to match before trusting — turning silent trust-on-first-use into
    // verified pinning. Otherwise pin TOFU but make it *visible* (never silent) so
    // the printed fingerprint can be checked out-of-band or with `mfb repo trust`.
    let fingerprint = crypto::fingerprint(server_key);
    match std::env::var("MFB_REPO_SERVER_FINGERPRINT") {
        Ok(expected) if !expected.trim().is_empty() => {
            if !expected.trim().eq_ignore_ascii_case(&fingerprint) {
                return Err(format!(
                    "registry server key fingerprint {fingerprint} does not match the expected \
                     MFB_REPO_SERVER_FINGERPRINT {}; refusing to pin",
                    expected.trim()
                ));
            }
        }
        _ => {
            eprintln!(
                "warning: trusting registry server key on first use (fingerprint {fingerprint}); \
                 verify it out-of-band, or pin it via MFB_REPO_SERVER_FINGERPRINT or `mfb repo trust`."
            );
        }
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
        assert_eq!(
            read_auth_private_key(&paths, "alice").unwrap(),
            auth_private
        );
        assert_eq!(
            read_ident_public_key(&paths, "alice").unwrap(),
            ident_public
        );
        assert_eq!(
            read_ident_private_key(&paths, "alice").unwrap(),
            ident_private
        );
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
    fn from_env_prefers_mfb_home_then_falls_back_to_home() {
        // MFB_HOME wins outright.
        std::env::set_var("MFB_HOME", "/tmp/mfb-home-test");
        let paths = LocalPaths::from_env().unwrap();
        assert_eq!(paths.keys_dir(), PathBuf::from("/tmp/mfb-home-test/keys"));
        std::env::remove_var("MFB_HOME");

        // Without MFB_HOME, HOME/.mfb is used.
        let prior_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", "/tmp/home-test");
        let paths = LocalPaths::from_env().unwrap();
        assert_eq!(paths.keys_dir(), PathBuf::from("/tmp/home-test/.mfb/keys"));

        // With neither, it is an error.
        std::env::remove_var("HOME");
        assert!(LocalPaths::from_env()
            .unwrap_err()
            .contains("HOME is not set"));
        if let Some(home) = prior_home {
            std::env::set_var("HOME", home);
        }
    }

    #[test]
    fn root_pin_round_trips_and_reads_none_when_absent() {
        let temp = tempfile::tempdir().unwrap();
        let paths = LocalPaths::new(temp.path().join(".mfb"));
        assert!(read_root_pin(&paths).unwrap().is_none());
        write_root_pin(&paths, "reg-1", "deadbeef").unwrap();
        assert_eq!(
            read_root_pin(&paths).unwrap().unwrap(),
            ("reg-1".to_string(), "deadbeef".to_string())
        );
        #[cfg(unix)]
        assert_eq!(mode(&paths.root_pin_path()), 0o600);
    }

    #[test]
    fn snapshot_version_round_trips_and_reads_none_when_absent() {
        let temp = tempfile::tempdir().unwrap();
        let paths = LocalPaths::new(temp.path().join(".mfb"));
        assert!(read_snapshot_version(&paths).unwrap().is_none());
        write_snapshot_version(&paths, 42).unwrap();
        assert_eq!(read_snapshot_version(&paths).unwrap(), Some(42));
        // A present-but-unparseable file fails closed (bug-276 R9). It used to
        // return `Ok(None)`, which `verify_pinned_metadata`'s `.unwrap_or(0)`
        // turned into an anti-rollback floor of 0 — silently disabling rollback
        // protection on a corrupt pin. `read_checkpoint` already errors on the
        // same corruption; this now matches it.
        fs::write(paths.snapshot_version_path(), "not-a-number").unwrap();
        assert!(read_snapshot_version(&paths)
            .unwrap_err()
            .contains("malformed pinned snapshot version"));
    }

    #[test]
    fn checkpoint_round_trips_and_rejects_malformed() {
        let temp = tempfile::tempdir().unwrap();
        let paths = LocalPaths::new(temp.path().join(".mfb"));
        assert!(read_checkpoint(&paths).unwrap().is_none());
        write_checkpoint(&paths, 7, "rootrhex").unwrap();
        assert_eq!(
            read_checkpoint(&paths).unwrap().unwrap(),
            (7, "rootrhex".to_string())
        );
        // Malformed contents (missing the size or the root) are errors.
        fs::write(paths.checkpoint_path(), "notanumber roothex").unwrap();
        assert!(read_checkpoint(&paths)
            .unwrap_err()
            .contains("malformed pinned checkpoint"));
        fs::write(paths.checkpoint_path(), "5").unwrap();
        assert!(read_checkpoint(&paths)
            .unwrap_err()
            .contains("malformed pinned checkpoint"));
    }

    #[test]
    fn read_key_errors_when_missing_or_corrupt() {
        let temp = tempfile::tempdir().unwrap();
        let paths = LocalPaths::new(temp.path().join(".mfb"));
        // Nothing written yet.
        assert!(read_auth_public_key(&paths, "alice").is_err());
        assert!(read_auth_private_key(&paths, "alice").is_err());
        assert!(read_ident_public_key(&paths, "alice").is_err());
        assert!(read_ident_private_key(&paths, "alice").is_err());
        assert!(read_pinned_server_key(&paths).is_err());
        assert!(read_session(&paths, "alice").is_err());
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
