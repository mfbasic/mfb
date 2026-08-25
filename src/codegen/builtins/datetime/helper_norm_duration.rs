//! `__datetime_normDuration` — shared private helper for the `datetime` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __datetime_normDuration(seconds AS Integer, nanos AS Integer) AS Duration
  MUT q AS Integer = nanos / 1000000000
  MUT r AS Integer = nanos MOD 1000000000
  IF r < 0 THEN
    r = r + 1000000000
    q = q - 1
  END IF
  RETURN Duration[seconds + q, r]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("datetime_normDuration", BODY));
}
