//! `__crypto_packPoint` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Pack an extended-coordinate point to its 32-byte compressed encoding.
FUNC __crypto_packPoint(p AS List OF Integer) AS List OF Byte
  LET px AS List OF Integer = __crypto_gfAt(p, 0)
  LET py AS List OF Integer = __crypto_gfAt(p, 1)
  LET pz AS List OF Integer = __crypto_gfAt(p, 2)
  LET zi AS List OF Integer = __crypto_inv25519(pz)
  LET tx AS List OF Integer = __crypto_edM(px, zi)
  LET ty AS List OF Integer = __crypto_edM(py, zi)
  MUT r AS List OF Byte = __crypto_pack25519(ty)
  LET parity AS Integer = __crypto_par25519(tx)
  LET r31 AS Integer = bits::bxor(toInt(collections::get(r, 31)), bits::sl(parity, 7))
  r = collections::set(r, 31, toByte(r31))
  RETURN r
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_packPoint", BODY));
}
