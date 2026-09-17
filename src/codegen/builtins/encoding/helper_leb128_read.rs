//! `__encoding_leb128Read` — shared private helper for the `encoding` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-619: shared unsigned LEB128 read loop, the inverse of __encoding_leb128Emit.
' Returns the raw 64-bit pattern. A tenth byte starts at bit 63, so it holds one
' bit of the pattern: it must terminate the sequence and its payload may not
' exceed tenthMax, or the value does not fit and the call raises. uleb128 passes
' 0 (bit 63 would make the result negative); varint passes 1 (its zigzag pattern
' uses all 64 bits).
FUNC __encoding_leb128Read(data AS List OF Byte, tenthMax AS Integer) AS Integer
  LET n AS Integer = len(data)
  IF n = 0 THEN
    FAIL error(77050003, "truncated leb128")
  END IF
  MUT result AS Integer = 0
  MUT shift AS Integer = 0
  MUT i AS Integer = 0
  MUT byteValue AS Integer = 0
  MUT done AS Boolean = FALSE
  WHILE done = FALSE
    IF i >= n THEN
      FAIL error(77050003, "truncated leb128")
    END IF
    byteValue = toInt(collections::get(data, i))
    IF shift = 63 AND byteValue > tenthMax THEN
      FAIL error(77050003, "leb128 overflow")
    END IF
    result = bits::bor(result, bits::sl(bits::band(byteValue, 127), shift))
    shift = shift + 7
    i = i + 1
    IF byteValue < 128 THEN
      done = TRUE
    END IF
  END WHILE
  RETURN result
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("encoding_leb128Read", BODY));
}
