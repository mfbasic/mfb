//! `__encoding_punyAdapt` — shared private helper for the `encoding` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __encoding_punyAdapt(delta AS Integer, numPoints AS Integer, firstTime AS Boolean) AS Integer
  MUT d AS Integer = delta
  IF firstTime THEN
    d = d / 700
  ELSE
    d = d / 2
  END IF
  d = d + d / numPoints
  MUT k AS Integer = 0
  WHILE d > 455
    d = d / 35
    k = k + 36
  END WHILE
  RETURN k + (36 * d) / (d + 38)
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("encoding_punyAdapt", BODY));
}
