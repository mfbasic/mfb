//! `__vector_angleFixed_integer4` — shared private helper for the `vector` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __vector_angleFixed_integer4(a AS Integer4, b AS Integer4) AS Fixed
  LET sa AS Integer = __vector_dot_integer4(a, a)
  LET sb AS Integer = __vector_dot_integer4(b, b)
  IF sa = 0 OR sb = 0 THEN
    FAIL error(77050002, "vector::angle with a zero-length vector")
  END IF
  LET la AS Fixed = math::sqrt(toFixed(sa))
  LET lb AS Fixed = math::sqrt(toFixed(sb))
  LET cosv AS Fixed = toFixed(__vector_dot_integer4(a, b)) / (la * lb)
  LET clamped AS Fixed = math::clamp(cosv, toFixed(-1.0), toFixed(1.0))
  RETURN math::acos(clamped)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("vector_angleFixed_integer4", BODY));
}
