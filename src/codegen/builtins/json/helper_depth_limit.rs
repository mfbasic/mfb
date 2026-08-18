//! `__JSON_DEPTH_LIMIT` — shared private helper for the `json` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-422: ceiling on structural nesting depth. The native stack is exhausted
' somewhere between 800 and 1000 nested array/object frames (measured: ~500 levels
' parse cleanly, ~1000 crash), so 256 fails cleanly with generous margin while
' still admitting any realistic document. Only genuine array/object nesting reaches
' it — the scalar scanners are iterative (bug-302) and unbounded at any length.
LET __JSON_DEPTH_LIMIT AS Integer = 256"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("json_depthLimit", BODY));
}
