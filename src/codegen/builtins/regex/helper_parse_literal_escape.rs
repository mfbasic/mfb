//! `__regex_parseLiteralEscape` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' A literal/control escape; i points at the backslash. allowSpace permits
' "\ " (x mode). Rejects backreferences and unknown letter/digit escapes.
FUNC __regex_parseLiteralEscape(pat AS List OF String, n AS Integer, i AS Integer, allowSpace AS Boolean) AS __regex_LitScalar
  IF i + 1 >= n THEN
    FAIL error(77050003, "invalid regex")
  END IF
  LET e AS String = collections::get(pat, i + 1)
  IF e = "n" THEN
    RETURN __regex_LitScalar["\n", i + 2]
  END IF
  IF e = "r" THEN
    RETURN __regex_LitScalar["\r", i + 2]
  END IF
  IF e = "t" THEN
    RETURN __regex_LitScalar["\t", i + 2]
  END IF
  IF e = "f" THEN
    RETURN __regex_LitScalar[__regex_chr(12), i + 2]
  END IF
  IF e = "v" THEN
    RETURN __regex_LitScalar[__regex_chr(11), i + 2]
  END IF
  IF e = "a" THEN
    RETURN __regex_LitScalar[__regex_chr(7), i + 2]
  END IF
  IF e = "e" THEN
    RETURN __regex_LitScalar[__regex_chr(27), i + 2]
  END IF
  IF e = "0" THEN
    RETURN __regex_LitScalar[__regex_chr(0), i + 2]
  END IF
  IF e = "x" THEN
    RETURN __regex_parseHexEscape(pat, n, i + 2)
  END IF
  IF __regex_isAsciiPunct(e) THEN
    RETURN __regex_LitScalar[e, i + 2]
  END IF
  IF allowSpace AND e = " " THEN
    RETURN __regex_LitScalar[" ", i + 2]
  END IF
  FAIL error(77050003, "invalid regex")
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_parseLiteralEscape", BODY));
}
