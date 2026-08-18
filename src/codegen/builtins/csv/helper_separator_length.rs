//! `__csv_separatorLength` — shared private helper for the `csv` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Length, in scalars, of a record separator starting at index, or 0 if the scalar
' there is not a separator. A bare LF is one step; a CR (13) followed by an LF
' (10) is two steps; a lone CR is data (0).
FUNC __csv_separatorLength(chars AS List OF Integer, count AS Integer, index AS Integer) AS Integer
  LET ch AS Integer = collections::get(chars, index)
  IF ch = 10 THEN
    RETURN 1
  END IF
  IF ch = 13 THEN
    IF index + 1 < count THEN
      IF collections::get(chars, index + 1) = 10 THEN
        RETURN 2
      END IF
    END IF
  END IF
  RETURN 0
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("csv_separatorLength", BODY));
}
