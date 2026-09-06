//! `__crypto_sel25519` — shared private helper for the `crypto` package.
//!
//! Constant-time limb select over the 16-limb GF(2^255-19) representation: with
//! `mask` all-ones (`-1`) the result is `b`, with `mask` zero it is `a`, computed
//! as `a XOR (mask AND (a XOR b))` per limb — no branch on the (secret) selector.
//! A conditional swap is two selects; a conditional reduction is one. This is
//! TweetNaCl's `sel25519` in the package's own idiom, and the 255-lane twin of
//! `__crypto_gf448Select` (bug-511). The two are kept separate rather than shared
//! because the fields differ — 16 × 16-bit limbs here, 16 × 28-bit limbs there —
//! the same split the package already makes between `__crypto_cswap128` and
//! `__crypto_ed448Cswap`.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Branch-free select: `b` where `mask` = -1 (all ones), `a` where `mask` = 0.
FUNC __crypto_sel25519(a AS List OF Integer, b AS List OF Integer, mask AS Integer) AS List OF Integer
  MUT o AS List OF Integer = []
  MUT i AS Integer = 0
  WHILE i < 16
    LET ai AS Integer = collections::get(a, i)
    o = collections::append(o, bits::bxor(ai, bits::band(mask, bits::bxor(ai, collections::get(b, i)))))
    i = i + 1
  END WHILE
  RETURN o
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_sel25519", BODY));
}
