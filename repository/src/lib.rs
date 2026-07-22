pub mod abi;
pub mod backfill;
pub mod blobstore;
pub mod client;
pub mod crypto;
pub mod gc;
pub mod local;
pub mod log;
pub mod package;
pub mod server;
pub mod store;
pub mod validation;
pub mod web;

/// Registry the client talks to when `MFB_REPO_URL` is unset. This is the
/// public hosted registry, not the local dev server: point at a local
/// `mfb-repo` with `MFB_REPO_URL=http://127.0.0.1:7777` (loopback `http` is
/// exempt from the TLS requirement — see `client::ensure_transport_security`).
pub const DEFAULT_REPO_URL: &str = "https://mfb-repo.fly.dev";
