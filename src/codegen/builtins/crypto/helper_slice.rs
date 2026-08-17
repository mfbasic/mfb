//! `__crypto_slice` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-339 B3: native range copy [start, stop). Every caller passes start <= stop.
FUNC __crypto_slice(data AS List OF Byte, start AS Integer, stop AS Integer) AS List OF Byte
  RETURN collections::mid(data, start, stop - start)
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_slice", BODY));
}
