pub mod build;
pub mod doc;
pub mod fmt;
pub mod init;
pub mod man;
pub mod pkg;
pub mod repo;
pub mod resolve;
pub mod spec;

use std::path::PathBuf;

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
    fn local_paths_for_repo_scopes_by_repo_url() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _mfb = EnvVarGuard::set("MFB_HOME", "/tmp/mfb-scope-test");
        let a = local_paths_for_repo("https://one.example").expect("a");
        let b = local_paths_for_repo("https://two.example").expect("b");
        // Different repo URLs hash to distinct store directories.
        assert_ne!(a.keys_dir(), b.keys_dir());
    }
}
