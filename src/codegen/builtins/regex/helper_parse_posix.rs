//! `__regex_parsePosix` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' POSIX [:name:] inside a class; idx points at the leading '['.
FUNC __regex_parsePosix(pat AS List OF String, n AS Integer, idx AS Integer) AS __regex_Endpoint
  MUT j AS Integer = idx + 2
  MUT neg AS Boolean = FALSE
  IF j < n AND collections::get(pat, j) = "^" THEN
    neg = TRUE
    j = j + 1
  END IF
  MUT name AS String = ""
  WHILE j < n AND collections::get(pat, j) <> ":"
    name = name & collections::get(pat, j)
    j = j + 1
  END WHILE
  IF j + 1 >= n THEN
    FAIL error(77050003, "invalid regex")
  END IF
  IF collections::get(pat, j) <> ":" OR collections::get(pat, j + 1) <> "]" THEN
    FAIL error(77050003, "invalid regex")
  END IF
  LET prop AS String = __regex_posixProp(name)
  IF prop = "" THEN
    FAIL error(77050003, "invalid regex")
  END IF
  LET item AS __regex_ClassItem = __regex_Prop[prop, neg]
  RETURN __regex_Endpoint[1, "", item, j + 2]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_parsePosix", BODY));
}
