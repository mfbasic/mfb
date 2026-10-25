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
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .map_err(|_| "HOME is not set".to_string())?;
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

    /// The highest `root.json` version seen (bug-584 rollback defense). Root
    /// renewal rotates the delegated online keys, so replaying a still-unexpired
    /// OLDER root would resurrect the keys a renewal retired.
    pub fn root_version_path(&self) -> PathBuf {
        self.home.join("root-version")
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
    // The first separator splits; the fingerprint keeps any later spaces.
    let (registry_id, root_fingerprint) = value
        .trim()
        .split_once(' ')
        .ok_or_else(|| "malformed root pin".to_string())?;
    Ok(Some((
        registry_id.to_string(),
        root_fingerprint.to_string(),
    )))
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

/// Pin the highest `root.json` version seen (bug-584). Advanced by every
/// verified chain; a lower root version is a rollback.
pub fn write_root_version(paths: &LocalPaths, version: i64) -> Result<(), String> {
    create_private_dir(&paths.home)?;
    write_private_file(&paths.root_version_path(), &version.to_string())
}

/// Read the pinned root version, if any. Fails CLOSED on a present-but-
/// unparseable file, exactly as `read_snapshot_version` does: returning
/// `Ok(None)` would drop the anti-rollback floor to 0 with no error.
pub fn read_root_version(paths: &LocalPaths) -> Result<Option<i64>, String> {
    let path = paths.root_version_path();
    if !path.is_file() {
        return Ok(None);
    }
    let value = fs::read_to_string(&path)
        .map_err(|err| format!("failed to read root version '{}': {err}", path.display()))?;
    value
        .trim()
        .parse::<i64>()
        .map(Some)
        .map_err(|_| format!("malformed pinned root version '{}'", path.display()))
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

/// Read a staged rotation's private key, if one is present. The pending
/// private key is the staging marker (bug-793): it is written last and promoted
/// last, so whenever it exists the staged key may be the account authority and
/// must not be overwritten or discarded without asking the registry.
pub fn read_pending_ident_private_key(
    paths: &LocalPaths,
    owner: &str,
) -> Result<Option<Vec<u8>>, String> {
    let path = paths.ident_pending_private_key_path(owner);
    if !path.is_file() {
        return Ok(None);
    }
    read_key_file(&path, "pending ident private key").map(Some)
}

/// Promote a staged rotation to the live ident keypair.
///
/// The live public key is rewritten from the staged private key, and the staged
/// private key moves last. A promotion interrupted part-way therefore leaves the
/// staging marker in place, and re-running it finishes the job (bug-793).
pub fn promote_pending_ident_keypair(paths: &LocalPaths, owner: &str) -> Result<(), String> {
    let from = paths.ident_pending_private_key_path(owner);
    let to = paths.ident_private_key_path(owner);
    let staged_private = read_pending_ident_private_key(paths, owner)?.ok_or_else(|| {
        format!(
            "failed to promote '{}' to '{}': no staged ident key",
            from.display(),
            to.display()
        )
    })?;
    let staged_public = crypto::public_from_private(&staged_private)?;
    write_private_file(
        &paths.ident_public_key_path(owner),
        &crypto::encode_bytes(&staged_public),
    )?;
    let _ = fs::remove_file(paths.ident_pending_public_key_path(owner));
    fs::rename(&from, &to).map_err(|err| {
        format!(
            "failed to promote '{}' to '{}': {err}",
            from.display(),
            to.display()
        )
    })
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
pub(crate) mod tests;
