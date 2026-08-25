//! `__http_emptyRequest` — shared private helper for the `http` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __http_emptyRequest() AS Request
  LET h AS Map OF String TO String = Map OF String TO String {}
  LET q AS Map OF String TO String = Map OF String TO String {}
  LET p AS Map OF String TO String = Map OF String TO String {}
  LET parts AS Map OF String TO RequestPart = Map OF String TO RequestPart {}
  LET b AS List OF Byte = []
  RETURN Request["", "/", "/", h, q, p, parts, b]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_emptyRequest", BODY));
}
