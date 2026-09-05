//! `__json_expectLiteralAt` — shared private helper for the `json` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-302: iterative (see __json_skipWhitespace). Bounded by the literal's length
' (`true`/`false`/`null`), so this one could not overflow either; converted for the
' same consistency reason.
FUNC __json_expectLiteralAt(bytes AS List OF Byte, index AS Integer, literal AS List OF Byte, offset AS Integer) AS Integer
  MUT at AS Integer = index
  MUT off AS Integer = offset
  WHILE off < len(literal)
    IF at >= len(bytes) THEN
      FAIL error(77050003, "invalid JSON format")
    END IF
    LET actual AS Integer = toInt(collections::get(bytes, at))
    LET expected AS Integer = toInt(collections::get(literal, off))
    IF actual <> expected THEN
      FAIL error(77050003, "invalid JSON format")
    END IF
    at = at + 1
    off = off + 1
  END WHILE
  RETURN at
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("json_expectLiteralAt", BODY));
}
