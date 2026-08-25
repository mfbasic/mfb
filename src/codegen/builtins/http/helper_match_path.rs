//! `__http_matchPath` — shared private helper for the `http` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __http_matchPath(pattern AS String, path AS String) AS __http_RouteMatch
  MUT params AS Map OF String TO String = Map OF String TO String {}
  LET pSegs AS List OF String = __http_segments(pattern)
  LET uSegs AS List OF String = __http_segments(path)
  LET pn AS Integer = len(pSegs)
  LET un AS Integer = len(uSegs)
  MUT pi AS Integer = 0
  MUT ui AS Integer = 0
  MUT ok AS Boolean = TRUE
  MUT stop AS Boolean = FALSE
  WHILE pi < pn AND stop = FALSE
    LET seg AS String = collections::get(pSegs, pi)
    IF seg = "*" THEN
      MUT rest AS String = ""
      MUT j AS Integer = ui
      WHILE j < un
        IF rest = "" THEN
          rest = collections::get(uSegs, j)
        ELSE
          rest = rest & "/" & collections::get(uSegs, j)
        END IF
        j = j + 1
      END WHILE
      params = collections::set(params, "*", rest)
      ui = un
      pi = pn
      stop = TRUE
    ELSEIF strings::startsWith(seg, ":") AND strings::endsWith(seg, "?") THEN
      LET nm AS String = __http_slice(seg, 1, len(seg) - 1)
      IF ui < un THEN
        params = collections::set(params, nm, collections::get(uSegs, ui))
        ui = ui + 1
      END IF
      pi = pi + 1
    ELSEIF strings::startsWith(seg, ":") THEN
      IF ui < un THEN
        params = collections::set(params, __http_slice(seg, 1, len(seg)), collections::get(uSegs, ui))
        ui = ui + 1
        pi = pi + 1
      ELSE
        ok = FALSE
        stop = TRUE
      END IF
    ELSE
      IF ui < un AND collections::get(uSegs, ui) = seg THEN
        ui = ui + 1
        pi = pi + 1
      ELSE
        ok = FALSE
        stop = TRUE
      END IF
    END IF
  END WHILE
  IF ok = TRUE AND ui < un THEN
    ok = FALSE
  END IF
  RETURN __http_RouteMatch[ok, params]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_matchPath", BODY));
}
