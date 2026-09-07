//! `__crypto_argon2MixAdd` — shared private helper for the `crypto` package.
//!
//! Argon2's fused mixing addition `(a + b + 2 * trunc(a) * trunc(b)) mod 2^64`
//! (RFC 9106 figure 19) — the member's hot path, called 512 times per 1 KiB block.
//!
//! Registered via `add_helper` under [`HelperGate::WhenUsed`] on `argon2id`, so a
//! program that imports `crypto` without calling `crypto::argon2id` carries none of
//! the BLAKE2b/Argon2 source. Renders in the helper section of the assembled source.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{HelperGate, RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"IMPORT bits

' Argon2's fused mixing addition: (a + b + 2 * trunc32(a) * trunc32(b)) mod 2^64,
' accumulated over 16- and 32-bit limbs so the trapping `+` and `*` never see an
' out-of-range operand. This is the inner step of the permutation and the hot path
' of the whole member.
FUNC __crypto_argon2MixAdd(a AS Integer, b AS Integer) AS Integer
  LET aLo AS Integer = bits::band(a, 4294967295)
  LET bLo AS Integer = bits::band(b, 4294967295)
  LET xl AS Integer = bits::band(aLo, 65535)
  LET xh AS Integer = bits::sr(aLo, 16)
  LET yl AS Integer = bits::band(bLo, 65535)
  LET yh AS Integer = bits::sr(bLo, 16)
  LET t0 AS Integer = bits::sl(xl * yl, 1)
  LET t1 AS Integer = bits::sl(xl * yh + xh * yl, 1)
  LET t2 AS Integer = bits::sl(xh * yh, 1)
  LET low AS Integer = aLo + bLo + t0 + bits::sl(bits::band(t1, 65535), 16)
  LET high AS Integer = bits::sr(a, 32) + bits::sr(b, 32) + bits::sr(t1, 16) + t2 + bits::sr(low, 32)
  RETURN bits::bor(bits::sl(bits::band(high, 4294967295), 32), bits::band(low, 4294967295))
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper {
        name: "crypto_argon2MixAdd",
        gate: HelperGate::WhenUsed(&["argon2id"]),
        body: Some(BODY),
        import_name: None,
        natively_called: false,
    });
}
