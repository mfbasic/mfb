//! `__crypto_pbkdf2Sha256_text` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __crypto_pbkdf2Sha256_text(password AS String, salt AS List OF Byte, iterations AS Integer, length AS Integer) AS List OF Byte
  RETURN __crypto_pbkdf2Sha256_bytes(strings::toBytes(password), salt, iterations, length)
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_pbkdf2Sha256_text", BODY));
}
