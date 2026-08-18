//! `__crypto_constantTimeEqual` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Compare `a` and `b` in time independent of their contents (length is not
' secret). Accumulate the XOR of every byte with no early exit.
FUNC __crypto_constantTimeEqual(a AS List OF Byte, b AS List OF Byte) AS Boolean
  ' bug-269 / CRY-03: fold the length comparison into the accumulated `diff`
  ' instead of returning early on a length mismatch, so the total time does not
  ' branch on length (in)equality. A length difference seeds `diff` non-zero, and
  ' the per-byte compare over the shared prefix stays constant-time (bor/bxor into
  ' `diff`). The remaining loop count (min length) is inherent and standard.
  LET na AS Integer = len(a)
  LET nb AS Integer = len(b)
  MUT diff AS Integer = bits::bxor(na, nb)
  MUT n AS Integer = na
  IF nb < na THEN
    n = nb
  END IF
  MUT i AS Integer = 0
  WHILE i < n
    LET x AS Integer = toInt(collections::get(a, i))
    LET y AS Integer = toInt(collections::get(b, i))
    diff = bits::bor(diff, bits::bxor(x, y))
    i = i + 1
  END WHILE
  RETURN diff = 0
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_constantTimeEqual", BODY));
}
