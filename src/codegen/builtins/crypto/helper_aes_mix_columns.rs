//! `__crypto_aesMixColumns` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __crypto_aesMixColumns(state AS List OF Byte) AS List OF Byte
  MUT out AS List OF Byte = __crypto_copyBytes(state)
  MUT c AS Integer = 0
  WHILE c < 4
    LET b AS Integer = c * 4
    LET s0 AS Integer = toInt(collections::get(state, b))
    LET s1 AS Integer = toInt(collections::get(state, b + 1))
    LET s2 AS Integer = toInt(collections::get(state, b + 2))
    LET s3 AS Integer = toInt(collections::get(state, b + 3))
    LET m0 AS Integer = bits::bxor(bits::bxor(__crypto_gmul8(s0, 2), __crypto_gmul8(s1, 3)), bits::bxor(s2, s3))
    LET m1 AS Integer = bits::bxor(bits::bxor(s0, __crypto_gmul8(s1, 2)), bits::bxor(__crypto_gmul8(s2, 3), s3))
    LET m2 AS Integer = bits::bxor(bits::bxor(s0, s1), bits::bxor(__crypto_gmul8(s2, 2), __crypto_gmul8(s3, 3)))
    LET m3 AS Integer = bits::bxor(bits::bxor(__crypto_gmul8(s0, 3), s1), bits::bxor(s2, __crypto_gmul8(s3, 2)))
    out = collections::set(out, b, toByte(m0))
    out = collections::set(out, b + 1, toByte(m1))
    out = collections::set(out, b + 2, toByte(m2))
    out = collections::set(out, b + 3, toByte(m3))
    c = c + 1
  END WHILE
  RETURN out
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_aesMixColumns", BODY));
}
