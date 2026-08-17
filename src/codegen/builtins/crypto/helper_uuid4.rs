//! `__crypto_uuid4` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' A random (version-4) UUID in canonical lowercase 8-4-4-4-12 form (RFC 4122).
FUNC __crypto_uuid4() AS String
  LET rb AS List OF Byte = crypto::randomBytes(16)
  MUT bytes AS List OF Byte = rb
  LET b6 AS Integer = bits::bor(bits::band(toInt(collections::get(rb, 6)), 15), 64)
  LET b8 AS Integer = bits::bor(bits::band(toInt(collections::get(rb, 8)), 63), 128)
  bytes = collections::set(bytes, 6, toByte(b6))
  bytes = collections::set(bytes, 8, toByte(b8))
  LET hex AS String = encoding::hexEncode(bytes)
  LET p1 AS String = strings::mid(hex, 0, 8)
  LET p2 AS String = strings::mid(hex, 8, 4)
  LET p3 AS String = strings::mid(hex, 12, 4)
  LET p4 AS String = strings::mid(hex, 16, 4)
  LET p5 AS String = strings::mid(hex, 20, 12)
  RETURN p1 & "-" & p2 & "-" & p3 & "-" & p4 & "-" & p5
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_uuid4", BODY));
}
