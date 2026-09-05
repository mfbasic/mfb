//! `__regex_charEq` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-510: compares a literal to the scalar at the cursor. Without folding the two
' code points are compared directly. With folding both sides are case-folded as
' Strings, exactly as before -- `strings::caseFold` can expand a scalar (U+00DF to
' "ss"), so a code-point compare would not be the same relation; `__regex_chr(cp)`
' is precisely the one-character String the old `ctx.text` list held.
FUNC __regex_charEq(lit AS __regex_Lit, cp AS Integer) AS Boolean
  IF lit.fold THEN
    RETURN strings::caseFold(lit.ch) = strings::caseFold(__regex_chr(cp))
  END IF
  RETURN lit.cp = cp
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_charEq", BODY));
}
