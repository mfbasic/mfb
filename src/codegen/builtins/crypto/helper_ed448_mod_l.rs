//! `__crypto_ed448ModL` — shared private helper for the `crypto` package.
//!
//! Reduce a byte-limb integer of up to 114 limbs (a SHAKE256 output or the
//! `r + k·s` sum) modulo the Ed448 group order `L`, returning the canonical
//! 57-byte little-endian scalar (`< L`, top byte 0). Three fixed
//! `__crypto_ed448Fold`s bring the value below `2L`, then one conditional
//! subtraction of `L`, computed as a full borrow chain and applied with a
//! branch-free mask, selects the representative — no control flow depends on the
//! (secret) value.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' x mod L for a byte-limb x of up to 114 limbs: three folds, one masked subtraction.
FUNC __crypto_ed448ModL(x AS List OF Integer) AS List OF Byte
  MUT y AS List OF Integer = __crypto_ed448Fold(__crypto_padLimbs(x, 57))
  y = __crypto_ed448Fold(__crypto_padLimbs(y, 57))
  y = __crypto_ed448Fold(__crypto_padLimbs(y, 57))
  y = __crypto_padLimbs(y, 57)
  MUT t AS List OF Integer = []
  MUT borrow AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < 57
    LET d AS Integer = collections::get(y, i) - collections::get(__CRYPTO_ED448_L, i) - borrow
    borrow = bits::band(bits::sra(d, 8), 1)
    t = collections::append(t, bits::band(d, 255))
    i = i + 1
  END WHILE
  LET keep AS Integer = 0 - borrow
  MUT out AS List OF Byte = []
  i = 0
  WHILE i < 57
    LET ti AS Integer = collections::get(t, i)
    out = collections::append(out, toByte(bits::bxor(ti, bits::band(keep, bits::bxor(ti, collections::get(y, i))))))
    i = i + 1
  END WHILE
  RETURN out
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_ed448ModL", BODY));
}
