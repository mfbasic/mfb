pub mod build;
pub mod doc;
pub mod fmt;
pub mod init;
pub mod man;
pub mod pkg;
pub mod repo;
pub mod resolve;
pub mod spec;
pub mod version;

use std::path::{Path, PathBuf};

/// Write an untrusted package blob into `packages_dir` under a fresh, exclusively
/// created name, and return that staging path.
///
/// The blob is attacker-controlled until it has been verified, so it must never
/// land on `packages/<name>.mfp` first: `fs::write` follows symlinks, so a
/// pre-planted link at the destination would be written *through*. `create_new`
/// refuses to open an existing path (symlink or not), and the staged file is
/// promoted only after it verifies.
pub(crate) fn stage_package_blob(
    packages_dir: &Path,
    name: &str,
    blob: &[u8],
) -> Result<PathBuf, String> {
    use std::io::Write;

    crate::manifest::package::validate_package_name(name)?;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    let staged = packages_dir.join(format!(".{name}.mfp.{}.{nanos}.part", std::process::id()));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staged)
        .map_err(|err| format!("failed to create '{}': {err}", staged.display()))?;
    file.write_all(blob)
        .and_then(|()| file.sync_all())
        .map_err(|err| {
            let _ = std::fs::remove_file(&staged);
            format!("failed to write '{}': {err}", staged.display())
        })?;
    Ok(staged)
}

/// Promote a staged package onto its final `packages/<name>.mfp` path. A rename
/// replaces a symlink sitting at the destination rather than writing through it.
pub(crate) fn commit_staged_package(staged: &Path, destination: &Path) -> Result<(), String> {
    std::fs::rename(staged, destination).map_err(|err| {
        let _ = std::fs::remove_file(staged);
        format!("failed to install '{}': {err}", destination.display())
    })
}

/// Stage an untrusted blob, verify it where it lies, and only then install it.
/// Returns the refusal detail when the package does not verify; nothing is left
/// behind in either case.
pub(crate) fn install_verified_package(
    packages_dir: &Path,
    name: &str,
    blob: &[u8],
    ident_key: Option<&str>,
) -> Result<PathBuf, String> {
    let staged = stage_package_blob(packages_dir, name, blob)?;
    let classification = build::classify_installed_package(&staged, ident_key);
    if classification.state != build::PackageVerification::Verified {
        let _ = std::fs::remove_file(&staged);
        return Err(classification
            .refusal
            .map(|(_, detail)| detail)
            .unwrap_or_else(|| "package did not verify".to_string()));
    }
    let destination = packages_dir.join(format!("{name}.mfp"));
    commit_staged_package(&staged, &destination)?;
    Ok(destination)
}

/// Place a downloaded, already hash-verified vendor library file at
/// `dir/filename`, using the same stage-verify-rename discipline as
/// [`install_verified_package`] (plan-48-B §4.4). The bytes arrive from
/// `fetch_blob`, which re-hashes them against the content address, so they are
/// trusted by the time they reach here; staging under an exclusively created
/// `.part` name (bug-27) still matters because `dir/filename` could be a
/// pre-planted symlink, and `fs::write` would follow it.
///
/// `filename` must be a validated bare filename (the caller re-checks the
/// section-10 `source` rule); it is joined onto `dir` without further escaping.
pub(crate) fn install_vendor_file(
    dir: &Path,
    filename: &str,
    bytes: &[u8],
) -> Result<PathBuf, String> {
    use std::io::Write;

    std::fs::create_dir_all(dir)
        .map_err(|err| format!("failed to create '{}': {err}", dir.display()))?;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    let staged = dir.join(format!(".{filename}.{}.{nanos}.part", std::process::id()));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staged)
        .map_err(|err| format!("failed to create '{}': {err}", staged.display()))?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|err| {
            let _ = std::fs::remove_file(&staged);
            format!("failed to write '{}': {err}", staged.display())
        })?;
    let destination = dir.join(filename);
    std::fs::rename(&staged, &destination).map_err(|err| {
        let _ = std::fs::remove_file(&staged);
        format!("failed to install '{}': {err}", destination.display())
    })?;
    Ok(destination)
}

/// Resolve the local key/session store scoped to a specific repository URL.
///
/// Local credentials are keyed only by owner name, so a single owner used
/// against two different repositories would otherwise collide on the same
/// `keys/` and `session/` files. Scoping the store under a hash of the repo
/// URL keeps each repository's keys and sessions isolated:
///   ~/.mfb/<hash>/keys/**
///   ~/.mfb/<hash>/session/**
/// where `<hash>` is the SHA-256 (hex) of the repo URL.
///
/// The base directory mirrors `LocalPaths::from_env` (MFB_HOME, else
/// HOME/.mfb); the repo hash is appended as an additional path component.
pub(crate) fn local_paths_for_repo(
    repo_url: &str,
) -> Result<mfb_repository::local::LocalPaths, String> {
    let base = if let Ok(home) = std::env::var("MFB_HOME") {
        PathBuf::from(home)
    } else {
        let home = std::env::var("HOME").map_err(|_| "HOME is not set".to_string())?;
        PathBuf::from(home).join(".mfb")
    };
    let hash = mfb_repository::crypto::fingerprint(repo_url.as_bytes());
    Ok(mfb_repository::local::LocalPaths::new(base.join(hash)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // `local_paths_for_repo` reads process-global env vars; serialize the
    // env-mutating tests so they cannot race each other.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Set `name` to `value`, returning a guard that restores the prior value
    /// (or removes it) on drop. Centralizing the restore keeps the branchy
    /// save/restore logic in one place.
    struct EnvVarGuard {
        name: &'static str,
        previous: Option<String>,
    }

    impl EnvVarGuard {
        fn set(name: &'static str, value: &str) -> Self {
            let previous = std::env::var(name).ok();
            std::env::set_var(name, value);
            Self { name, previous }
        }

        fn unset(name: &'static str) -> Self {
            let previous = std::env::var(name).ok();
            std::env::remove_var(name);
            Self { name, previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            match &self.previous {
                Some(value) => std::env::set_var(self.name, value),
                None => std::env::remove_var(self.name),
            }
        }
    }

    #[test]
    fn local_paths_for_repo_uses_mfb_home_when_set() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _mfb = EnvVarGuard::set("MFB_HOME", "/tmp/mfb-home-test");
        let paths = local_paths_for_repo("https://registry.example").expect("paths");
        // The store is scoped under MFB_HOME/<repo-hash>.
        let hash = mfb_repository::crypto::fingerprint("https://registry.example".as_bytes());
        assert!(paths
            .keys_dir()
            .starts_with(PathBuf::from("/tmp/mfb-home-test").join(&hash)));
    }

    #[test]
    fn local_paths_for_repo_falls_back_to_home_dot_mfb() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _mfb = EnvVarGuard::unset("MFB_HOME");
        let _home = EnvVarGuard::set("HOME", "/tmp/home-test");
        let paths = local_paths_for_repo("repo").expect("paths");
        let hash = mfb_repository::crypto::fingerprint("repo".as_bytes());
        assert!(paths
            .keys_dir()
            .starts_with(PathBuf::from("/tmp/home-test").join(".mfb").join(&hash)));
    }

    #[test]
    fn local_paths_for_repo_errors_when_home_unset() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _mfb = EnvVarGuard::unset("MFB_HOME");
        let _home = EnvVarGuard::unset("HOME");
        assert!(local_paths_for_repo("repo")
            .unwrap_err()
            .contains("HOME is not set"));
    }

    #[test]
    fn staging_rejects_traversing_names_before_writing_anything() {
        let dir = tempfile::tempdir().expect("temp dir");
        let packages = dir.path().join("packages");
        std::fs::create_dir_all(&packages).expect("packages dir");
        for name in ["../evil", "..", "/etc/passwd", ".hidden", "a/b", "a\\b", ""] {
            assert!(
                stage_package_blob(&packages, name, b"blob").is_err(),
                "name `{name}` must be rejected"
            );
        }
        // Nothing escaped, and nothing was staged.
        assert!(!dir.path().join("evil.mfp").exists());
        assert_eq!(std::fs::read_dir(&packages).expect("read dir").count(), 0);
    }

    #[test]
    fn staging_never_writes_through_a_symlink_at_the_destination() {
        let dir = tempfile::tempdir().expect("temp dir");
        let packages = dir.path().join("packages");
        std::fs::create_dir_all(&packages).expect("packages dir");
        let victim = dir.path().join("victim");
        std::fs::write(&victim, b"original").expect("victim");
        let destination = packages.join("shape.mfp");
        std::os::unix::fs::symlink(&victim, &destination).expect("symlink");

        let staged = stage_package_blob(&packages, "shape", b"attacker").expect("stage");
        assert_ne!(staged, destination);
        assert_eq!(std::fs::read(&victim).expect("victim"), b"original");

        // Committing replaces the symlink itself, never its target.
        commit_staged_package(&staged, &destination).expect("commit");
        assert_eq!(std::fs::read(&victim).expect("victim"), b"original");
        assert_eq!(std::fs::read(&destination).expect("dest"), b"attacker");
        assert!(!destination.is_symlink());
    }

    #[test]
    fn unverified_package_is_never_left_on_the_destination_path() {
        let dir = tempfile::tempdir().expect("temp dir");
        let packages = dir.path().join("packages");
        std::fs::create_dir_all(&packages).expect("packages dir");
        // Not a valid .mfp, so classification cannot reach Verified.
        let error = install_verified_package(&packages, "shape", b"not a package", None)
            .expect_err("garbage must not verify");
        assert!(!error.is_empty());
        // No destination file, and no staging leftovers.
        assert!(!packages.join("shape.mfp").exists());
        assert_eq!(std::fs::read_dir(&packages).expect("read dir").count(), 0);
    }

    #[test]
    fn stage_package_blob_errors_when_the_directory_is_missing() {
        // A valid name, but the packages directory does not exist, so the
        // exclusive `create_new` open fails before anything is written.
        let dir = tempfile::tempdir().expect("temp dir");
        let missing = dir.path().join("no-such-packages-dir");
        let err = stage_package_blob(&missing, "shape", b"blob").expect_err("open must fail");
        assert!(err.contains("failed to create"), "got: {err}");
    }

    #[test]
    fn commit_staged_package_errors_when_the_rename_fails() {
        // The staged file was never created, so the promoting rename fails and the
        // installer surfaces the error (having tried to clean up the phantom stage).
        let dir = tempfile::tempdir().expect("temp dir");
        let staged = dir.path().join("phantom.part");
        let destination = dir.path().join("shape.mfp");
        let err = commit_staged_package(&staged, &destination).expect_err("rename must fail");
        assert!(err.contains("failed to install"), "got: {err}");
        assert!(!destination.exists());
    }

    #[test]
    fn install_verified_package_propagates_a_staging_failure() {
        // A traversing name is rejected while staging; `install_verified_package`
        // returns that failure via `?` rather than proceeding to classify/commit.
        let dir = tempfile::tempdir().expect("temp dir");
        let packages = dir.path().join("packages");
        std::fs::create_dir_all(&packages).expect("packages dir");
        let err = install_verified_package(&packages, "../evil", b"blob", None)
            .expect_err("traversing name must be rejected");
        assert!(!err.is_empty());
        // Nothing was staged or installed.
        assert_eq!(std::fs::read_dir(&packages).expect("read dir").count(), 0);
    }

    #[test]
    fn local_paths_for_repo_scopes_by_repo_url() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _mfb = EnvVarGuard::set("MFB_HOME", "/tmp/mfb-scope-test");
        let a = local_paths_for_repo("https://one.example").expect("a");
        let b = local_paths_for_repo("https://two.example").expect("b");
        // Different repo URLs hash to distinct store directories.
        assert_ne!(a.keys_dir(), b.keys_dir());
    }

    #[test]
    fn install_refuses_a_structurally_valid_but_unsigned_package() {
        // A well-formed but UNSIGNED `.mfp` stages and parses cleanly, so
        // classification reaches `Unsigned` (state != Verified) with no refusal
        // detail — the installer removes the stage and returns the default
        // "did not verify" message rather than committing it.
        let fixture =
            Path::new("tests/syntax/packages/package-trap-builtin/golden/trap_builtin_pkg.mfp");
        let blob = std::fs::read(fixture).expect("unsigned package fixture must exist");
        let dir = tempfile::tempdir().expect("temp dir");
        let packages = dir.path().join("packages");
        std::fs::create_dir_all(&packages).expect("packages dir");
        let err = install_verified_package(&packages, "trap_builtin_pkg", &blob, None)
            .expect_err("an unsigned package must not verify");
        assert!(!err.is_empty());
        // Nothing was installed and no staging leftovers remain.
        assert!(!packages.join("trap_builtin_pkg.mfp").exists());
        assert_eq!(std::fs::read_dir(&packages).expect("read dir").count(), 0);
    }
}
