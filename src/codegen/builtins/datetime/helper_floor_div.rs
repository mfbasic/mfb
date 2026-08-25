//! `__datetime_floorDiv` — shared private helper for the `datetime` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __datetime_floorDiv(a AS Integer, b AS Integer) AS Integer
  MUT q AS Integer = a / b
  LET r AS Integer = a MOD b
  IF r <> 0 AND r < 0 THEN
    q = q - 1
  END IF
  RETURN q
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("datetime_floorDiv", BODY));
}
