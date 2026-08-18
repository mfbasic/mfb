//! `__encoding_baseDecodeBits` — shared private helper for the `encoding` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Generic bit-buffer decoder over already-looked-up symbol `values`.
FUNC __encoding_baseDecodeBits(values AS List OF Integer, bitsPer AS Integer) AS List OF Byte
  MUT result AS List OF Byte = []
  MUT buffer AS Integer = 0
  MUT bitCount AS Integer = 0
  MUT byteValue AS Integer = 0
  FOR EACH v IN values
    buffer = bits::bor(bits::sl(buffer, bitsPer), v)
    bitCount = bitCount + bitsPer
    WHILE bitCount >= 8
      bitCount = bitCount - 8
      byteValue = bits::band(bits::sr(buffer, bitCount), 255)
      result = collections::append(result, toByte(byteValue))
    END WHILE
    buffer = __encoding_lowBits(buffer, bitCount)
  NEXT
  RETURN result
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("encoding_baseDecodeBits", BODY));
}
