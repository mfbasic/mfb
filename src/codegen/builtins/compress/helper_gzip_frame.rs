//! `__compress_gzipDecode` — `compress::gzipDecode`: every member of an RFC 1952 gzip file.
//!
//! Per member (plan-137-B §4.3): `ID1 = 0x1f`, `ID2 = 0x8b`, `CM = 8`, reserved `FLG` bits 5–7 clear
//! ("unknown header flags set", as zlib 1.2.12); `MTIME`/`XFL`/`OS` skipped; `FEXTRA` (`XLEN` + bytes),
//! `FNAME` and `FCOMMENT` (zero-terminated) walked with bounds checks; `FHCRC`'s two bytes must be
//! present and, unless `ignoreChecksum`, equal the low 16 bits of the CRC-32 of the header bytes before
//! them. The DEFLATE data is decoded with what remains of `maxBytes`, so the limit bounds the total. The
//! `CRC32` and `ISIZE` trailer must be present and, unless `ignoreChecksum`, match. Another member is
//! decoded only while the next two bytes are `1f 8b`; anything else after the last member is ignored,
//! while a `1f 8b` that does not start a valid member is refused (plan-137-A §Decisions).
//!
//! Members after the first are appended byte by byte to the local result, the in-place shape
//! (`.ai/collections.md`); the first member is taken as-is.
//!
//! Registered via `add_helper` under [`HelperGate::WhenUsed`] on `gzipDecode`. A gated helper is
//! injected as its own file, so the body carries its own `IMPORT`s.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{HelperGate, RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"IMPORT compress
IMPORT bits
IMPORT collections

' A little-endian 32-bit field of `data` at `at`.
FUNC __compress_le32(data AS List OF Byte, at AS Integer) AS Integer
  RETURN toInt(collections::get(data, at)) + 256 * toInt(collections::get(data, at + 1)) + 65536 * toInt(collections::get(data, at + 2)) + 16777216 * toInt(collections::get(data, at + 3))
END FUNC

' compress::gzipDecode(data, maxBytes, ignoreChecksum): every member, concatenated.
FUNC __compress_gzipDecode(data AS List OF Byte, maxBytes AS Integer, ignoreChecksum AS Boolean) AS List OF Byte
  IF maxBytes < 0 THEN
    FAIL error(77050002, "compress::gzipDecode: maxBytes must not be negative")
  END IF
  LET n AS Integer = len(data)
  MUT out AS List OF Byte = []
  MUT pos AS Integer = 0
  MUT more AS Boolean = TRUE
  WHILE more
    IF pos + 10 > n THEN
      FAIL error(77050003, "compress::gzipDecode: input ends inside a gzip header")
    END IF
    IF toInt(collections::get(data, pos)) <> 31 OR toInt(collections::get(data, pos + 1)) <> 139 THEN
      FAIL error(77050003, "compress::gzipDecode: not gzip data")
    END IF
    IF toInt(collections::get(data, pos + 2)) <> 8 THEN
      FAIL error(77050003, "compress::gzipDecode: unknown compression method")
    END IF
    LET flg AS Integer = toInt(collections::get(data, pos + 3))
    IF bits::band(flg, 224) <> 0 THEN
      FAIL error(77050003, "compress::gzipDecode: unknown header flags set")
    END IF
    MUT p AS Integer = pos + 10
    IF bits::band(flg, 4) <> 0 THEN
      IF p + 2 > n THEN
        FAIL error(77050003, "compress::gzipDecode: input ends inside a gzip header")
      END IF
      p = p + 2 + toInt(collections::get(data, p)) + 256 * toInt(collections::get(data, p + 1))
      IF p > n THEN
        FAIL error(77050003, "compress::gzipDecode: input ends inside a gzip header")
      END IF
    END IF
    MUT field AS Integer = 8
    WHILE field <= 16
      IF bits::band(flg, field) <> 0 THEN
        WHILE p < n AND toInt(collections::getOr(data, p, toByte(1))) <> 0
          p = p + 1
        END WHILE
        IF p >= n THEN
          FAIL error(77050003, "compress::gzipDecode: input ends inside a gzip header")
        END IF
        p = p + 1
      END IF
      field = field * 2
    END WHILE
    IF bits::band(flg, 2) <> 0 THEN
      IF p + 2 > n THEN
        FAIL error(77050003, "compress::gzipDecode: input ends inside a gzip header")
      END IF
      IF ignoreChecksum = FALSE THEN
        LET headerCrc AS Integer = bits::band(__compress_crc32(collections::mid(data, pos, p - pos), 0), 65535)
        IF headerCrc <> toInt(collections::get(data, p)) + 256 * toInt(collections::get(data, p + 1)) THEN
          FAIL error(77050003, "compress::gzipDecode: header crc mismatch")
        END IF
      END IF
      p = p + 2
    END IF
    LET withEnd AS List OF Byte = __compress_inflateCore(data, p, maxBytes - len(out))
    LET endPos AS Integer = __compress_endPosition(withEnd)
    IF endPos + 8 > n THEN
      FAIL error(77050003, "compress::gzipDecode: input ends inside a gzip trailer")
    END IF
    LET member AS List OF Byte = __compress_stripEnd(withEnd)
    IF ignoreChecksum = FALSE THEN
      IF __compress_crc32(member, 0) <> __compress_le32(data, endPos) THEN
        FAIL error(77050003, "compress::gzipDecode: incorrect data check")
      END IF
      IF len(member) MOD 4294967296 <> __compress_le32(data, endPos + 4) THEN
        FAIL error(77050003, "compress::gzipDecode: incorrect length check")
      END IF
    END IF
    IF len(out) = 0 THEN
      out = member
    ELSE
      MUT k AS Integer = 0
      LET memberLen AS Integer = len(member)
      WHILE k < memberLen
        out = collections::append(out, collections::get(member, k))
        k = k + 1
      END WHILE
    END IF
    pos = endPos + 8
    more = toInt(collections::getOr(data, pos, toByte(0))) = 31 AND toInt(collections::getOr(data, pos + 1, toByte(0))) = 139
  END WHILE
  RETURN out
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper {
        name: "compress_gzip_frame",
        gate: HelperGate::WhenUsed(&["gzipDecode"]),
        body: Some(BODY),
        import_name: None,
        natively_called: false,
    });
}
