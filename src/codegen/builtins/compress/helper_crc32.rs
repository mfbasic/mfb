//! `__compress_crc32` — the CRC-32/ISO-HDLC loop behind `compress::crc32`.
//!
//! Slicing-by-8 (plan-137-A §4.2): while at least eight bytes remain, the low four
//! bytes are folded into the register and all eight are looked up in the eight
//! 256-entry tables of `__COMPRESS_CRC32_TABLES` (table `k` at index `k*256`); the
//! last 0–7 bytes take the classic one-byte step through table 0. One function owns
//! the loop and only reads `data`.
//!
//! Registered via `add_helper` under [`HelperGate::WhenUsed`] on `crc32`, so a
//! program that imports `compress` without calling it carries none of this source.
//! A gated helper is injected as its own file, so the body carries its own `IMPORT`s.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{HelperGate, RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"IMPORT compress
IMPORT bits
IMPORT collections

' CRC-32/ISO-HDLC of `data`, continued from `running` (0 starts a new checksum).
FUNC __compress_crc32(data AS List OF Byte, running AS Integer) AS Integer
  IF running < 0 OR running > 4294967295 THEN
    FAIL error(77050002, "compress::crc32: running must be 0..4294967295")
  END IF
  LET n AS Integer = len(data)
  MUT crc AS Integer = bits::bxor(running, 4294967295)
  MUT i AS Integer = 0
  LET stop8 AS Integer = n - 8
  WHILE i <= stop8
    LET x AS Integer = bits::bxor(crc, toInt(collections::get(data, i)) + 256 * toInt(collections::get(data, i + 1)) + 65536 * toInt(collections::get(data, i + 2)) + 16777216 * toInt(collections::get(data, i + 3)))
    LET hi AS Integer = bits::bxor(bits::bxor(collections::get(__COMPRESS_CRC32_TABLES, 1792 + bits::band(x, 255)), collections::get(__COMPRESS_CRC32_TABLES, 1536 + bits::band(bits::sr(x, 8), 255))), bits::bxor(collections::get(__COMPRESS_CRC32_TABLES, 1280 + bits::band(bits::sr(x, 16), 255)), collections::get(__COMPRESS_CRC32_TABLES, 1024 + bits::sr(x, 24))))
    LET lo AS Integer = bits::bxor(bits::bxor(collections::get(__COMPRESS_CRC32_TABLES, 768 + toInt(collections::get(data, i + 4))), collections::get(__COMPRESS_CRC32_TABLES, 512 + toInt(collections::get(data, i + 5)))), bits::bxor(collections::get(__COMPRESS_CRC32_TABLES, 256 + toInt(collections::get(data, i + 6))), collections::get(__COMPRESS_CRC32_TABLES, toInt(collections::get(data, i + 7)))))
    crc = bits::bxor(hi, lo)
    i = i + 8
  END WHILE
  WHILE i < n
    crc = bits::bxor(bits::sr(crc, 8), collections::get(__COMPRESS_CRC32_TABLES, bits::band(bits::bxor(crc, toInt(collections::get(data, i))), 255)))
    i = i + 1
  END WHILE
  RETURN bits::bxor(crc, 4294967295)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper {
        name: "compress_crc32",
        gate: HelperGate::WhenUsed(&["crc32", "gzipDecode"]),
        body: Some(BODY),
        import_name: None,
        natively_called: false,
    });
}
