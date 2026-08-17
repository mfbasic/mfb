//! `__crypto_aesShiftRows` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' ShiftRows on the column-major AES state (byte i is row i%4, col i/4).
FUNC __crypto_aesShiftRows(state AS List OF Byte) AS List OF Byte
  MUT out AS List OF Byte = __crypto_copyBytes(state)
  MUT r AS Integer = 1
  WHILE r < 4
    MUT c AS Integer = 0
    WHILE c < 4
      LET src AS Integer = ((c + r) MOD 4) * 4 + r
      out = collections::set(out, c * 4 + r, collections::get(state, src))
      c = c + 1
    END WHILE
    r = r + 1
  END WHILE
  RETURN out
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_aesShiftRows", BODY));
}
