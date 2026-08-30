//! `__crypto_ed448Encode` — shared private helper for the `crypto` package.
//!
//! RFC 8032 §5.2.2 point encoding: normalise `(X : Y : Z)` to affine, emit the
//! 56-byte canonical little-endian `y`, then one byte whose top bit is the
//! parity (low bit) of `x`; the other seven bits are zero.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Encode a projective edwards448 point as 57 bytes: y (56, LE) then the x-parity bit.
FUNC __crypto_ed448Encode(p AS List OF Integer) AS List OF Byte
  LET zi AS List OF Integer = __crypto_gf448Inv(__crypto_ed448PointAt(p, 2))
  LET x AS List OF Byte = __crypto_gf448Pack(__crypto_gf448Mul(__crypto_ed448PointAt(p, 0), zi))
  MUT out AS List OF Byte = __crypto_gf448Pack(__crypto_gf448Mul(__crypto_ed448PointAt(p, 1), zi))
  LET parity AS Integer = bits::band(toInt(collections::get(x, 0)), 1)
  out = collections::append(out, toByte(bits::sl(parity, 7)))
  RETURN out
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_ed448Encode", BODY));
}
