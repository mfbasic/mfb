//! `__crypto_randomInt` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Uniform, unbiased integer in the inclusive range [min, max] (rejection sampled).
FUNC __crypto_randomInt(min AS Integer, max AS Integer) AS Integer
  IF min > max THEN
    FAIL error(77050002, "randomInt min > max")
  END IF
  IF min = max THEN
    RETURN min
  END IF
  LET span AS Integer = max - min
  LET range AS Integer = span + 1
  IF range <= 0 THEN
    FAIL error(77050002, "randomInt range too large")
  END IF
  LET maxVal AS Integer = 4611686018427387904
  ' bug-305: when `range > maxVal` (2^62), `maxVal MOD range` is `maxVal`, so
  ' `limit` was 0 and `WHILE v >= limit` never terminated -- `v` is always >= 0.
  ' The `range <= 0` guard above only catches i64 overflow, not the band
  ' (2^62, 2^63-1]. Such a range needs more entropy than a 62-bit draw provides,
  ' so draw 63 bits instead.
  '
  ' For any `range` in that band, floor(2^63 / range) is exactly 1 (since
  ' 2*range > 2^63 >= range), so the rejection limit IS `range` and the accepted
  ' draw needs no modulo -- which also avoids naming 2^63, a value an Integer
  ' cannot hold.
  IF range > maxVal THEN
    MUT wide AS Integer = __crypto_rand63()
    WHILE wide >= range
      wide = __crypto_rand63()
    END WHILE
    RETURN min + wide
  END IF
  LET limit AS Integer = maxVal - (maxVal MOD range)
  MUT v AS Integer = __crypto_rand62()
  WHILE v >= limit
    v = __crypto_rand62()
  END WHILE
  RETURN min + (v MOD range)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_randomInt", BODY));
}
