//! `__crypto_hpkeI2osp2` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' RFC 9180 I2OSP(n, 2): the two-byte big-endian encoding of a length or suite id.
FUNC __crypto_hpkeI2osp2(n AS Integer) AS List OF Byte
  MUT out AS List OF Byte = []
  out = collections::append(out, toByte(bits::band(bits::sr(n, 8), 255)))
  out = collections::append(out, toByte(bits::band(n, 255)))
  RETURN out
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_hpkeI2osp2", BODY));
}
