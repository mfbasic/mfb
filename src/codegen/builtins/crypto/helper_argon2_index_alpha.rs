//! `__crypto_argon2IndexAlpha` — shared private helper for the `crypto` package.
//!
//! Argon2's reference-block index (RFC 9106 §3.4.2). This is where a memory-hard
//! function is easiest to get subtly wrong: the size of the referenceable area differs
//! per pass, per slice and by whether the reference lane is the current one, and every
//! wrong variant still produces a plausible digest.
//!
//! Registered via `add_helper` under [`HelperGate::WhenUsed`] on `argon2id`, so a
//! program that imports `crypto` without calling `crypto::argon2id` carries none of
//! the BLAKE2b/Argon2 source. Renders in the helper section of the assembled source.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{HelperGate, RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"IMPORT bits

' Argon2's reference-block index (RFC 9106 section 3.4.2): size the reference area
' for this pass/slice/lane, map the pseudo-random word onto it with the nonuniform
' J1 distribution, and rotate by the slice start.
FUNC __crypto_argon2IndexAlpha(pass AS Integer, laneLen AS Integer, segLen AS Integer, slice AS Integer, index AS Integer, rnd AS Integer, sameLane AS Boolean) AS Integer
  MUT area AS Integer = 0
  IF pass = 0 THEN
    IF slice = 0 THEN
      area = index - 1
    ELSE
      IF sameLane THEN
        area = slice * segLen + index - 1
      ELSE
        area = slice * segLen
        IF index = 0 THEN
          area = area - 1
        END IF
      END IF
    END IF
  ELSE
    IF sameLane THEN
      area = laneLen - segLen + index - 1
    ELSE
      area = laneLen - segLen
      IF index = 0 THEN
        area = area - 1
      END IF
    END IF
  END IF
  MUT rel AS Integer = bits::sr(__crypto_argon2Mul32(rnd, rnd), 32)
  rel = area - 1 - bits::sr(__crypto_argon2Mul32(area, rel), 32)
  MUT start AS Integer = 0
  IF pass <> 0 AND slice <> 3 THEN
    start = (slice + 1) * segLen
  END IF
  RETURN (start + rel) MOD laneLen
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper {
        name: "crypto_argon2IndexAlpha",
        gate: HelperGate::WhenUsed(&["argon2id"]),
        body: Some(BODY),
        import_name: None,
        natively_called: false,
    });
}
