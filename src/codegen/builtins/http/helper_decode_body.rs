//! `__http_decodeBody` — shared private helper for the `http` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-303: chunk framing is byte-length framing. The text version read a hex
' chunk *byte* length and then sliced with `strings::mid`, which indexes by Unicode
' SCALAR -- so any non-ASCII byte desynchronized the offsets and corrupted the body
' or raised "malformed chunk". `__http_dechunkBytes` (already used by the server)
' does the same framing on bytes, where the lengths actually mean what they say.
' bug-858: a Content-Length body was never truncated to the declared length, so
' bytes past it that shared a read with the head leaked into the result. The
' drive loop already waits for `have >= Content-Length` before calling this, so
' `bodyRaw` is at least that long; slice it down to exactly `Content-Length`
' bytes. A malformed/absent Content-Length is left alone here -- `clText = ""`
' means close-delimited (bug-858's other half, handled in frameComplete), and a
' non-numeric value is caught by `__http_framingLength` recovering to 0 (an
' existing, unrelated behavior this change does not touch).
FUNC __http_decodeBody(status AS Integer, headers AS Map OF String TO String, bodyRaw AS List OF Byte) AS List OF Byte
  IF status = 204 OR status = 304 THEN
    RETURN []
  END IF
  LET transferEncoding AS String = collections::getOr(headers, "transfer-encoding", "")
  IF strings::contains(strings::lower(transferEncoding), "chunked") THEN
    RETURN __http_dechunkBytes(bodyRaw)
  END IF
  LET clText AS String = collections::getOr(headers, "content-length", "")
  IF clText <> "" THEN
    LET cl AS Integer = toInt(strings::trim(clText), 10) TRAP(e)
      RECOVER -1
    END TRAP
    IF cl >= 0 AND cl < len(bodyRaw) THEN
      RETURN __http_byteSlice(bodyRaw, 0, cl)
    END IF
  END IF
  RETURN bodyRaw
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_decodeBody", BODY));
}
