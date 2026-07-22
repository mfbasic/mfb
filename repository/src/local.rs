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

    /// `pin_server_key` reads the process-wide `MFB_REPO_SERVER_FINGERPRINT`,
    /// so every test that exercises it must run one at a time. Acquiring the
    /// guard also clears the variable, so a test that panics mid-way cannot
    /// leak a fingerprint into the next one.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn env_guard() -> std::sync::MutexGuard<'static, ()> {
        let guard = ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        std::env::remove_var("MFB_REPO_SERVER_FINGERPRINT");
        guard
    }

    /// A `LocalPaths` whose home is a *regular file*, so every `create_dir_all`
    /// underneath it fails. Returns the `TempDir` so the caller keeps it alive.
    fn home_is_a_file() -> (tempfile::TempDir, LocalPaths) {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join(".mfb");
        fs::write(&home, "this is not a directory").unwrap();
        (temp, LocalPaths::new(home))
    }

    fn fixture() -> (tempfile::TempDir, LocalPaths) {
        let temp = tempfile::tempdir().unwrap();
        let paths = LocalPaths::new(temp.path().join(".mfb"));
        fs::create_dir_all(temp.path().join(".mfb")).unwrap();
        (temp, paths)
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
        let _guard = env_guard();
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

    #[test]
    fn every_writer_reports_the_directory_it_could_not_create() {
        let (_temp, paths) = home_is_a_file();
        let (public, private) = crypto::generate_keypair();
        let cases: Vec<(String, PathBuf)> = vec![
            (
                write_root_pin(&paths, "reg-1", "deadbeef").unwrap_err(),
                paths.root_pin_path(),
            ),
            (
                write_snapshot_version(&paths, 9).unwrap_err(),
                paths.snapshot_version_path(),
            ),
            (
                write_checkpoint(&paths, 9, "roothex").unwrap_err(),
                paths.checkpoint_path(),
            ),
            (
                write_session(&paths, "alice", "token").unwrap_err(),
                paths.session_path("alice"),
            ),
            (
                write_auth_keypair(&paths, "alice", &public, &private).unwrap_err(),
                paths.auth_public_key_path("alice"),
            ),
            (
                write_ident_keypair(&paths, "alice", &public, &private).unwrap_err(),
                paths.ident_public_key_path("alice"),
            ),
            (
                write_pending_ident_keypair(&paths, "alice", &public, &private).unwrap_err(),
                paths.ident_pending_public_key_path("alice"),
            ),
        ];
        for (err, target) in cases {
            // The failure is reported, not swallowed, and it names the
            // directory rather than the file that was going to be written.
            assert!(
                err.starts_with("failed to create directory '"),
                "expected a directory error, got: {err}"
            );
            let parent = target.parent().unwrap().display().to_string();
            assert!(err.contains(&parent), "{err} should name '{parent}'");
            assert!(!target.exists(), "{} must not exist", target.display());
        }
        // Nothing was persisted, and the readers stay quiet rather than erroring.
        assert!(read_root_pin(&paths).unwrap().is_none());
        assert!(read_snapshot_version(&paths).unwrap().is_none());
        assert!(read_checkpoint(&paths).unwrap().is_none());
        assert!(read_pending_ident_public_key(&paths, "alice")
            .unwrap()
            .is_none());
    }

    /// A directory squatting on a key path makes `fs::write` fail, which is the
    /// only way to observe how the two-file keypair writers behave when the
    /// first or the second write fails.
    fn assert_keypair_writer_surfaces_write_failures(
        write: fn(&LocalPaths, &str, &[u8], &[u8]) -> Result<(), String>,
        public_path: fn(&LocalPaths, &str) -> PathBuf,
        private_path: fn(&LocalPaths, &str) -> PathBuf,
    ) {
        let (_temp, paths) = fixture();
        let (public, private) = crypto::generate_keypair();
        fs::create_dir_all(paths.keys_dir()).unwrap();

        // First write fails: the private key must not be written at all.
        fs::create_dir(public_path(&paths, "alice")).unwrap();
        let err = write(&paths, "alice", &public, &private).unwrap_err();
        assert!(err.starts_with("failed to write '"), "{err}");
        assert!(
            err.contains(&public_path(&paths, "alice").display().to_string()),
            "{err}"
        );
        assert!(
            !private_path(&paths, "alice").exists(),
            "the private key must not be written after the public write failed"
        );
        fs::remove_dir(public_path(&paths, "alice")).unwrap();

        // Second write fails: the error names the private key path.
        fs::create_dir(private_path(&paths, "alice")).unwrap();
        let err = write(&paths, "alice", &public, &private).unwrap_err();
        assert!(err.starts_with("failed to write '"), "{err}");
        assert!(
            err.contains(&private_path(&paths, "alice").display().to_string()),
            "{err}"
        );
        // The public half did land, so the caller sees a real error rather
        // than a silently half-written keypair.
        assert_eq!(
            crypto::decode_bytes(
                fs::read_to_string(public_path(&paths, "alice"))
                    .unwrap()
                    .trim(),
                "public",
            )
            .unwrap(),
            public
        );
    }

    #[test]
    fn auth_keypair_writer_surfaces_write_failures() {
        assert_keypair_writer_surfaces_write_failures(
            write_auth_keypair,
            |paths, owner| paths.auth_public_key_path(owner),
            |paths, owner| paths.auth_private_key_path(owner),
        );
    }

    #[test]
    fn ident_keypair_writer_surfaces_write_failures() {
        assert_keypair_writer_surfaces_write_failures(
            write_ident_keypair,
            |paths, owner| paths.ident_public_key_path(owner),
            |paths, owner| paths.ident_private_key_path(owner),
        );
    }

    #[test]
    fn pending_ident_keypair_writer_surfaces_write_failures() {
        assert_keypair_writer_surfaces_write_failures(
            write_pending_ident_keypair,
            |paths, owner| paths.ident_pending_public_key_path(owner),
            |paths, owner| paths.ident_pending_private_key_path(owner),
        );
    }

    #[test]
    fn pinned_state_readers_fail_closed_on_unreadable_files() {
        let (_temp, paths) = fixture();
        // Non-UTF-8 content: the file exists, so these are not "absent" —
        // every reader must report the failure instead of returning `None`,
        // which would silently drop the rollback/pinning defence.
        for path in [
            paths.root_pin_path(),
            paths.snapshot_version_path(),
            paths.checkpoint_path(),
        ] {
            fs::write(&path, [0xff, 0xfe, 0x00, 0x80]).unwrap();
        }
        let err = read_root_pin(&paths).unwrap_err();
        assert!(err.starts_with("failed to read root pin '"), "{err}");
        let err = read_snapshot_version(&paths).unwrap_err();
        assert!(
            err.starts_with("failed to read snapshot version '"),
            "{err}"
        );
        let err = read_checkpoint(&paths).unwrap_err();
        assert!(err.starts_with("failed to read checkpoint '"), "{err}");
    }

    #[test]
    fn root_pin_without_a_separator_is_malformed() {
        let (_temp, paths) = fixture();
        fs::write(paths.root_pin_path(), "registry-id-but-no-fingerprint").unwrap();
        assert_eq!(read_root_pin(&paths).unwrap_err(), "malformed root pin");
        // The value is trimmed before it is split, so a trailing separator is
        // still a pin with no fingerprint at all.
        fs::write(paths.root_pin_path(), "reg-1 \n").unwrap();
        assert_eq!(read_root_pin(&paths).unwrap_err(), "malformed root pin");
        // Only the first separator splits: a fingerprint is never truncated at
        // an embedded space, and surrounding whitespace is stripped.
        fs::write(paths.root_pin_path(), "  reg-1 dead beef\n").unwrap();
        assert_eq!(
            read_root_pin(&paths).unwrap().unwrap(),
            ("reg-1".to_string(), "dead beef".to_string())
        );
    }

    #[test]
    fn pending_ident_rotation_stages_then_promotes() {
        let (_temp, paths) = fixture();
        assert!(read_pending_ident_public_key(&paths, "alice")
            .unwrap()
            .is_none());

        let (live_public, live_private) = crypto::generate_keypair();
        write_ident_keypair(&paths, "alice", &live_public, &live_private).unwrap();
        let (next_public, next_private) = crypto::generate_keypair();
        write_pending_ident_keypair(&paths, "alice", &next_public, &next_private).unwrap();

        // Staging must not disturb the live keypair (bug-276 R1): until the
        // server commits, the old ident is still the account authority.
        assert_eq!(read_ident_public_key(&paths, "alice").unwrap(), live_public);
        assert_eq!(
            read_pending_ident_public_key(&paths, "alice")
                .unwrap()
                .unwrap(),
            next_public
        );
        #[cfg(unix)]
        assert_eq!(
            mode(&paths.ident_pending_private_key_path("alice")),
            0o600,
            "a staged private key must be as private as a live one"
        );

        promote_pending_ident_keypair(&paths, "alice").unwrap();
        assert_eq!(read_ident_public_key(&paths, "alice").unwrap(), next_public);
        assert_eq!(
            read_ident_private_key(&paths, "alice").unwrap(),
            next_private
        );
        // Promotion moves, it does not copy: no staged rotation is left behind.
        assert!(read_pending_ident_public_key(&paths, "alice")
            .unwrap()
            .is_none());
        assert!(!paths.ident_pending_private_key_path("alice").exists());
    }

    #[test]
    fn discarding_a_staged_rotation_keeps_the_live_keypair() {
        let (_temp, paths) = fixture();
        let (live_public, live_private) = crypto::generate_keypair();
        write_ident_keypair(&paths, "alice", &live_public, &live_private).unwrap();
        let (next_public, next_private) = crypto::generate_keypair();
        write_pending_ident_keypair(&paths, "alice", &next_public, &next_private).unwrap();

        remove_pending_ident_keypair(&paths, "alice");
        assert!(read_pending_ident_public_key(&paths, "alice")
            .unwrap()
            .is_none());
        assert!(!paths.ident_pending_private_key_path("alice").exists());
        // The server rejected the rotation, so the live keypair must survive.
        assert_eq!(read_ident_public_key(&paths, "alice").unwrap(), live_public);
        assert_eq!(
            read_ident_private_key(&paths, "alice").unwrap(),
            live_private
        );
        // Discarding again is a no-op, not a failure.
        remove_pending_ident_keypair(&paths, "alice");
        assert!(read_pending_ident_public_key(&paths, "alice")
            .unwrap()
            .is_none());
    }

    #[test]
    fn promoting_without_a_staged_rotation_reports_the_missing_file() {
        let (_temp, paths) = fixture();
        let (live_public, live_private) = crypto::generate_keypair();
        write_ident_keypair(&paths, "alice", &live_public, &live_private).unwrap();

        let err = promote_pending_ident_keypair(&paths, "alice").unwrap_err();
        assert!(err.starts_with("failed to promote '"), "{err}");
        assert!(
            err.contains(
                &paths
                    .ident_pending_public_key_path("alice")
                    .display()
                    .to_string()
            ),
            "{err}"
        );
        assert!(
            err.contains(&paths.ident_public_key_path("alice").display().to_string()),
            "the error should name both ends of the rename: {err}"
        );
        // A failed promotion must leave the live keypair intact.
        assert_eq!(read_ident_public_key(&paths, "alice").unwrap(), live_public);
        assert_eq!(
            read_ident_private_key(&paths, "alice").unwrap(),
            live_private
        );
    }

    #[test]
    fn pin_server_key_refuses_a_corrupt_pin_file() {
        let _guard = env_guard();
        let (_temp, paths) = fixture();
        fs::write(paths.server_key_path(), "not*valid*base64").unwrap();

        let (server_key, _private) = crypto::generate_keypair();
        // A pin file that exists but cannot be decoded must be an error, never
        // a silent re-pin of whatever key the server just offered.
        let err = pin_server_key(&paths, &server_key).unwrap_err();
        assert_eq!(err, "malformed pinned server key");
        assert_eq!(
            fs::read_to_string(paths.server_key_path()).unwrap(),
            "not*valid*base64"
        );
    }

    #[test]
    fn pin_server_key_reports_a_home_that_is_not_a_directory() {
        let _guard = env_guard();
        let (_temp, paths) = home_is_a_file();
        let (server_key, _private) = crypto::generate_keypair();
        let err = pin_server_key(&paths, &server_key).unwrap_err();
        assert!(err.starts_with("failed to create directory '"), "{err}");
        assert!(read_pinned_server_key(&paths).is_err());
    }

    #[test]
    fn pin_server_key_honours_an_out_of_band_fingerprint() {
        let _guard = env_guard();
        let (server_key, _private) = crypto::generate_keypair();
        let fingerprint = crypto::fingerprint(&server_key);

        // A mismatching out-of-band fingerprint must refuse to pin at all
        // (audit-2 SUP-02): first contact with the wrong key is the whole
        // threat this variable exists to stop.
        let (_temp, paths) = fixture();
        std::env::set_var("MFB_REPO_SERVER_FINGERPRINT", "0".repeat(64));
        let err = pin_server_key(&paths, &server_key).unwrap_err();
        assert!(err.contains("does not match the expected"), "{err}");
        assert!(err.contains(&fingerprint), "{err}");
        assert!(
            !paths.server_key_path().exists(),
            "a refused key must not be pinned"
        );

        // A matching fingerprint pins, and the comparison is case-insensitive
        // and whitespace-tolerant so a pasted value works.
        std::env::set_var(
            "MFB_REPO_SERVER_FINGERPRINT",
            format!("  {}  ", fingerprint.to_uppercase()),
        );
        pin_server_key(&paths, &server_key).unwrap();
        assert_eq!(read_pinned_server_key(&paths).unwrap(), server_key);

        // A blank value is treated as "not set" and falls back to TOFU.
        let (_temp2, paths2) = fixture();
        std::env::set_var("MFB_REPO_SERVER_FINGERPRINT", "   ");
        pin_server_key(&paths2, &server_key).unwrap();
        assert_eq!(read_pinned_server_key(&paths2).unwrap(), server_key);

        std::env::remove_var("MFB_REPO_SERVER_FINGERPRINT");
    }
}
