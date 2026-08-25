//! `__http_parseStatusLine` — shared private helper for the `http` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __http_parseStatusLine(line AS String) AS Response
  LET firstSpace AS Integer = __http_indexOf(line, " ", 0)
  IF firstSpace < 0 THEN
    FAIL error(77050003, "malformed status line")
  END IF
  LET versionToken AS String = __http_slice(line, 0, firstSpace)
  IF strings::startsWith(versionToken, "HTTP/") = FALSE THEN
    FAIL error(77050003, "malformed status line")
  END IF
  LET version AS String = strings::stripPrefix(versionToken, "HTTP/")
  LET afterVersion AS String = __http_slice(line, firstSpace + 1, len(line))
  LET secondSpace AS Integer = __http_indexOf(afterVersion, " ", 0)
  MUT statusText AS String = afterVersion
  MUT reason AS String = ""
  IF secondSpace >= 0 THEN
    statusText = __http_slice(afterVersion, 0, secondSpace)
    reason = __http_slice(afterVersion, secondSpace + 1, len(afterVersion))
  END IF
  LET status AS Integer = __http_decToInt(strings::trim(statusText))
  LET ok AS Boolean = status >= 200 AND status <= 299
  LET emptyHeaders AS Map OF String TO String = Map OF String TO String {}
  RETURN Response[status, reason, version, emptyHeaders, strings::toBytes(""), ok]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_parseStatusLine", BODY));
}
