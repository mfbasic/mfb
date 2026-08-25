//! `__http_buildResponse` — shared private helper for the `http` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Parse + dispatch, mapping framing errors to 400/413. 404/500 are handled by
' `dispatch`, so this never fails — the accept loop is crash-proof (§F.5.1).
FUNC __http_buildResponse(raw AS List OF Byte, routes AS List OF Route) AS Response
  MUT parseErr AS Integer = 0
  MUT req AS Request = __http_emptyRequest()
  req = __http_parseRequest(raw) TRAP(e)
    parseErr = e.code
    RECOVER __http_emptyRequest()
  END TRAP
  IF parseErr = errorCode::ErrOverflow THEN
    RETURN __http_status(413, "Payload Too Large")
  END IF
  IF parseErr <> 0 THEN
    RETURN __http_status(400, "Bad Request")
  END IF
  RETURN __http_dispatch(req, routes)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_buildResponse", BODY));
}
