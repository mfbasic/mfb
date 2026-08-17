//! `__encoding_baseEncode` — shared private helper for the `encoding` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Generic bit-buffer encoder: emit `bitsPer`-bit symbols from `alphabet`, pad
' the output to a multiple of `groupSymbols` with '=' when `pad`.
FUNC __encoding_baseEncode(data AS List OF Byte, alphabet AS String, bitsPer AS Integer, groupSymbols AS Integer, pad AS Boolean) AS String
  MUT out AS String = ""
  MUT buffer AS Integer = 0
  MUT bitCount AS Integer = 0
  MUT idx AS Integer = 0
  FOR EACH b IN data
    buffer = bits::bor(bits::sl(buffer, 8), toInt(b))
    bitCount = bitCount + 8
    WHILE bitCount >= bitsPer
      bitCount = bitCount - bitsPer
      idx = bits::band(bits::sr(buffer, bitCount), bits::sl(1, bitsPer) - 1)
      out = out & strings::mid(alphabet, idx, 1)
    END WHILE
    buffer = __encoding_lowBits(buffer, bitCount)
  NEXT
  IF bitCount > 0 THEN
    idx = bits::band(bits::sl(buffer, bitsPer - bitCount), bits::sl(1, bitsPer) - 1)
    out = out & strings::mid(alphabet, idx, 1)
  END IF
  IF pad THEN
    WHILE len(out) - (len(out) / groupSymbols) * groupSymbols <> 0
      out = out & "="
    END WHILE
  END IF
  RETURN out
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("encoding_baseEncode", BODY));
}
