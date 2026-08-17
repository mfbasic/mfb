//! `__crypto_truncate` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' The first `n` bytes of `data`.
' bug-339 B3: native prefix copy. Formerly an element loop; __crypto_bytePrefix
' was a byte-identical twin of this and has been folded into it.
FUNC __crypto_truncate(data AS List OF Byte, n AS Integer) AS List OF Byte
  RETURN collections::mid(data, 0, n)
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_truncate", BODY));
}
