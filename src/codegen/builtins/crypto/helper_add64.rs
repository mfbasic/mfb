//! `__crypto_add64` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' 64-bit modular addition without tripping the trapping `+`: add the 32-bit
' halves, propagate the carry, and recombine as a raw 64-bit bit pattern.
FUNC __crypto_add64(a AS Integer, b AS Integer) AS Integer
  LET aLo AS Integer = bits::band(a, 4294967295)
  LET bLo AS Integer = bits::band(b, 4294967295)
  LET aHi AS Integer = bits::band(bits::sr(a, 32), 4294967295)
  LET bHi AS Integer = bits::band(bits::sr(b, 32), 4294967295)
  LET lo AS Integer = aLo + bLo
  LET carry AS Integer = bits::sr(lo, 32)
  LET loMasked AS Integer = bits::band(lo, 4294967295)
  LET hi AS Integer = aHi + bHi + carry
  LET hiMasked AS Integer = bits::band(hi, 4294967295)
  LET hiShifted AS Integer = bits::sl(hiMasked, 32)
  RETURN bits::bor(hiShifted, loMasked)
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_add64", BODY));
}
