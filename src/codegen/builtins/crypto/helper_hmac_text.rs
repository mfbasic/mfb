//! `__crypto_hmacText` — shared private helper for the `crypto` package.
//!
//! The `String` overload of `crypto::hmac(Hash, key, data)` rewrites to this shim: it
//! UTF-8-encodes `data` and re-enters the `List OF Byte` `hmac` path, so a `String`
//! message reaches the same hash-generic HMAC core as raw bytes (identical to the legacy
//! `hmacSha*` `_text` cores, which are `strings::toBytes` then the `_bytes` core). A
//! `String` and a `List OF Byte` are not ABI-interchangeable, so the two overloads must
//! rewrite to distinct MFB bodies rather than share one bytes-shaped entry point.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __crypto_hmacText(algo AS Hash, key AS List OF Byte, data AS String) AS List OF Byte
  RETURN crypto::hmac(algo, key, strings::toBytes(data))
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_hmacText", BODY));
}
