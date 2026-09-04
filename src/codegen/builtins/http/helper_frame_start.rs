//! `__http_frameStart` — shared private helper for the `http` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' The scanner state for a fresh connection: nothing scanned, no head yet.
FUNC __http_frameStart() AS __http_FrameState
  RETURN __http_FrameState[0, FALSE, 0, -1, 0, 0]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_frameStart", BODY));
}
