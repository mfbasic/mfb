//! `__http_indexOfBytes` — shared private helper for the `http` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __http_indexOfBytes(buf AS List OF Byte, needle AS List OF Byte, start AS Integer) AS Integer
  LET nn AS Integer = len(needle)
  IF nn = 0 THEN
    RETURN start
  END IF
  MUT i AS Integer = start
  LET limit AS Integer = len(buf) - nn
  WHILE i <= limit
    IF __http_bytesMatchAt(buf, i, needle) THEN
      RETURN i
    END IF
    i = i + 1
  END WHILE
  RETURN -1
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_indexOfBytes", BODY));
}
