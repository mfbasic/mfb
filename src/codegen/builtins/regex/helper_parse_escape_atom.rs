//! `__regex_parseEscapeAtom` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __regex_parseEscapeAtom(pat AS List OF String, n AS Integer, i AS Integer, flags AS __regex_Flags, g AS Integer, names AS Map OF String TO Integer) AS __regex_Parse
  IF i + 1 >= n THEN
    FAIL error(77050003, "invalid regex")
  END IF
  LET e AS String = collections::get(pat, i + 1)
  IF e = "A" THEN
    LET node AS __regex_Node = __regex_Anchor[3, FALSE]
    RETURN __regex_Parse[node, i + 2, g, names]
  END IF
  IF e = "z" THEN
    LET node AS __regex_Node = __regex_Anchor[4, FALSE]
    RETURN __regex_Parse[node, i + 2, g, names]
  END IF
  IF e = "b" THEN
    LET node AS __regex_Node = __regex_Anchor[5, FALSE]
    RETURN __regex_Parse[node, i + 2, g, names]
  END IF
  IF e = "B" THEN
    LET node AS __regex_Node = __regex_Anchor[6, FALSE]
    RETURN __regex_Parse[node, i + 2, g, names]
  END IF
  LET sk AS Integer = __regex_shortKind(e)
  IF sk <> 0 THEN
    LET item AS __regex_ClassItem = __regex_Short[sk]
    LET items AS List OF __regex_ClassItem = [item]
    LET node AS __regex_Node = __regex_makeClass(FALSE, flags.ci, items)
    RETURN __regex_Parse[node, i + 2, g, names]
  END IF
  IF e = "p" OR e = "P" THEN
    LET pp AS __regex_PropParse = __regex_parseProp(pat, n, i + 2, (e = "P"))
    LET item AS __regex_ClassItem = __regex_Prop[pp.name, pp.neg]
    LET items AS List OF __regex_ClassItem = [item]
    LET node AS __regex_Node = __regex_makeClass(FALSE, flags.ci, items)
    RETURN __regex_Parse[node, pp.nxt, g, names]
  END IF
  LET lit AS __regex_LitScalar = __regex_parseLiteralEscape(pat, n, i, flags.verbose)
  LET node AS __regex_Node = __regex_Lit[lit.ch, flags.ci, __regex_scalarToCp(lit.ch)]
  RETURN __regex_Parse[node, lit.nxt, g, names]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_parseEscapeAtom", BODY));
}
