//! `__regex_parseClassEndpoint` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' One class endpoint; kind 0 = literal scalar (rangeable), 1 = item.
FUNC __regex_parseClassEndpoint(pat AS List OF String, n AS Integer, idx AS Integer) AS __regex_Endpoint
  LET c AS String = collections::get(pat, idx)
  IF c = "\\" THEN
    IF idx + 1 >= n THEN
      FAIL error(77050003, "invalid regex")
    END IF
    LET e AS String = collections::get(pat, idx + 1)
    LET sk AS Integer = __regex_shortKind(e)
    IF sk <> 0 THEN
      LET item AS __regex_ClassItem = __regex_Short[sk]
      RETURN __regex_Endpoint[1, "", item, idx + 2]
    END IF
    IF e = "p" OR e = "P" THEN
      LET pp AS __regex_PropParse = __regex_parseProp(pat, n, idx + 2, (e = "P"))
      LET item AS __regex_ClassItem = __regex_Prop[pp.name, pp.neg]
      RETURN __regex_Endpoint[1, "", item, pp.nxt]
    END IF
    IF e = "b" OR e = "A" OR e = "z" OR e = "B" THEN
      FAIL error(77050003, "invalid regex")
    END IF
    LET lit AS __regex_LitScalar = __regex_parseLiteralEscape(pat, n, idx, FALSE)
    LET dummy AS __regex_ClassItem = __regex_Single[lit.ch]
    RETURN __regex_Endpoint[0, lit.ch, dummy, lit.nxt]
  END IF
  LET dummy AS __regex_ClassItem = __regex_Single[c]
  RETURN __regex_Endpoint[0, c, dummy, idx + 1]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_parseClassEndpoint", BODY));
}
