//! `__compress_deflate` — `compress::deflate`: raw DEFLATE at a validated level.
//!
//! Validates `level` and runs the encoder core ([`super::helper_deflate_core`]).
//!
//! Registered via `add_helper` under [`HelperGate::WhenUsed`] on `deflate` (the zlib and gzip
//! encoders validate their own level and call the core directly). A gated helper
//! is injected as its own file, so the body carries its own `IMPORT`s.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{HelperGate, RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"IMPORT compress
IMPORT bits
IMPORT collections

' compress::deflate(data, level): raw RFC 1951 data.
FUNC __compress_deflate(data AS List OF Byte, level AS Integer) AS List OF Byte
  IF level < 0 OR level > 9 THEN
    FAIL error(77050002, "compress::deflate: level must be from 0 to 9")
  END IF
  LET none AS List OF Byte = []
  RETURN __compress_deflateCore(data, level, none)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper {
        name: "compress_deflate",
        gate: HelperGate::WhenUsed(&["deflate"]),
        body: Some(BODY),
        import_name: None,
        natively_called: false,
    });
}
