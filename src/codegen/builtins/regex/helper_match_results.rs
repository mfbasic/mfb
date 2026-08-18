//! `__regex_matchResults` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-339 C3: the global-match walk — including the zero-width-match rule (a
' match ending where the previous one did is skipped by advancing one position,
' otherwise it is recorded and the cursor jumps past it) — lived identically in
' both __regex_findAll and __regex_replace. Written once here; both consume the
' resulting list of matches, differing only in what they do with each one.
FUNC __regex_matchResults(prog AS __regex_Program, ctx AS __regex_Ctx, start AS Integer) AS List OF __regex_Result
  MUT out AS List OF __regex_Result = []
  MUT lastEnd AS Integer = start
  MUT lastMatch AS Integer = -1
  DO WHILE lastEnd <= ctx.n
    LET r AS __regex_Result = __regex_searchFrom(prog, ctx, lastEnd)
    IF r.ok = FALSE THEN
      EXIT DO
    END IF
    LET mstart AS Integer = collections::get(r.caps, 0)
    LET mend AS Integer = r.pos
    IF mstart = mend THEN
      IF mend = lastMatch THEN
        lastEnd = mend + 1
        CONTINUE DO
      END IF
      out = collections::append(out, r)
      lastMatch = mend
      lastEnd = mend + 1
      CONTINUE DO
    END IF
    out = collections::append(out, r)
    lastMatch = mend
    lastEnd = mend
  LOOP
  RETURN out
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_matchResults", BODY));
}
