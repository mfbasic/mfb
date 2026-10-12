//! `__net_byteAt` — shared private helper for the `net` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-979: one byte of a host's UTF-8 as an Integer. The host validators walk
' bytes, not scalars: every character they give a meaning to is ASCII, and a
' byte index is constant-time where `strings::mid` is not.
FUNC __net_byteAt(b AS List OF Byte, i AS Integer) AS Integer
  RETURN toInt(collections::get(b, i))
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("net_byteAt", BODY));
}
