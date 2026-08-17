//! `__encoding_parseHex` — shared private helper for the `encoding` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __encoding_parseHex(text AS String) AS Integer
  LET data AS List OF Byte = strings::toBytes(text)
  LET n AS Integer = len(data)
  IF n = 0 THEN
    RETURN -1
  END IF
  MUT value AS Integer = 0
  MUT i AS Integer = 0
  MUT d AS Integer = 0
  WHILE i < n
    d = __encoding_hexValue(toInt(collections::get(data, i)))
    IF d < 0 THEN
      RETURN -1
    END IF
    value = value * 16 + d
    ' bug-306 S2: same cap as the decimal parser above.
    IF value > 1114111 THEN
      RETURN -1
    END IF
    i = i + 1
  END WHILE
  RETURN value
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("encoding_parseHex", BODY));
}
