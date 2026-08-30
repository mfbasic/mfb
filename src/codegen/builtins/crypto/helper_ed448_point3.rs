//! `__crypto_ed448Point3` / `__crypto_ed448PointAt` — shared private helpers for
//! the `crypto` package (registered as one chunk).
//!
//! An edwards448 point in projective coordinates `(X : Y : Z)` is one flat
//! `List OF Integer` of 48 limbs (three 16-limb field elements); `Point3` builds
//! it and `PointAt(p, i)` extracts coordinate `i`. The identity is `(0 : 1 : 1)`.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' A projective edwards448 point (X : Y : Z) as one 48-limb list.
FUNC __crypto_ed448Point3(x AS List OF Integer, y AS List OF Integer, z AS List OF Integer) AS List OF Integer
  RETURN __crypto_concatInt(__crypto_concatInt(x, y), z)
END FUNC
' Coordinate `i` (0 = X, 1 = Y, 2 = Z) of a projective point.
FUNC __crypto_ed448PointAt(p AS List OF Integer, i AS Integer) AS List OF Integer
  RETURN collections::mid(p, i * 16, 16)
END FUNC
' The identity (0 : 1 : 1).
FUNC __crypto_ed448Identity() AS List OF Integer
  RETURN __crypto_ed448Point3(__crypto_gf448Zero(), __crypto_gf448One(), __crypto_gf448One())
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_ed448Point3", BODY));
}
