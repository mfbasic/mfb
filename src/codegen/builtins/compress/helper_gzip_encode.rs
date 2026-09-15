//! `__compress_gzipEncode` — `compress::gzipEncode`: one RFC 1952 member around the encoder core.
//!
//! The ten-byte header is `1f 8b 08`, `FLG = 0` (no name, comment, extra field or header CRC),
//! `MTIME = 0`, an `XFL` that follows zlib 1.2.12's `deflate.c` (`2` at level 9, `4` at levels 0–1,
//! else `0`), and `OS = 255` ("unknown"), so the same input and level give the same bytes on every
//! host and at every time (plan-137-D §1, §2). The header is handed to the core as its output
//! prefix, so the compressed data is never copied; the CRC-32 of `data` and its length modulo 2^32
//! follow, little-endian.
//!
//! Registered via `add_helper` under [`HelperGate::WhenUsed`] on `gzipEncode`. A gated helper is
//! injected as its own file, so the body carries its own `IMPORT`s.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{HelperGate, RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"IMPORT compress
IMPORT bits
IMPORT collections

' compress::gzipEncode(data, level): a ten-byte header, raw RFC 1951 data, CRC-32 and ISIZE little-endian.
FUNC __compress_gzipEncode(data AS List OF Byte, level AS Integer) AS List OF Byte
  IF level < 0 OR level > 9 THEN
    FAIL error(77050002, "compress::gzipEncode: level must be from 0 to 9")
  END IF
  MUT xfl AS Integer = 0
  IF level = 9 THEN
    xfl = 2
  ELSEIF level < 2 THEN
    xfl = 4
  END IF
  LET prefix AS List OF Byte = [toByte(31), toByte(139), toByte(8), toByte(0), toByte(0), toByte(0), toByte(0), toByte(0), toByte(xfl), toByte(255)]
  MUT out AS List OF Byte = __compress_deflateCore(data, level, prefix)
  LET crc AS Integer = __compress_crc32(data, 0)
  LET size AS Integer = bits::band(len(data), 4294967295)
  out = collections::append(out, toByte(bits::band(crc, 255)))
  out = collections::append(out, toByte(bits::band(bits::sr(crc, 8), 255)))
  out = collections::append(out, toByte(bits::band(bits::sr(crc, 16), 255)))
  out = collections::append(out, toByte(bits::band(bits::sr(crc, 24), 255)))
  out = collections::append(out, toByte(bits::band(size, 255)))
  out = collections::append(out, toByte(bits::band(bits::sr(size, 8), 255)))
  out = collections::append(out, toByte(bits::band(bits::sr(size, 16), 255)))
  out = collections::append(out, toByte(bits::band(bits::sr(size, 24), 255)))
  RETURN out
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper {
        name: "compress_gzip_encode",
        gate: HelperGate::WhenUsed(&["gzipEncode"]),
        body: Some(BODY),
        import_name: None,
        natively_called: false,
    });
}
