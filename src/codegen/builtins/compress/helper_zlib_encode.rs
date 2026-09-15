//! `__compress_zlibEncode` — `compress::zlibEncode`: RFC 1950 framing around the encoder core.
//!
//! The two-byte header is `CMF = 0x78` (DEFLATE, 32 KiB window) and an `FLG` whose `FLEVEL` bits
//! follow zlib 1.2.12's `deflate.c` `level_flags` (0 for levels 0–1, 1 for 2–5, 2 for 6, 3 for 7–9)
//! and whose `FCHECK` is computed as zlib does (`header += 31 - (header % 31)`); plan-137-D §2
//! pastes both and the header bytes Python and Node write per level. The header is handed to the
//! core as its output prefix, so the compressed data is never copied; the Adler-32 of `data` follows
//! big-endian.
//!
//! Registered via `add_helper` under [`HelperGate::WhenUsed`] on `zlibEncode`. A gated helper is
//! injected as its own file, so the body carries its own `IMPORT`s.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{HelperGate, RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"IMPORT compress
IMPORT bits
IMPORT collections

' compress::zlibEncode(data, level): a two-byte header, raw RFC 1951 data, Adler-32 big-endian.
FUNC __compress_zlibEncode(data AS List OF Byte, level AS Integer) AS List OF Byte
  IF level < 0 OR level > 9 THEN
    FAIL error(77050002, "compress::zlibEncode: level must be from 0 to 9")
  END IF
  MUT levelFlags AS Integer = 3
  IF level < 2 THEN
    levelFlags = 0
  ELSEIF level < 6 THEN
    levelFlags = 1
  ELSEIF level = 6 THEN
    levelFlags = 2
  END IF
  MUT header AS Integer = 30720 + levelFlags * 64
  header = header + 31 - (header MOD 31)
  LET prefix AS List OF Byte = [toByte(header / 256), toByte(header MOD 256)]
  MUT out AS List OF Byte = __compress_deflateCore(data, level, prefix)
  LET adler AS Integer = __compress_adler32(data)
  out = collections::append(out, toByte(bits::band(bits::sr(adler, 24), 255)))
  out = collections::append(out, toByte(bits::band(bits::sr(adler, 16), 255)))
  out = collections::append(out, toByte(bits::band(bits::sr(adler, 8), 255)))
  out = collections::append(out, toByte(bits::band(adler, 255)))
  RETURN out
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper {
        name: "compress_zlib_encode",
        gate: HelperGate::WhenUsed(&["zlibEncode"]),
        body: Some(BODY),
        import_name: None,
        natively_called: false,
    });
}
