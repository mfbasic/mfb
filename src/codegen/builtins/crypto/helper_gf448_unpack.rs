//! `__crypto_gf448Unpack` — shared private helper for the `crypto` package.
//!
//! Decode 56 little-endian bytes (RFC 7748 §5 / RFC 8032 §5.2 field encoding)
//! into 16 × 28-bit limbs: each 7-byte group is one 56-bit value split into two
//! limbs. No masking — a non-canonical value `≥ p` is simply reduced by the field
//! arithmetic, as RFC 7748 specifies for `u`.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' 56 little-endian bytes -> 16 limbs of 28 bits (two limbs per 7-byte group).
FUNC __crypto_gf448Unpack(b AS List OF Byte) AS List OF Integer
  MUT o AS List OF Integer = []
  MUT g AS Integer = 0
  WHILE g < 8
    MUT v AS Integer = 0
    MUT i AS Integer = 0
    WHILE i < 7
      v = bits::bor(v, bits::sl(toInt(collections::get(b, g * 7 + i)), i * 8))
      i = i + 1
    END WHILE
    o = collections::append(o, bits::band(v, 268435455))
    o = collections::append(o, bits::sr(v, 28))
    g = g + 1
  END WHILE
  RETURN o
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_gf448Unpack", BODY));
}
