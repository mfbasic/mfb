//! `__regex_parseParen` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __regex_parseParen(pat AS List OF String, n AS Integer, i AS Integer, flags AS __regex_Flags, g AS Integer, names AS Map OF String TO Integer, depth AS Integer) AS __regex_Paren
  MUT idx AS Integer = i + 1
  IF idx >= n THEN
    FAIL error(77050003, "invalid regex")
  END IF
  LET c AS String = collections::get(pat, idx)
  IF c <> "?" THEN
    LET slot AS Integer = g + 1
    LET inner AS __regex_Parse = __regex_parseAlt(pat, n, idx, flags, slot, names, depth + 1)
    IF inner.nxt >= n OR collections::get(pat, inner.nxt) <> ")" THEN
      FAIL error(77050003, "invalid regex")
    END IF
    LET node AS __regex_Node = __regex_Group[inner.node, slot]
    RETURN __regex_Paren[FALSE, flags, node, inner.nxt + 1, inner.groups, inner.names]
  END IF
  idx = idx + 1
  IF idx >= n THEN
    FAIL error(77050003, "invalid regex")
  END IF
  LET d AS String = collections::get(pat, idx)
  IF d = ":" THEN
    LET inner AS __regex_Parse = __regex_parseAlt(pat, n, idx + 1, flags, g, names, depth + 1)
    IF inner.nxt >= n OR collections::get(pat, inner.nxt) <> ")" THEN
      FAIL error(77050003, "invalid regex")
    END IF
    RETURN __regex_Paren[FALSE, flags, inner.node, inner.nxt + 1, inner.groups, inner.names]
  END IF
  IF d = "<" THEN
    IF idx + 1 < n THEN
      LET d2 AS String = collections::get(pat, idx + 1)
      IF d2 = "=" OR d2 = "!" THEN
        FAIL error(77050003, "invalid regex")
      END IF
    END IF
    RETURN __regex_parseNamedGroup(pat, n, idx + 1, flags, g, names, depth + 1)
  END IF
  IF d = "P" THEN
    IF idx + 1 >= n OR collections::get(pat, idx + 1) <> "<" THEN
      FAIL error(77050003, "invalid regex")
    END IF
    RETURN __regex_parseNamedGroup(pat, n, idx + 2, flags, g, names, depth + 1)
  END IF
  IF d = "=" OR d = "!" THEN
    FAIL error(77050003, "invalid regex")
  END IF
  LET spec AS __regex_FlagSpec = __regex_parseFlagSpec(pat, n, idx, flags)
  IF spec.term = ")" THEN
    IF spec.any = FALSE THEN
      FAIL error(77050003, "invalid regex")
    END IF
    LET empty AS List OF __regex_Node = []
    LET node AS __regex_Node = __regex_Concat[empty]
    RETURN __regex_Paren[TRUE, spec.flags, node, spec.nxt + 1, g, names]
  END IF
  IF spec.term = ":" THEN
    LET inner AS __regex_Parse = __regex_parseAlt(pat, n, spec.nxt + 1, spec.flags, g, names, depth + 1)
    IF inner.nxt >= n OR collections::get(pat, inner.nxt) <> ")" THEN
      FAIL error(77050003, "invalid regex")
    END IF
    RETURN __regex_Paren[FALSE, flags, inner.node, inner.nxt + 1, inner.groups, inner.names]
  END IF
  FAIL error(77050003, "invalid regex")
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_parseParen", BODY));
}
