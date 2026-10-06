//! `__http_statusOfHead` — shared private helper for the `http` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-858: `__http_frameComplete` needs the status code before the body is
' even framed (RFC 9112 §6.3 rules 1-2: 1xx/204/304 are categorically
' bodyless, regardless of any Content-Length/Transfer-Encoding present), but
' it must never RAISE -- `done`'s declared Errors table does not include a
' malformed-status-line code, and a malformed status line already surfaces
' from `finish`'s `__http_parseResponse`/`__http_parseStatusLine` instead.
' So this duplicates `__http_parseStatusLine`'s status-line walk but swallows
' every failure, returning -1 (not a valid status, and not `-1`/`-2` as used
' by `__http_framingLength` for the SAME call so the two sentinels are never
' compared against each other) for anything it cannot parse.
FUNC __http_statusOfHead(headStr AS String) AS Integer
  LET lineEnd AS Integer = __http_indexOf(headStr, "\r\n", 0)
  MUT line AS String = headStr
  IF lineEnd >= 0 THEN
    line = __http_slice(headStr, 0, lineEnd)
  END IF
  LET firstSpace AS Integer = __http_indexOf(line, " ", 0)
  IF firstSpace < 0 THEN
    RETURN -1
  END IF
  LET afterVersion AS String = __http_slice(line, firstSpace + 1, len(line))
  LET secondSpace AS Integer = __http_indexOf(afterVersion, " ", 0)
  MUT statusText AS String = afterVersion
  IF secondSpace >= 0 THEN
    statusText = __http_slice(afterVersion, 0, secondSpace)
  END IF
  LET status AS Integer = __http_decToInt(strings::trim(statusText)) TRAP(e)
    RECOVER -1
  END TRAP
  RETURN status
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_statusOfHead", BODY));
}
