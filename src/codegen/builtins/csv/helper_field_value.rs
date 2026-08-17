//! `__csv_fieldValue` — shared private helper for the `csv` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Decode one CSV field. A quoted field's content was collapsed into `buf`
' (doubled quotes folded, delimiters stripped), so it is decoded from the buffer.
' An unquoted field is exactly the contiguous scalar run chars[fieldStart..index),
' so it is decoded straight from the shared scalar buffer with one range decode.
FUNC __csv_fieldValue(chars AS List OF Integer, buf AS List OF Integer, wasQuoted AS Boolean, fieldStart AS Integer, index AS Integer) AS String
  IF wasQuoted THEN
    RETURN encoding::utf32Decode(buf)
  END IF
  RETURN __csv_decodeRange(chars, fieldStart, index)
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("csv_fieldValue", BODY));
}
