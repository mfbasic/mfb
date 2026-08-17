//! `__crypto_sha384Iv` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __crypto_sha384Iv() AS List OF Integer
  LET hex AS String = "cbbb9d5dc1059ed8629a292a367cd5079159015a3070dd17152fecd8f70e593967332667ffc00b318eb44a8768581511db0c2e0d64f98fa747b5481dbefa4fa4"
  RETURN __crypto_beWords64(encoding::hexDecode(hex))
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_sha384Iv", BODY));
}
