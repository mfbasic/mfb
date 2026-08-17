//! `__regex_expand` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __regex_expand(repl AS String, r AS __regex_Result, value AS String, prog AS __regex_Program) AS String
  LET rs AS List OF String = __regex_toScalars(repl)
  LET m AS Integer = len(rs)
  MUT out AS String = ""
  MUT i AS Integer = 0
  DO WHILE i < m
    LET c AS String = collections::get(rs, i)
    IF c <> "$" THEN
      out = out & c
      i = i + 1
      CONTINUE DO
    END IF
    IF i + 1 >= m THEN
      out = out & "$"
      i = i + 1
      CONTINUE DO
    END IF
    LET d AS String = collections::get(rs, i + 1)
    IF d = "$" THEN
      out = out & "$"
      i = i + 2
      CONTINUE DO
    END IF
    IF d = "{" THEN
      MUT j AS Integer = i + 2
      MUT ref AS String = ""
      MUT closed AS Boolean = FALSE
      DO WHILE j < m
        LET e AS String = collections::get(rs, j)
        IF e = "}" THEN
          closed = TRUE
          EXIT DO
        END IF
        ref = ref & e
        j = j + 1
      LOOP
      IF closed THEN
        out = out & __regex_lookupRef(ref, r, value, prog)
        i = j + 1
      ELSE
        out = out & "$"
        i = i + 1
      END IF
      CONTINUE DO
    END IF
    IF __regex_isDigit(d) THEN
      MUT j AS Integer = i + 1
      MUT num AS String = ""
      DO WHILE j < m AND __regex_isDigit(collections::get(rs, j))
        num = num & collections::get(rs, j)
        j = j + 1
      LOOP
      out = out & __regex_lookupNum(__regex_parseIntClamp(num), r, value, prog)
      i = j
      CONTINUE DO
    END IF
    IF __regex_isNameStart(d) THEN
      MUT j AS Integer = i + 1
      MUT nm AS String = ""
      DO WHILE j < m AND __regex_isNameCont(collections::get(rs, j))
        nm = nm & collections::get(rs, j)
        j = j + 1
      LOOP
      out = out & __regex_lookupName(nm, r, value, prog)
      i = j
      CONTINUE DO
    END IF
    out = out & "$"
    i = i + 1
  LOOP
  RETURN out
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_expand", BODY));
}
