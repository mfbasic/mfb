//! `__crypto_copyBytes` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' A fresh mutable copy of `data` (so appends don't disturb the caller's input).
' bug-339 B3: delegate to the native bulk range copy instead of an element loop.
FUNC __crypto_copyBytes(data AS List OF Byte) AS List OF Byte
  RETURN collections::mid(data, 0, len(data))
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_copyBytes", BODY));
}
