//! `__vector_isqrtRound` — shared private helper for the `vector` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Integer square root rounded half away from zero (n >= 0). The exact half
' (f + 0.5)^2 = f^2 + f + 0.25 is never an integer, so there is no tie: round up
' exactly when the remainder exceeds the floor.
FUNC __vector_isqrtRound(n AS Integer) AS Integer
  LET f AS Integer = __vector_isqrtFloor(n)
  IF n - f * f > f THEN
    RETURN f + 1
  END IF
  RETURN f
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("vector_isqrtRound", BODY));
}
