//! `__regex_parseCounted` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __regex_parseCounted(pat AS List OF String, n AS Integer, i AS Integer) AS __regex_Count
  MUT j AS Integer = i + 1
  MUT loS AS String = ""
  WHILE j < n AND __regex_isDigit(collections::get(pat, j))
    loS = loS & collections::get(pat, j)
    j = j + 1
  END WHILE
  LET lo AS Integer = __regex_parseIntClamp(loS)
  IF collections::get(pat, j) = "}" THEN
    RETURN __regex_Count[lo, lo, j + 1]
  END IF
  j = j + 1
  IF collections::get(pat, j) = "}" THEN
    RETURN __regex_Count[lo, -1, j + 1]
  END IF
  MUT hiS AS String = ""
  WHILE j < n AND __regex_isDigit(collections::get(pat, j))
    hiS = hiS & collections::get(pat, j)
    j = j + 1
  END WHILE
  LET hi AS Integer = __regex_parseIntClamp(hiS)
  IF lo > hi THEN
    FAIL error(77050003, "invalid regex")
  END IF
  RETURN __regex_Count[lo, hi, j + 1]
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_parseCounted", BODY));
}
