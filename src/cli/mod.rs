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
