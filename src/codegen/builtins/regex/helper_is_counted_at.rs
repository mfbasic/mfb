//! `__regex_isCountedAt` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __regex_isCountedAt(pat AS List OF String, n AS Integer, i AS Integer) AS Boolean
  MUT j AS Integer = i + 1
  IF j >= n OR __regex_isDigit(collections::get(pat, j)) = FALSE THEN
    RETURN FALSE
  END IF
  WHILE j < n AND __regex_isDigit(collections::get(pat, j))
    j = j + 1
  END WHILE
  IF j < n AND collections::get(pat, j) = "}" THEN
    RETURN TRUE
  END IF
  IF j < n AND collections::get(pat, j) = "," THEN
    j = j + 1
    IF j < n AND collections::get(pat, j) = "}" THEN
      RETURN TRUE
    END IF
    IF j < n AND __regex_isDigit(collections::get(pat, j)) THEN
      WHILE j < n AND __regex_isDigit(collections::get(pat, j))
        j = j + 1
      END WHILE
      IF j < n AND collections::get(pat, j) = "}" THEN
        RETURN TRUE
      END IF
    END IF
  END IF
  RETURN FALSE
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_isCountedAt", BODY));
}
