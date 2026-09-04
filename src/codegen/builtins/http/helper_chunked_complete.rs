//! `__http_chunkedComplete` — shared private helper for the `http` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Whether a chunked body starting at `bodyStart` is fully present: the terminating
' zero-length chunk has been received at a real chunk boundary. Walks the chunk
' framing (`__http_chunkedScan`, mirroring `__http_dechunkBytes`) instead of
' substring-searching for `0\r\n\r\n`, which can appear INSIDE chunk data and
' stop the read early. FALSE while any size line, or a chunk's data plus its
' trailing CRLF, is still incomplete; TRUE only on reaching `size = 0`. Raises
' on a malformed size line, exactly as the de-chunker later would.
FUNC __http_chunkedComplete(raw AS List OF Byte, bodyStart AS Integer) AS Boolean
  RETURN __http_chunkedScan(raw, bodyStart) = -1
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_chunkedComplete", BODY));
}
