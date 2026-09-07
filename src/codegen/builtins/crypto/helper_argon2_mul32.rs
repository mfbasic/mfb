//! `__crypto_argon2Mul32` — shared private helper for the `crypto` package.
//!
//! The low 64 bits of the product of two 32-bit values, over 16-bit limbs so the
//! trapping `*` never overflows. Used by the reference-index mapping (RFC 9106 §3.4.2),
//! whose `J_1^2 / 2^32` needs the high half of a full 64-bit product.
//!
//! Registered via `add_helper` under [`HelperGate::WhenUsed`] on `argon2id`, so a
//! program that imports `crypto` without calling `crypto::argon2id` carries none of
//! the BLAKE2b/Argon2 source. Renders in the helper section of the assembled source.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{HelperGate, RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"IMPORT bits

' The low 64 bits of the product of the low 32 bits of `a` and of `b`, built from
' 16-bit partial products so the trapping `*` never sees an out-of-range operand.
FUNC __crypto_argon2Mul32(a AS Integer, b AS Integer) AS Integer
  LET x AS Integer = bits::band(a, 4294967295)
  LET y AS Integer = bits::band(b, 4294967295)
  LET xl AS Integer = bits::band(x, 65535)
  LET xh AS Integer = bits::sr(x, 16)
  LET yl AS Integer = bits::band(y, 65535)
  LET yh AS Integer = bits::sr(y, 16)
  LET mid AS Integer = xl * yh + xh * yl
  RETURN __crypto_add64(__crypto_add64(xl * yl, bits::sl(mid, 16)), bits::sl(xh * yh, 32))
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper {
        name: "crypto_argon2Mul32",
        gate: HelperGate::WhenUsed(&["argon2id"]),
        body: Some(BODY),
        import_name: None,
        natively_called: false,
    });
}
