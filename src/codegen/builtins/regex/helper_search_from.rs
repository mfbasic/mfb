//! `__regex_searchFrom` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __regex_searchFrom(prog AS __regex_Program, ctx AS __regex_Ctx, from AS Integer) AS __regex_Result
  ' bug-315: one budget per search, not per start offset -- the scan over start
  ' positions is itself part of the work an adversarial pattern multiplies.
  __regex_steps = 0
  ' plan-77 R5: if the pattern must begin with a fixed literal code point, skip
  ' start offsets that cannot match it instead of running the full engine at each.
  LET firstCp AS Integer = __regex_requiredFirstCp(prog.root)
  MUT s AS Integer = from
  WHILE s <= ctx.n
    IF firstCp >= 0 THEN
      WHILE s < ctx.n AND collections::get(ctx.cps, s) <> firstCp
        s = s + 1
      END WHILE
    END IF
    LET r AS __regex_Result = __regex_tryAt(prog, ctx, s)
    IF r.ok THEN
      RETURN r
    END IF
    s = s + 1
  END WHILE
  RETURN __regex_fail()
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_searchFrom", BODY));
}
