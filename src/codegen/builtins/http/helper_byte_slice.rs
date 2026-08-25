//! `__http_byteSlice` — shared private helper for the `http` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __http_byteSlice(buf AS List OF Byte, start AS Integer, stop AS Integer) AS List OF Byte
  MUT out AS List OF Byte = []
  IF stop <= start THEN
    RETURN out
  END IF
  MUT i AS Integer = start
  WHILE i < stop
    out = collections::append(out, collections::get(buf, i))
    i = i + 1
  END WHILE
  RETURN out
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_byteSlice", BODY));
}
