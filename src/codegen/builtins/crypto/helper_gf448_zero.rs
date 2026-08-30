//! `__crypto_gf448Zero` — shared private helper for the `crypto` package.
//!
//! The Curve448 field GF(2^448 − 2^224 − 1) is carried as 16 little-endian
//! limbs of 28 bits each (`List OF Integer`), the representation of the
//! Goldilocks reference arithmetic; every field op returns limbs carried back
//! into `0..2^28` so the schoolbook product's accumulators stay far below the
//! trapping `Integer` boundary (see `gf448_mul_accumulators_fit_i63`).
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' The GF(2^448-2^224-1) zero element: 16 limbs of 28 bits.
FUNC __crypto_gf448Zero() AS List OF Integer
  RETURN [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_gf448Zero", BODY));
}
