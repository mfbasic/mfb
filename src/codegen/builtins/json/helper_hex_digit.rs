//! `__json_hexDigit` — shared private helper for the `json` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-339 D4: index the uppercase hex alphabet (json emits uppercase, unlike
' encoding::__encoding_hexDigit's lowercase) instead of a 16-branch ladder.
' value is a nibble 0..15 at every call site (__json_unicodeControlEscape splits a
' control code point < 32), so mid's own bounds check replaces the old FAIL.
FUNC __json_hexDigit(value AS Integer) AS String
  RETURN strings::mid("0123456789ABCDEF", value, 1)
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("json_hexDigit", BODY));
}
