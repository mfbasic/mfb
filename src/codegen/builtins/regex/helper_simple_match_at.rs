//! `__regex_simpleMatchAt` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Does the one-scalar leaf `leaves[at]` (Lit, Any or Class) match the scalar at `pos`?
' Both the node visit and the greedy simple-repeat loop of __regex_run ask here, so the
' two accept precisely the same scalars. plan-134-I: `leaves` is a parameter the helper
' never rebinds and `leaf` is read only by the MATCH, so the `get` borrows the leaf in
' place (plan-86 E) -- a visit copies nothing.
FUNC __regex_simpleMatchAt(leaves AS List OF __regex_Leaf, at AS Integer, pos AS Integer, ctx AS __regex_Ctx) AS Boolean
  IF pos >= ctx.n THEN
    RETURN FALSE
  END IF
  LET leaf AS __regex_Leaf = collections::get(leaves, at)
  MATCH leaf
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
