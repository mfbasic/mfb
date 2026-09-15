//! `__compress_inflate` — `compress::inflate`: raw DEFLATE, trailer removed.
//!
//! Validates `maxBytes`, runs the core from byte 0, and drops the core's 8-byte end-position
//! trailer. Bytes after the final block are ignored (plan-137-A §Decisions).
//!
//! Registered via `add_helper` under [`HelperGate::WhenUsed`] on the decoders. A gated helper
//! is injected as its own file, so the body carries its own `IMPORT`s.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{HelperGate, RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"IMPORT compress
IMPORT bits
IMPORT collections

' compress::inflate(data, maxBytes): raw RFC 1951 data; bytes after the final block are ignored.
FUNC __compress_inflate(data AS List OF Byte, maxBytes AS Integer) AS List OF Byte
  IF maxBytes < 0 THEN
    FAIL error(77050002, "compress::inflate: maxBytes must not be negative")
  END IF
  LET withEnd AS List OF Byte = __compress_inflateCore(data, 0, maxBytes)
  RETURN collections::mid(withEnd, 0, len(withEnd) - 8)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper {
        name: "compress_inflate",
        gate: HelperGate::WhenUsed(&["inflate", "zlibDecode", "gzipDecode"]),
        body: Some(BODY),
        import_name: None,
        natively_called: false,
    });
}
