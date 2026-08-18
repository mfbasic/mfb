//! `__regex_parseName` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __regex_parseName(pat AS List OF String, n AS Integer, i AS Integer) AS __regex_Name
  IF i >= n OR __regex_isNameStart(collections::get(pat, i)) = FALSE THEN
    FAIL error(77050003, "invalid regex")
  END IF
  MUT j AS Integer = i
  MUT name AS String = ""
  WHILE j < n AND __regex_isNameCont(collections::get(pat, j))
    name = name & collections::get(pat, j)
    j = j + 1
  END WHILE
  IF j >= n OR collections::get(pat, j) <> ">" THEN
    FAIL error(77050003, "invalid regex")
  END IF
  RETURN __regex_Name[name, j + 1]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_parseName", BODY));
}
