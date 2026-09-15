//! `__compress_zlibDecode` — `compress::zlibDecode`: the RFC 1950 zlib wrapper around DEFLATE.
//!
//! Header checks follow zlib 1.2.12 `inflate.c`'s order and refusals (`/tmp/p137ref`, plan-137-B §4.3):
//! `(CMF * 256 + FLG) MOD 31 <> 0` ("incorrect header check"), `CM <> 8` ("unknown compression
//! method"), `CINFO > 7` ("invalid window size"), and a set `FDICT` bit — zlib's `Z_NEED_DICT` — is
//! refused because `compress` offers no dictionary. The 4-byte big-endian Adler-32 after the DEFLATE
//! data must be present; it is compared unless the caller passes `ignoreChecksum := TRUE`, in which case
//! it is not computed at all. Bytes after it are ignored.
//!
//! Registered via `add_helper` under [`HelperGate::WhenUsed`] on `zlibDecode`. A gated helper is
//! injected as its own file, so the body carries its own `IMPORT`s.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{HelperGate, RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"IMPORT compress
IMPORT bits
IMPORT collections

' compress::zlibDecode(data, maxBytes, ignoreChecksum): RFC 1950; bytes after the Adler-32 are ignored.
FUNC __compress_zlibDecode(data AS List OF Byte, maxBytes AS Integer, ignoreChecksum AS Boolean) AS List OF Byte
  IF maxBytes < 0 THEN
    FAIL error(77050002, "compress::zlibDecode: maxBytes must not be negative")
  END IF
  LET n AS Integer = len(data)
  IF n < 2 THEN
    FAIL error(77050003, "compress::zlibDecode: input ends inside the zlib header")
  END IF
  LET cmf AS Integer = toInt(collections::get(data, 0))
  LET flg AS Integer = toInt(collections::get(data, 1))
  IF (cmf * 256 + flg) MOD 31 <> 0 THEN
    FAIL error(77050003, "compress::zlibDecode: incorrect header check")
  END IF
  IF bits::band(cmf, 15) <> 8 THEN
    FAIL error(77050003, "compress::zlibDecode: unknown compression method")
  END IF
  IF bits::sr(cmf, 4) > 7 THEN
    FAIL error(77050003, "compress::zlibDecode: invalid window size")
  END IF
  IF bits::band(flg, 32) <> 0 THEN
    FAIL error(77050003, "compress::zlibDecode: preset dictionary is not supported")
  END IF
  LET withEnd AS List OF Byte = __compress_inflateCore(data, 2, maxBytes)
  LET endPos AS Integer = __compress_endPosition(withEnd)
  IF endPos + 4 > n THEN
    FAIL error(77050003, "compress::zlibDecode: input ends inside the Adler-32 trailer")
  END IF
  LET out AS List OF Byte = __compress_stripEnd(withEnd)
  IF ignoreChecksum = FALSE THEN
    MUT want AS Integer = 0
    MUT k AS Integer = 0
    WHILE k < 4
      want = want * 256 + toInt(collections::get(data, endPos + k))
      k = k + 1
    END WHILE
    IF __compress_adler32(out) <> want THEN
      FAIL error(77050003, "compress::zlibDecode: incorrect data check")
    END IF
  END IF
  RETURN out
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper {
        name: "compress_zlib_frame",
        gate: HelperGate::WhenUsed(&["zlibDecode"]),
        body: Some(BODY),
        import_name: None,
        natively_called: false,
    });
}
