//! `__crypto_sha512Schedule` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Build the 80-entry SHA-512 message schedule for the block at `base`.
FUNC __crypto_sha512Schedule(msg AS List OF Byte, base AS Integer) AS List OF Integer
  MUT w AS List OF Integer = []
  MUT t AS Integer = 0
  WHILE t < 16
    w = collections::append(w, __crypto_beWord64(msg, base + t * 8))
    t = t + 1
  END WHILE
  t = 16
  WHILE t < 80
    LET a AS Integer = __crypto_ssig1_64(collections::get(w, t - 2))
    LET b AS Integer = collections::get(w, t - 7)
    LET c AS Integer = __crypto_ssig0_64(collections::get(w, t - 15))
    LET d AS Integer = collections::get(w, t - 16)
    LET s1 AS Integer = __crypto_add64(a, b)
    LET s2 AS Integer = __crypto_add64(c, d)
    w = collections::append(w, __crypto_add64(s1, s2))
    t = t + 1
  END WHILE
  RETURN w
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_sha512Schedule", BODY));
}
