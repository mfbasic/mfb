//! `__regex_parseProp` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Parse \p{...} / \pX / \P...; i points just after the p/P. neg=TRUE for \P.
FUNC __regex_parseProp(pat AS List OF String, n AS Integer, i AS Integer, neg AS Boolean) AS __regex_PropParse
  IF i >= n THEN
    FAIL error(77050003, "invalid regex")
  END IF
  IF collections::get(pat, i) = "{" THEN
    MUT j AS Integer = i + 1
    MUT name AS String = ""
    WHILE j < n AND collections::get(pat, j) <> "}"
      name = name & collections::get(pat, j)
      j = j + 1
    END WHILE
    IF j >= n THEN
      FAIL error(77050003, "invalid regex")
    END IF
    LET canon AS String = __regex_canonProp(name)
    IF canon = "" THEN
      FAIL error(77050003, "invalid regex")
    END IF
    RETURN __regex_PropParse[canon, neg, j + 1]
  END IF
  LET canon AS String = __regex_canonProp(collections::get(pat, i))
  IF canon = "" THEN
    FAIL error(77050003, "invalid regex")
  END IF
  RETURN __regex_PropParse[canon, neg, i + 1]
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_parseProp", BODY));
}
