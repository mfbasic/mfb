//! `__http_bytesMatchAt` — shared private helper for the `http` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __http_bytesMatchAt(buf AS List OF Byte, pos AS Integer, needle AS List OF Byte) AS Boolean
  LET nn AS Integer = len(needle)
  IF pos < 0 OR pos + nn > len(buf) THEN
    RETURN FALSE
  END IF
  MUT i AS Integer = 0
  WHILE i < nn
    IF collections::get(buf, pos + i) <> collections::get(needle, i) THEN
      RETURN FALSE
    END IF
    i = i + 1
  END WHILE
  RETURN TRUE
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_bytesMatchAt", BODY));
}
