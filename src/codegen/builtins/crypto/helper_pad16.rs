//! `__crypto_pad16` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Right-pad `data` to a multiple of 16 bytes with zeros (AEAD MAC padding).
FUNC __crypto_pad16(data AS List OF Byte) AS List OF Byte
  MUT out AS List OF Byte = __crypto_copyBytes(data)
  WHILE (len(out) MOD 16) <> 0
    out = collections::append(out, toByte(0))
  END WHILE
  RETURN out
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_pad16", BODY));
}
