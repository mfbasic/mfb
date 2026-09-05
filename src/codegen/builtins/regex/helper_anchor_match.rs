//! `__regex_anchorMatch` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __regex_anchorMatch(anchor AS __regex_Anchor, pos AS Integer, ctx AS __regex_Ctx) AS Boolean
  IF anchor.kind = 3 THEN
    RETURN pos = 0
  END IF
  IF anchor.kind = 4 THEN
    RETURN pos = ctx.n
  END IF
  IF anchor.kind = 1 THEN
    IF pos = 0 THEN
      RETURN TRUE
    END IF
    IF anchor.ml AND collections::get(ctx.cps, pos - 1) = 10 THEN
      RETURN TRUE
    END IF
    RETURN FALSE
  END IF
  IF anchor.kind = 2 THEN
    IF pos = ctx.n THEN
      RETURN TRUE
    END IF
    IF anchor.ml AND collections::get(ctx.cps, pos) = 10 THEN
      RETURN TRUE
    END IF
    RETURN FALSE
  END IF
  IF anchor.kind = 5 THEN
    RETURN __regex_wordBoundary(pos, ctx)
  END IF
  RETURN NOT __regex_wordBoundary(pos, ctx)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_anchorMatch", BODY));
}
