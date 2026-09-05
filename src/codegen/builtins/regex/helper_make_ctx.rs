//! `__regex_makeCtx` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-510 (DEC-03): the subject is held once, as its scalars. It used to be held
' twice -- a `List OF String` of one-character Strings as well -- at ~750 bytes per
' character for the pair on a 1.2 MB subject. Every consumer that needs a character
' as text builds it from the scalar with `__regex_chr`, which is exactly the String
' the old list held.
' bug-510 (DEC-02): every public call builds one context at its start, so this is
' where the call-wide backtracking budget is armed: the per-search budget plus a
' hundred steps per scalar of subject. `findAll`/`replace` can no longer spend a
' fresh budget on every match, while an ordinary scan of a long subject -- a few
' steps per scalar -- stays far inside the line.
FUNC __regex_makeCtx(value AS String) AS __regex_Ctx
  LET cps AS List OF Integer = encoding::utf32Encode(value)
  __regex_callSteps = 0
  __regex_callBudget = __REGEX_STEP_BUDGET + 100 * len(cps)
  RETURN __regex_Ctx[cps, len(cps)]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_makeCtx", BODY));
}
