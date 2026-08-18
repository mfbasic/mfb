//! `__encoding_percentDecodeBytes` — shared private helper for the `encoding` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Decode percent escapes (and, when `plusSpace`, '+' as space) into raw bytes,
' then validate UTF-8 and return the decoded String.
FUNC __encoding_percentDecodeBytes(text AS String, plusSpace AS Boolean) AS String
  LET data AS List OF Byte = strings::toBytes(text)
  LET n AS Integer = len(data)
  MUT result AS List OF Byte = []
  MUT i AS Integer = 0
  MUT c AS Integer = 0
  MUT hi AS Integer = 0
  MUT lo AS Integer = 0
  WHILE i < n
    c = toInt(collections::get(data, i))
    IF c = 37 THEN
      IF i + 2 >= n THEN
        FAIL error(77050003, "truncated percent escape")
      END IF
      hi = __encoding_hexValue(toInt(collections::get(data, i + 1)))
      lo = __encoding_hexValue(toInt(collections::get(data, i + 2)))
      IF hi < 0 OR lo < 0 THEN
        FAIL error(77050003, "invalid percent escape")
      END IF
      result = collections::append(result, toByte(hi * 16 + lo))
      i = i + 3
    ELSE
      IF plusSpace AND c = 43 THEN
        result = collections::append(result, toByte(32))
      ELSE
        result = collections::append(result, toByte(c))
      END IF
      i = i + 1
    END IF
  END WHILE
  IF __encoding_utf8Valid(result) = FALSE THEN
    FAIL error(77050003, "invalid utf-8")
  END IF
  RETURN toString(result)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("encoding_percentDecodeBytes", BODY));
}
