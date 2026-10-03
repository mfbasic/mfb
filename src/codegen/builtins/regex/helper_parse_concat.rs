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
  ' bug-773: `c` is the scalar at `cAt`. The quantifier check after an atom reads
  ' the next scalar into it, so when no quantifier follows, the next turn of the
  ' loop reuses it instead of reading it again.
  MUT c AS String = ""
  MUT cAt AS Integer = -1
  DO WHILE idx < n
    IF cAt <> idx THEN
      c = collections::get(pat, idx)
      cAt = idx
    END IF
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
    ' bug-773: an atom is handed to __regex_parseQuantSuffix only when a quantifier
    ' follows it; otherwise its node goes straight into `parts`. The suffix parser
    ' returns the groups and names it was given, so neither is re-read from it.
    MUT qi AS Integer = 0
    MUT quant AS Boolean = FALSE
    IF c = "(" THEN
      LET pr AS __regex_Paren = __regex_parseParen(pat, n, idx, fl, gg, nm, depth)
      gg = pr.groups
      nm = pr.names
      IF pr.isDir THEN
        fl = pr.flags
        idx = pr.nxt
        CONTINUE DO
      END IF
      qi = pr.nxt
      IF qi < n THEN
        c = collections::get(pat, qi)
        cAt = qi
        quant = (c = "*" OR c = "+" OR c = "?" OR (c = "{" AND __regex_isCountedAt(pat, n, qi)))
      END IF
      IF quant THEN
        LET q AS __regex_Parse = __regex_parseQuantSuffix(pat, n, qi, fl, pr.node, gg, nm)
        parts = collections::append(parts, q.node)
        idx = q.nxt
      ELSE
        parts = collections::append(parts, pr.node)
        idx = qi
      END IF
      CONTINUE DO
    END IF
    IF c = "*" OR c = "+" OR c = "?" THEN
      FAIL error(77050003, "invalid regex")
    END IF
    IF c = "{" AND __regex_isCountedAt(pat, n, idx) THEN
      FAIL error(77050003, "invalid regex")
    END IF
    IF c = "." OR c = "^" OR c = "$" OR c = "[" OR c = "\\" THEN
      ' __regex_parseAtom returns the groups and names it was given.
      LET atom AS __regex_Parse = __regex_parseAtom(pat, n, idx, fl, gg, nm)
      qi = atom.nxt
      IF qi < n THEN
        c = collections::get(pat, qi)
        cAt = qi
        quant = (c = "*" OR c = "+" OR c = "?" OR (c = "{" AND __regex_isCountedAt(pat, n, qi)))
      END IF
      IF quant THEN
        LET q AS __regex_Parse = __regex_parseQuantSuffix(pat, n, qi, fl, atom.node, gg, nm)
        parts = collections::append(parts, q.node)
        idx = q.nxt
      ELSE
        parts = collections::append(parts, atom.node)
        idx = qi
      END IF
      CONTINUE DO
    END IF
    ' bug-773: a plain literal -- __regex_parseAtom's last case -- is built here, so
    ' it is not wrapped in a __regex_Parse only to be unwrapped again. `c` is one
    ' scalar of __regex_toScalars, so toScalar is total and gives the code point
    ' __regex_scalarToCp would, without its utf32Encode list.
    LET lit AS __regex_Node = __regex_Lit[c, fl.ci, toInt(toScalar(c))]
    qi = idx + 1
    IF qi < n THEN
      c = collections::get(pat, qi)
      cAt = qi
      quant = (c = "*" OR c = "+" OR c = "?" OR (c = "{" AND __regex_isCountedAt(pat, n, qi)))
    END IF
    IF quant THEN
      LET q AS __regex_Parse = __regex_parseQuantSuffix(pat, n, qi, fl, lit, gg, nm)
      parts = collections::append(parts, q.node)
      idx = q.nxt
    ELSE
      parts = collections::append(parts, lit)
      idx = qi
    END IF
  LOOP
  IF len(parts) = 1 THEN
    RETURN __regex_Parse[collections::get(parts, 0), idx, gg, nm]
  END IF
  LET node AS __regex_Node = __regex_Concat[parts]
  RETURN __regex_Parse[node, idx, gg, nm]
END FUNC"##;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_parseConcat", BODY));
}
