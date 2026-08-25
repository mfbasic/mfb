//! `__vector_isqrtFloor` — shared private helper for the `vector` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __vector_isqrtFloor(n AS Integer) AS Integer
  IF n <= 0 THEN
    RETURN 0
  END IF
  MUT seed AS Integer = toInt(math::sqrt(toFloat(n)))
  IF seed < 0 THEN
    seed = 0
  END IF
  ' Bring seed down to a lower bound: seed*seed > n  <=>  seed > n / seed.
  WHILE seed > 0 AND seed > n / seed
    seed = seed - 1
  END WHILE
  ' Climb while (seed+1)^2 <= n  <=>  (seed+1) <= n / (seed+1).
  WHILE seed + 1 <= n / (seed + 1)
    seed = seed + 1
  END WHILE
  RETURN seed
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("vector_isqrtFloor", BODY));
}
