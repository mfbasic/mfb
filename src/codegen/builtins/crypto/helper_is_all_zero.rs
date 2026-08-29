//! `__crypto_isAllZero` — shared private helper for the `crypto` package.
//!
//! Whether every byte is zero, accumulated with a bitwise OR over the whole
//! list (no early exit), so the check's timing depends only on the length —
//! the RFC 7748 §6.1 all-zero shared-secret test.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' TRUE when every byte of `b` is zero; scans the whole list (no early exit).
FUNC __crypto_isAllZero(b AS List OF Byte) AS Boolean
  MUT acc AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < len(b)
    acc = bits::bor(acc, toInt(collections::get(b, i)))
    i = i + 1
  END WHILE
  RETURN acc = 0
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_isAllZero", BODY));
}
