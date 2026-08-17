//! `__encoding_isUnreserved` — shared private helper for the `encoding` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-339 C4: the RFC 3986 unreserved set is the alphanumerics plus "-._~", so
' reuse __encoding_isAlphaNum rather than repeating its three range checks.
FUNC __encoding_isUnreserved(c AS Integer) AS Boolean
  IF __encoding_isAlphaNum(c) THEN
    RETURN TRUE
  END IF
  RETURN c = 45 OR c = 46 OR c = 95 OR c = 126
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("encoding_isUnreserved", BODY));
}
