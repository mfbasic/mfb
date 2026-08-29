//! `__crypto_shake256Text` — shared private helper for the `crypto` package.
//!
//! The `String` overload of `crypto::shake256(data, length)` rewrites to this
//! shim: it UTF-8-encodes `data` and re-enters the `List OF Byte` core — a
//! `String` and a `List OF Byte` are not ABI-interchangeable, so the two overloads
//! rewrite to distinct MFB bodies (the `hash`/`hmac` `_text` pattern).
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __crypto_shake256Text(data AS String, length AS Integer) AS List OF Byte
  RETURN __crypto_shake256(strings::toBytes(data), length)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_shake256Text", BODY));
}
