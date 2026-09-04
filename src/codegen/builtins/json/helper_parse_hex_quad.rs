//! `__json_parseHexQuad` — shared private helper for the `json` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __json_parseHexQuad(bytes AS List OF Byte, index AS Integer) AS Integer
  IF index + 3 >= len(bytes) THEN
    FAIL error(77050003, "invalid JSON format")
  END IF
  ' Strict 4-digit `\uXXXX` escape: take exactly four bytes and let toInt(_, 16)
  ' validate the hex digits, keeping json's own error. A hex digit is ASCII, so a
  ' byte >= 128 is malformed before toInt ever sees it -- and rejecting it here is
  ' also what keeps those four bytes a whole, well-formed String.
  LET quad AS List OF Byte = collections::mid(bytes, index, 4)
  FOR EACH unit IN quad
    IF toInt(unit) >= 128 THEN
      FAIL error(77050003, "invalid JSON format")
    END IF
  NEXT
  LET value AS Integer = toInt(toString(quad), 16) TRAP(err)
    FAIL error(77050003, "invalid JSON format")
  END TRAP
  RETURN value
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("json_parseHexQuad", BODY));
}
