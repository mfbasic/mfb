//! `__compress_endPosition` / `__compress_stripEnd` — read and remove the inflate core's trailer.
//!
//! `__compress_inflateCore` returns its output with the byte position just past the DEFLATE data
//! appended as 8 little-endian bytes (plan-137-B §4.1 shape (b)). The framing helpers read that
//! position to find the zlib or gzip trailer and the next gzip member, then drop the 8 bytes.
//!
//! Registered via `add_helper` under [`HelperGate::WhenUsed`] on the decoders. A gated helper is
//! injected as its own file, so the body carries its own `IMPORT`s.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{HelperGate, RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"IMPORT compress
IMPORT collections

' The byte position the inflate core appended to its output (8 bytes, little-endian).
FUNC __compress_endPosition(withEnd AS List OF Byte) AS Integer
  LET last AS Integer = len(withEnd) - 1
  MUT v AS Integer = 0
  MUT k AS Integer = 0
  WHILE k < 8
    v = v * 256 + toInt(collections::get(withEnd, last - k))
    k = k + 1
  END WHILE
  RETURN v
END FUNC

' The inflate core's output without its 8-byte end-position trailer.
FUNC __compress_stripEnd(withEnd AS List OF Byte) AS List OF Byte
  RETURN collections::mid(withEnd, 0, len(withEnd) - 8)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper {
        name: "compress_end_position",
        gate: HelperGate::WhenUsed(&["inflate", "zlibDecode", "gzipDecode"]),
        body: Some(BODY),
        import_name: None,
        natively_called: false,
    });
}
