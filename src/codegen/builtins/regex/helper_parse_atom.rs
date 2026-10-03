//! `__regex_parseAtom` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-773: the atoms that need more than their one scalar -- `.`, `^`, `$`, a class and
' an escape. A plain literal is built by __regex_parseConcat itself, which calls this only
' when `c` is one of those five, so the last case is the escape.
FUNC __regex_parseAtom(pat AS List OF String, n AS Integer, i AS Integer, flags AS __regex_Flags, g AS Integer, names AS Map OF String TO Integer) AS __regex_Parse
  LET c AS String = collections::get(pat, i)
  IF c = "." THEN
    LET node AS __regex_Node = __regex_Any[flags.dotall]
    RETURN __regex_Parse[node, i + 1, g, names]
  END IF
  IF c = "^" THEN
    LET node AS __regex_Node = __regex_Anchor[1, flags.ml]
    RETURN __regex_Parse[node, i + 1, g, names]
  END IF
  IF c = "$" THEN
    LET node AS __regex_Node = __regex_Anchor[2, flags.ml]
    RETURN __regex_Parse[node, i + 1, g, names]
  END IF
  IF c = "[" THEN
    RETURN __regex_parseClass(pat, n, i, flags, g, names)
  END IF
  RETURN __regex_parseEscapeAtom(pat, n, i, flags, g, names)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_parseAtom", BODY));
}
