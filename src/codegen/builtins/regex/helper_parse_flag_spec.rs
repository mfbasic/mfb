//! `__regex_parseFlagSpec` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __regex_parseFlagSpec(pat AS List OF String, n AS Integer, i AS Integer, base AS __regex_Flags) AS __regex_FlagSpec
  MUT fi AS Boolean = base.ci
  MUT fm AS Boolean = base.ml
  MUT fs AS Boolean = base.dotall
  MUT fu AS Boolean = base.ungreedy
  MUT fx AS Boolean = base.verbose
  MUT j AS Integer = i
  MUT any AS Boolean = FALSE
  DO WHILE j < n
    LET c AS String = collections::get(pat, j)
    IF c = "i" THEN
      fi = TRUE
    ELSEIF c = "m" THEN
      fm = TRUE
    ELSEIF c = "s" THEN
      fs = TRUE
    ELSEIF c = "U" THEN
      fu = TRUE
    ELSEIF c = "x" THEN
      fx = TRUE
    ELSE
      EXIT DO
    END IF
    any = TRUE
    j = j + 1
  LOOP
  IF j < n AND collections::get(pat, j) = "-" THEN
    j = j + 1
    MUT negAny AS Boolean = FALSE
    DO WHILE j < n
      LET c AS String = collections::get(pat, j)
      IF c = "i" THEN
        fi = FALSE
      ELSEIF c = "m" THEN
        fm = FALSE
      ELSEIF c = "s" THEN
        fs = FALSE
      ELSEIF c = "U" THEN
        fu = FALSE
      ELSEIF c = "x" THEN
        fx = FALSE
      ELSE
        EXIT DO
      END IF
      negAny = TRUE
      any = TRUE
      j = j + 1
    LOOP
    IF negAny = FALSE THEN
      FAIL error(77050003, "invalid regex")
    END IF
  END IF
  LET fl AS __regex_Flags = __regex_Flags[fi, fm, fs, fu, fx]
  MUT term AS String = ""
  IF j < n THEN
    term = collections::get(pat, j)
  END IF
  RETURN __regex_FlagSpec[fl, any, term, j]
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_parseFlagSpec", BODY));
}
