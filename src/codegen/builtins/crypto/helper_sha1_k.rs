//! `__crypto_sha1K` — shared private helper for the `crypto` package.
//!
//! The SHA-1 round constant `K_t` (FIPS 180-4 §4.2.1): `0x5a827999` for rounds
//! 0–19, `0x6ed9eba1` for 20–39, `0x8f1bbcdc` for 40–59, `0xca62c1d6` for 60–79.
//! Selected by the public round counter alone.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' SHA-1 round constant K_t: 0x5a827999 / 0x6ed9eba1 / 0x8f1bbcdc / 0xca62c1d6 per 20-round quarter.
FUNC __crypto_sha1K(t AS Integer) AS Integer
  IF t < 20 THEN
    RETURN 1518500249
  END IF
  IF t < 40 THEN
    RETURN 1859775393
  END IF
  IF t < 60 THEN
    RETURN 2400959708
  END IF
  RETURN 3395469782
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_sha1K", BODY));
}
