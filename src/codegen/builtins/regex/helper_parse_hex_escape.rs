//! `__regex_parseHexEscape` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Parse \xHH or \x{H..H}; i points just after the 'x'.
FUNC __regex_parseHexEscape(pat AS List OF String, n AS Integer, i AS Integer) AS __regex_LitScalar
  IF i >= n THEN
    FAIL error(77050003, "invalid regex")
  END IF
  IF collections::get(pat, i) = "{" THEN
    ' Collect the run between `{` and `}`, then let toInt(_, 16) validate the
    ' hex digits (it FAILs on any non-hex char). The 1-6 digit bound and the
    ' surrogate/range clamp stay regex's own checks (regex.md §8.2).
    MUT j AS Integer = i + 1
    MUT hexText AS String = ""
    WHILE j < n AND collections::get(pat, j) <> "}"
      hexText = hexText & collections::get(pat, j)
      j = j + 1
    END WHILE
    IF j >= n THEN
      FAIL error(77050003, "invalid regex")
    END IF
    LET digits AS Integer = len(hexText)
    IF digits < 1 OR digits > 6 THEN
      FAIL error(77050003, "invalid regex")
    END IF
    LET value AS Integer = toInt(hexText, 16) TRAP(err)
      FAIL error(77050003, "invalid regex")
    END TRAP
    IF value > 1114111 OR (value >= 55296 AND value <= 57343) THEN
      FAIL error(77050003, "invalid regex")
    END IF
    RETURN __regex_LitScalar[__regex_chr(value), j + 1]
  END IF
  IF i + 1 >= n THEN
    FAIL error(77050003, "invalid regex")
  END IF
  LET hexPair AS String = collections::get(pat, i) & collections::get(pat, i + 1)
  LET value AS Integer = toInt(hexPair, 16) TRAP(err)
    FAIL error(77050003, "invalid regex")
  END TRAP
  RETURN __regex_LitScalar[__regex_chr(value), i + 2]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_parseHexEscape", BODY));
}
