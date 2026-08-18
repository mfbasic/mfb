//! `__regex_parseNamedGroup` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __regex_parseNamedGroup(pat AS List OF String, n AS Integer, nameStart AS Integer, flags AS __regex_Flags, g AS Integer, names AS Map OF String TO Integer, depth AS Integer) AS __regex_Paren
  LET nm AS __regex_Name = __regex_parseName(pat, n, nameStart)
  IF collections::hasKey(names, nm.name) THEN
    FAIL error(77050003, "invalid regex")
  END IF
  LET slot AS Integer = g + 1
  LET names2 AS Map OF String TO Integer = collections::set(names, nm.name, slot)
  LET inner AS __regex_Parse = __regex_parseAlt(pat, n, nm.nxt, flags, slot, names2, depth)
  IF inner.nxt >= n OR collections::get(pat, inner.nxt) <> ")" THEN
    FAIL error(77050003, "invalid regex")
  END IF
  LET node AS __regex_Node = __regex_Group[inner.node, slot]
  RETURN __regex_Paren[FALSE, flags, node, inner.nxt + 1, inner.groups, inner.names]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_parseNamedGroup", BODY));
}
