//! `__encoding_utf8Encode` — shared private helper for the `encoding` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __encoding_utf8Encode(value AS String) AS List OF Byte
  RETURN strings::toBytes(value)
END FUNC

FUNC __encoding_utf8Encode(value AS String) AS List OF Integer
  LET data AS List OF Byte = strings::toBytes(value)
  MUT result AS List OF Integer = []
  FOR EACH b IN data
    result = collections::append(result, toInt(b))
  NEXT
  RETURN result
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("encoding_utf8Encode", BODY));
}
