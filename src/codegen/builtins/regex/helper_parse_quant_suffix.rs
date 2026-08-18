//! `__regex_parseQuantSuffix` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __regex_parseQuantSuffix(pat AS List OF String, n AS Integer, i AS Integer, flags AS __regex_Flags, atom AS __regex_Node, g AS Integer, names AS Map OF String TO Integer) AS __regex_Parse
  IF i >= n THEN
    RETURN __regex_Parse[atom, i, g, names]
  END IF
  LET c AS String = collections::get(pat, i)
  MUT lo AS Integer = 0
  MUT hi AS Integer = 0
  MUT has AS Boolean = FALSE
  MUT nxt AS Integer = i
  IF c = "*" THEN
    lo = 0
    hi = -1
    has = TRUE
    nxt = i + 1
  ELSEIF c = "+" THEN
    lo = 1
    hi = -1
    has = TRUE
    nxt = i + 1
  ELSEIF c = "?" THEN
    lo = 0
    hi = 1
    has = TRUE
    nxt = i + 1
  ELSEIF c = "{" AND __regex_isCountedAt(pat, n, i) THEN
    LET cnt AS __regex_Count = __regex_parseCounted(pat, n, i)
    lo = cnt.lo
    hi = cnt.hi
    has = TRUE
    nxt = cnt.nxt
  END IF
  IF has = FALSE THEN
    RETURN __regex_Parse[atom, i, g, names]
  END IF
  MUT greedy AS Boolean = TRUE
  IF flags.ungreedy THEN
    greedy = FALSE
  END IF
  IF nxt < n AND collections::get(pat, nxt) = "?" THEN
    IF flags.ungreedy THEN
      greedy = TRUE
    ELSE
      greedy = FALSE
    END IF
    nxt = nxt + 1
  END IF
  LET rep AS __regex_Node = __regex_Repeat[atom, lo, hi, greedy]
  RETURN __regex_Parse[rep, nxt, g, names]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_parseQuantSuffix", BODY));
}
