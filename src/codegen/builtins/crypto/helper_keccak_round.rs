//! `__crypto_keccakRound` — shared private helper for the `crypto` package.
//!
//! One Keccak-f[1600] round (FIPS 202 §3.3): theta, rho and pi (one table-driven
//! pass), chi, and iota, over the 25-lane state indexed `x + 5y`. Each lane is
//! one full 64-bit `Integer` bit pattern manipulated only through `bits::`
//! (XOR/AND/NOT/`rl64`), so no value ever enters trapping arithmetic. The body is
//! branch-free: every loop bound is a constant and every index is a loop counter
//! or a public table entry — nothing depends on the state's contents.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' One Keccak-f[1600] round: theta, rho+pi, chi, iota with round constant `rc`.
FUNC __crypto_keccakRound(a AS List OF Integer, rc AS Integer) AS List OF Integer
  MUT c AS List OF Integer = []
  MUT x AS Integer = 0
  WHILE x < 5
    LET c01 AS Integer = bits::bxor(collections::get(a, x), collections::get(a, x + 5))
    LET c23 AS Integer = bits::bxor(collections::get(a, x + 10), collections::get(a, x + 15))
    c = collections::append(c, bits::bxor(bits::bxor(c01, c23), collections::get(a, x + 20)))
    x = x + 1
  END WHILE
  MUT t AS List OF Integer = []
  MUT i AS Integer = 0
  WHILE i < 25
    LET xi AS Integer = i MOD 5
    LET d AS Integer = bits::bxor(collections::get(c, (xi + 4) MOD 5), bits::rl64(collections::get(c, (xi + 1) MOD 5), 1))
    t = collections::append(t, bits::bxor(collections::get(a, i), d))
    i = i + 1
  END WHILE
  MUT b AS List OF Integer = __crypto_keccakZero()
  i = 0
  WHILE i < 25
    LET rotated AS Integer = bits::rl64(collections::get(t, i), collections::get(__CRYPTO_KECCAK_RHO, i))
    b = collections::set(b, collections::get(__CRYPTO_KECCAK_PI, i), rotated)
    i = i + 1
  END WHILE
  MUT out AS List OF Integer = []
  i = 0
  WHILE i < 25
    LET row AS Integer = i - (i MOD 5)
    LET x1 AS Integer = row + (((i MOD 5) + 1) MOD 5)
    LET x2 AS Integer = row + (((i MOD 5) + 2) MOD 5)
    LET andNot AS Integer = bits::band(bits::bnot(collections::get(b, x1)), collections::get(b, x2))
    out = collections::append(out, bits::bxor(collections::get(b, i), andNot))
    i = i + 1
  END WHILE
  RETURN collections::set(out, 0, bits::bxor(collections::get(out, 0), rc))
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_keccakRound", BODY));
}
