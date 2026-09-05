//! `__regex_requiredFirstCp` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' plan-77 R5: the code point a match MUST begin with, or -1 when the pattern has
' no fixed first literal (so __regex_searchFrom can fast-skip start offsets). Only
' a non-folding literal yields a fixed cp; Concat/Group are transparent to their
' first child. Everything else (Any, Class, Anchor, Alt, Repeat — which may be
' optional or multi-valued) is conservatively "unknown" (-1).
FUNC __regex_requiredFirstCp(node AS __regex_Node) AS Integer
  MATCH node
    CASE __regex_Lit(lit)
      IF lit.fold THEN
        RETURN -1
      END IF
      RETURN lit.cp
    CASE __regex_Concat(cat)
      IF len(cat.parts) = 0 THEN
        RETURN -1
      END IF
      RETURN __regex_requiredFirstCp(collections::get(cat.parts, 0))
    CASE __regex_Group(grp)
      RETURN __regex_requiredFirstCp(grp.child)
    CASE ELSE
      RETURN -1
  END MATCH
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_requiredFirstCp", BODY));
}
