//! `__compress_crc32` — the CRC-32/ISO-HDLC loop behind `compress::crc32`.
//!
//! Slicing-by-8 (plan-137-A §4.2): while at least eight bytes remain, the low four
//! bytes are folded into the register and all eight are looked up in the eight
//! 256-entry tables of `__COMPRESS_CRC32_TABLES` (table `k` at index `k*256`); the
//! last 0–7 bytes take the classic one-byte step through table 0. `__compress_crc32`
//! validates `running` and hands the table list to the loop, `__compress_crc32Run`, as a
//! parameter, so the loop reads a local rather than the module global on every lookup;
//! the loop only reads `data` and the table (bug-774).
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
  RETURN bits::bxor(__compress_crc32Run(data, __COMPRESS_CRC32_TABLES, bits::bxor(running, 4294967295)), 4294967295)
END FUNC

' The slicing-by-8 loop over the pre-inverted register `crc`. `t` is the table list, passed in
' so the loop reads a parameter rather than the module global on every lookup. Every index is
' built with `bits::bor`, not `+`: `i` is a multiple of 8 inside the 8-byte loop, so `i + k` for
' k < 8 is `bor(i, k)`; table `k` starts at a multiple of 256 and each byte index is below 256;
' `crc` stays below 2^32, so `sr(x, 24)` is below 256. Same values, and no overflow check can fire.
FUNC __compress_crc32Run(data AS List OF Byte, t AS List OF Integer, start AS Integer) AS Integer
  LET n AS Integer = len(data)
  MUT crc AS Integer = start
  MUT i AS Integer = 0
  LET stop8 AS Integer = n - 8
  WHILE i <= stop8
    LET x AS Integer = bits::bxor(crc, bits::bor(bits::bor(toInt(collections::get(data, i)), bits::sl(toInt(collections::get(data, bits::bor(i, 1))), 8)), bits::bor(bits::sl(toInt(collections::get(data, bits::bor(i, 2))), 16), bits::sl(toInt(collections::get(data, bits::bor(i, 3))), 24))))
    LET hi AS Integer = bits::bxor(bits::bxor(collections::get(t, bits::bor(1792, bits::band(x, 255))), collections::get(t, bits::bor(1536, bits::band(bits::sr(x, 8), 255)))), bits::bxor(collections::get(t, bits::bor(1280, bits::band(bits::sr(x, 16), 255))), collections::get(t, bits::bor(1024, bits::sr(x, 24)))))
    LET lo AS Integer = bits::bxor(bits::bxor(collections::get(t, bits::bor(768, toInt(collections::get(data, bits::bor(i, 4))))), collections::get(t, bits::bor(512, toInt(collections::get(data, bits::bor(i, 5)))))), bits::bxor(collections::get(t, bits::bor(256, toInt(collections::get(data, bits::bor(i, 6))))), collections::get(t, toInt(collections::get(data, bits::bor(i, 7))))))
    crc = bits::bxor(hi, lo)
    i = i + 8
  END WHILE
  WHILE i < n
    crc = bits::bxor(bits::sr(crc, 8), collections::get(t, bits::band(bits::bxor(crc, toInt(collections::get(data, i))), 255)))
    i = i + 1
  END WHILE
  RETURN crc
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper {
        name: "compress_crc32",
        gate: HelperGate::WhenUsed(&["crc32", "gzipDecode", "gzipEncode"]),
        body: Some(BODY),
        import_name: None,
        natively_called: false,
    });
}
