//! `__regex_parseConcat` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r##"FUNC __regex_parseConcat(pat AS List OF String, n AS Integer, i AS Integer, flags AS __regex_Flags, g AS Integer, names AS Map OF String TO Integer, depth AS Integer) AS __regex_Parse
  MUT idx AS Integer = i
  MUT gg AS Integer = g
  MUT nm AS Map OF String TO Integer = names
  MUT fl AS __regex_Flags = flags
  MUT parts AS List OF __regex_Node = []
  DO WHILE idx < n
    LET c AS String = collections::get(pat, idx)
    IF c = "|" OR c = ")" THEN
      EXIT DO
    END IF
    IF fl.verbose AND __regex_isPatSpace(c) THEN
      idx = idx + 1
      CONTINUE DO
    END IF
    IF fl.verbose AND c = "#" THEN
      WHILE idx < n AND collections::get(pat, idx) <> "\n"
        idx = idx + 1
      END WHILE
      CONTINUE DO
    END IF
    IF c = "(" THEN
      LET pr AS __regex_Paren = __regex_parseParen(pat, n, idx, fl, gg, nm, depth)
      gg = pr.groups
      nm = pr.names
      IF pr.isDir THEN
        fl = pr.flags
        idx = pr.nxt
        CONTINUE DO
      END IF
      LET q AS __regex_Parse = __regex_parseQuantSuffix(pat, n, pr.nxt, fl, pr.node, gg, nm)
      parts = collections::append(parts, q.node)
      idx = q.nxt
      gg = q.groups
      nm = q.names
      CONTINUE DO
    END IF
    IF c = "*" OR c = "+" OR c = "?" THEN
      FAIL error(77050003, "invalid regex")
    END IF
    IF c = "{" AND __regex_isCountedAt(pat, n, idx) THEN
      FAIL error(77050003, "invalid regex")
    END IF
    LET atom AS __regex_Parse = __regex_parseAtom(pat, n, idx, fl, gg, nm)
    LET q AS __regex_Parse = __regex_parseQuantSuffix(pat, n, atom.nxt, fl, atom.node, atom.groups, atom.names)
    parts = collections::append(parts, q.node)
    idx = q.nxt
    gg = q.groups
    nm = q.names
  LOOP
  IF len(parts) = 1 THEN
    RETURN __regex_Parse[collections::get(parts, 0), idx, gg, nm]
  END IF
  LET node AS __regex_Node = __regex_Concat[parts]
  RETURN __regex_Parse[node, idx, gg, nm]
END FUNC"##;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_parseConcat", BODY));
}
