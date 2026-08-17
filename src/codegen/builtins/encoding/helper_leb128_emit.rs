//! `__encoding_leb128Emit` — shared private helper for the `encoding` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-339 C4: shared LEB128 base-128 emit loop. Takes the seed as a raw 64-bit
' pattern and uses a LOGICAL shift (bits::sr), so it terminates for both a plain
' non-negative value (uleb128) and a zigzag pattern that is negative as a signed
' Integer (varint, |value| >= 2^62). The negativity guard stays in the uleb128
' wrapper — folding it in here would wrongly reject those large varint inputs.
FUNC __encoding_leb128Emit(seed AS Integer) AS List OF Byte
  MUT result AS List OF Byte = []
  MUT remaining AS Integer = seed
  MUT chunk AS Integer = 0
  MUT more AS Boolean = TRUE
  WHILE more
    chunk = bits::band(remaining, 127)
    remaining = bits::sr(remaining, 7)
    IF remaining = 0 THEN
      result = collections::append(result, toByte(chunk))
      more = FALSE
    ELSE
      result = collections::append(result, toByte(chunk + 128))
    END IF
  END WHILE
  RETURN result
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("encoding_leb128Emit", BODY));
}
