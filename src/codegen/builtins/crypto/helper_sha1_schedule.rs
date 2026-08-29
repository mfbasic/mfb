//! `__crypto_sha1Schedule` — shared private helper for the `crypto` package.
//!
//! The 80-word SHA-1 message schedule (FIPS 180-4 §6.1.2 step 1): the 16 big-endian
//! block words, then `W_t = ROTL1(W_{t-3} XOR W_{t-8} XOR W_{t-14} XOR W_{t-16})`.
//! The rotate is the one difference from the (unrotated) SHA-0 schedule.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Build the 80-entry SHA-1 message schedule for the block at `base`.
FUNC __crypto_sha1Schedule(msg AS List OF Byte, base AS Integer) AS List OF Integer
  MUT w AS List OF Integer = []
  MUT t AS Integer = 0
  WHILE t < 16
    LET word AS Integer = __crypto_beWord(msg, base + t * 4)
    w = collections::append(w, word)
    t = t + 1
  END WHILE
  t = 16
  WHILE t < 80
    LET a AS Integer = bits::bxor(collections::get(w, t - 3), collections::get(w, t - 8))
    LET b AS Integer = bits::bxor(collections::get(w, t - 14), collections::get(w, t - 16))
    LET word AS Integer = __crypto_rotl32(bits::bxor(a, b), 1)
    w = collections::append(w, word)
    t = t + 1
  END WHILE
  RETURN w
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_sha1Schedule", BODY));
}
