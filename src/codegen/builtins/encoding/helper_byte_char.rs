//! `__encoding_byteChar` — shared private helper for the `encoding` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' The single ASCII/Latin-1 byte `c` as a one-character String (a typed binding
' so the list literal's element type resolves to Byte).
FUNC __encoding_byteChar(c AS Integer) AS String
  LET one AS List OF Byte = [toByte(c)]
  RETURN toString(one)
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("encoding_byteChar", BODY));
}
