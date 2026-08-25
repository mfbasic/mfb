//! `__http_addPart` — shared private helper for the `http` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __http_addPart(parts AS Map OF String TO RequestPart, partRaw AS List OF Byte) AS Map OF String TO RequestPart
  LET he AS Integer = __http_indexOfBytes(partRaw, strings::toBytes("\r\n\r\n"), 0)
  IF he < 0 THEN
    FAIL error(errorCode::ErrInvalidFormat, "malformed multipart part")
  END IF
  LET headStr AS String = __http_bytesToText(__http_byteSlice(partRaw, 0, he))
  LET partBody AS List OF Byte = __http_byteSlice(partRaw, he + 4, len(partRaw))
  LET disposition AS String = __http_partHeader(headStr, "content-disposition")
  LET name AS String = __http_dispositionParam(disposition, "name")
  LET filename AS String = __http_dispositionParam(disposition, "filename")
  LET contentType AS String = __http_partHeader(headStr, "content-type")
  IF name = "" THEN
    RETURN parts
  END IF
  RETURN collections::set(parts, name, RequestPart[filename, contentType, partBody])
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_addPart", BODY));
}
