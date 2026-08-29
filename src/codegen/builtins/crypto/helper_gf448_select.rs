//! `__crypto_gf448Select` — shared private helper for the `crypto` package.
//!
//! Constant-time limb select: with `mask` all-ones (`-1`) the result is `b`, with
//! `mask` zero it is `a`, computed as `a XOR (mask AND (a XOR b))` per limb — no
//! branch on the (secret) selector. The Montgomery ladder's conditional swap is
//! two selects.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Branch-free select: `b` where `mask` = -1 (all ones), `a` where `mask` = 0.
FUNC __crypto_gf448Select(a AS List OF Integer, b AS List OF Integer, mask AS Integer) AS List OF Integer
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
    pkg.add_helper(RegistryHelper::always("crypto_gf448Select", BODY));
}
