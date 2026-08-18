//! `__regex_parseClass` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __regex_parseClass(pat AS List OF String, n AS Integer, i AS Integer, flags AS __regex_Flags, g AS Integer, names AS Map OF String TO Integer) AS __regex_Parse
  MUT idx AS Integer = i + 1
  MUT neg AS Boolean = FALSE
  IF idx < n AND collections::get(pat, idx) = "^" THEN
    neg = TRUE
    idx = idx + 1
  END IF
  MUT items AS List OF __regex_ClassItem = []
  MUT count AS Integer = 0
  DO WHILE TRUE
    IF idx >= n THEN
      FAIL error(77050003, "invalid regex")
    END IF
    LET c AS String = collections::get(pat, idx)
    IF c = "]" THEN
      IF count = 0 THEN
        FAIL error(77050003, "invalid regex")
      END IF
      idx = idx + 1
      EXIT DO
    END IF
    IF c = "&" AND idx + 1 < n AND collections::get(pat, idx + 1) = "&" THEN
      FAIL error(77050003, "invalid regex")
    END IF
    IF c = "[" AND idx + 1 < n AND collections::get(pat, idx + 1) = ":" THEN
      LET px AS __regex_Endpoint = __regex_parsePosix(pat, n, idx)
      items = collections::append(items, px.item)
      count = count + 1
      idx = px.nxt
      CONTINUE DO
    END IF
    LET ep AS __regex_Endpoint = __regex_parseClassEndpoint(pat, n, idx)
    idx = ep.nxt
    IF ep.kind = 1 THEN
      items = collections::append(items, ep.item)
      count = count + 1
      CONTINUE DO
    END IF
    IF idx < n AND collections::get(pat, idx) = "-" AND idx + 1 < n AND collections::get(pat, idx + 1) <> "]" THEN
      idx = idx + 1
      LET ep2 AS __regex_Endpoint = __regex_parseClassEndpoint(pat, n, idx)
      idx = ep2.nxt
      IF ep2.kind = 1 THEN
        FAIL error(77050003, "invalid regex")
      END IF
      IF ep.ch > ep2.ch THEN
        FAIL error(77050003, "invalid regex")
      END IF
      LET rng AS __regex_ClassItem = __regex_Range[ep.ch, ep2.ch]
      items = collections::append(items, rng)
      count = count + 1
      CONTINUE DO
    END IF
    LET sng AS __regex_ClassItem = __regex_Single[ep.ch]
    items = collections::append(items, sng)
    count = count + 1
  LOOP
  LET node AS __regex_Node = __regex_makeClass(neg, flags.ci, items)
  RETURN __regex_Parse[node, idx, g, names]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_parseClass", BODY));
}
