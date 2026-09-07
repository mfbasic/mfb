//! `__crypto_blake2bG` — shared private helper for the `crypto` package.
//!
//! One BLAKE2b mixing step G (RFC 7693 §3.1) over the 16-word working vector.
//! Distinct from Argon2's GB, which adds the 64-bit multiplication and takes no
//! message words — see [`super::helper_argon2_mixadd`].
//!
//! Registered via `add_helper` under [`HelperGate::WhenUsed`] on `argon2id`, so a
//! program that imports `crypto` without calling `crypto::argon2id` carries none of
//! the BLAKE2b/Argon2 source. Renders in the helper section of the assembled source.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{HelperGate, RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"IMPORT bits
IMPORT collections

'  One BLAKE2b mixing step (RFC 7693 section 3.1) over the 16-word working vector.
FUNC __crypto_blake2bG(v AS List OF Integer, a AS Integer, b AS Integer, c AS Integer, d AS Integer, x AS Integer, y AS Integer) AS List OF Integer
  MUT w AS List OF Integer = v
  MUT va AS Integer = collections::get(w, a)
  MUT vb AS Integer = collections::get(w, b)
  MUT vc AS Integer = collections::get(w, c)
  MUT vd AS Integer = collections::get(w, d)
  va = __crypto_add64(__crypto_add64(va, vb), x)
  vd = bits::rr64(bits::bxor(vd, va), 32)
  vc = __crypto_add64(vc, vd)
  vb = bits::rr64(bits::bxor(vb, vc), 24)
  va = __crypto_add64(__crypto_add64(va, vb), y)
  vd = bits::rr64(bits::bxor(vd, va), 16)
  vc = __crypto_add64(vc, vd)
  vb = bits::rr64(bits::bxor(vb, vc), 63)
  w = collections::set(w, a, va)
  w = collections::set(w, b, vb)
  w = collections::set(w, c, vc)
  w = collections::set(w, d, vd)
  RETURN w
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper {
        name: "crypto_blake2bG",
        gate: HelperGate::WhenUsed(&["argon2id"]),
        body: Some(BODY),
        import_name: None,
        natively_called: false,
    });
}
