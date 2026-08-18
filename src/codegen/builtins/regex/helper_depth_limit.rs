//! `__REGEX_DEPTH_LIMIT` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Ceiling on matcher recursion depth. Measured: the native stack is exhausted
' between 800 and 1000 nested frames, so 600 fails cleanly with margin. Only
' constructs that genuinely recurse per repetition reach it -- a greedy repeat
' over a single-scalar child is iterative and unaffected at any input length.
LET __REGEX_DEPTH_LIMIT AS Integer = 600"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_depthLimit", BODY));
}
