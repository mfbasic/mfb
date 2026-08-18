//! `__crypto_sha256Schedule` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Build the 64-entry SHA-256 message schedule for the block at `base`.
FUNC __crypto_sha256Schedule(msg AS List OF Byte, base AS Integer) AS List OF Integer
  MUT w AS List OF Integer = []
  MUT t AS Integer = 0
  WHILE t < 16
    LET word AS Integer = __crypto_beWord(msg, base + t * 4)
    w = collections::append(w, word)
    t = t + 1
  END WHILE
  t = 16
  WHILE t < 64
    LET a AS Integer = __crypto_ssig1(collections::get(w, t - 2))
    LET b AS Integer = collections::get(w, t - 7)
    LET c AS Integer = __crypto_ssig0(collections::get(w, t - 15))
    LET d AS Integer = collections::get(w, t - 16)
    LET s1 AS Integer = __crypto_add32(a, b)
    LET s2 AS Integer = __crypto_add32(c, d)
    LET word AS Integer = __crypto_add32(s1, s2)
    w = collections::append(w, word)
    t = t + 1
  END WHILE
  RETURN w
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_sha256Schedule", BODY));
}
