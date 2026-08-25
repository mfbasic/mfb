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
FUNC __http_decodeBody(status AS Integer, headers AS Map OF String TO String, bodyRaw AS List OF Byte) AS List OF Byte
  IF status = 204 OR status = 304 THEN
    RETURN []
  END IF
  LET transferEncoding AS String = collections::getOr(headers, "transfer-encoding", "")
  IF strings::contains(strings::lower(transferEncoding), "chunked") THEN
    RETURN __http_dechunkBytes(bodyRaw)
  END IF
  RETURN bodyRaw
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_decodeBody", BODY));
}
