//! `__datetime_peek` — shared private helper for the `datetime` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' A bounded fixed-count slice: `strings::mid(value, start, count)` when the input
' actually has that many characters left, and `""` when it does not (bug-306 S1).
'
' `strings::mid` raises ErrIndexOutOfRange (77050001) past the end, but truncated or
' malformed input is a STRUCTURAL problem and this module documents those as
' ErrInvalidFormat (77050003). Returning `""` lets each caller reach its own
' mismatch path and report the documented code, instead of leaking an
' internal-looking index error to the caller.
FUNC __datetime_peek(value AS String, start AS Integer, count AS Integer) AS String
  IF start < 0 OR count <= 0 THEN
    RETURN ""
  END IF
  IF start + count > len(value) THEN
    RETURN ""
  END IF
  RETURN strings::mid(value, start, count)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("datetime_peek", BODY));
}
