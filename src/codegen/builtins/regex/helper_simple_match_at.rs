//! `__regex_simpleMatchAt` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Does a simple node match the single scalar at `pos`? Mirrors the Lit/Any/Class
' arms of __regex_run exactly, so the iterative repeat path accepts precisely what
' the general one does.
FUNC __regex_simpleMatchAt(node AS __regex_Node, pos AS Integer, ctx AS __regex_Ctx) AS Boolean
  IF pos >= ctx.n THEN
    RETURN FALSE
  END IF
  MATCH node
    CASE __regex_Lit(litNode)
      RETURN __regex_charEq(litNode, collections::get(ctx.cps, pos))
    CASE __regex_Any(anyNode)
      IF anyNode.dotall THEN
        RETURN TRUE
      END IF
      RETURN collections::get(ctx.cps, pos) <> 10
    CASE __regex_Class(clsNode)
      RETURN __regex_classMatch(clsNode, pos, ctx)
    CASE ELSE
      RETURN FALSE
  END MATCH
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_simpleMatchAt", BODY));
}
