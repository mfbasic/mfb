//! `__regex_isSimpleNode` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __regex_isSimpleNode(node AS __regex_Node) AS Boolean
  MATCH node
    CASE __regex_Lit(litNode)
      RETURN TRUE
    CASE __regex_Any(anyNode)
      RETURN TRUE
    CASE __regex_Class(clsNode)
      RETURN TRUE
    CASE ELSE
      RETURN FALSE
  END MATCH
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_isSimpleNode", BODY));
}
