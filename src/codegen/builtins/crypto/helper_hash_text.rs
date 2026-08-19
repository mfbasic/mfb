//! `__crypto_hashText` — shared private helper for the `crypto` package.
//!
//! The `String` overload of `crypto::hash(type, data)` rewrites to this shim: it
//! UTF-8-encodes `data` and re-enters the `List OF Byte` `hash` `AbiFunction`, so a
//! `String` argument reaches the same per-ordinal SHA dispatch as raw bytes (identical
//! to the legacy `sha*` `_text` cores, which are `strings::toBytes` then the `_bytes`
//! core). It cannot be a second `AbiFunction` overload: an `AbiFunction` member emits a
//! single `crypto.hash` runtime helper whose body is the first (`List OF Byte`) overload,
//! and a `String` pointer read through that `List OF Byte`-shaped body faults.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled source
//! (before the member bodies), in the order `mod.rs` calls the helpers. Body
//! byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __crypto_hashText(algo AS Hash, data AS String) AS List OF Byte
  RETURN crypto::hash(algo, strings::toBytes(data))
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_hashText", BODY));
}
