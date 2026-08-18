//! `__crypto_ghashMul` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Multiply two GF(2^128) elements (each a 16-byte big-endian block).
FUNC __crypto_ghashMul(xHi AS Integer, xLo AS Integer, yHi AS Integer, yLo AS Integer) AS List OF Integer
  MUT zHi AS Integer = 0
  MUT zLo AS Integer = 0
  MUT vHi AS Integer = yHi
  MUT vLo AS Integer = yLo
  MUT i AS Integer = 0
  WHILE i < 128
    MUT bit AS Integer = 0
    IF i < 64 THEN
      bit = bits::band(bits::sr(xHi, 63 - i), 1)
    ELSE
      bit = bits::band(bits::sr(xLo, 127 - i), 1)
    END IF
    IF bit <> 0 THEN
      zHi = bits::bxor(zHi, vHi)
      zLo = bits::bxor(zLo, vLo)
    END IF
    LET lsb AS Integer = bits::band(vLo, 1)
    ' v = v >> 1 across the 128-bit pair.
    LET carry AS Integer = bits::sl(bits::band(vHi, 1), 63)
    vLo = bits::bor(bits::sr(vLo, 1), carry)
    vHi = bits::sr(vHi, 1)
    IF lsb <> 0 THEN
      ' XOR the reduction polynomial R = 0xe1 << 120 into the high half
      ' (0xe1 in the top byte of the 128-bit value → 0xe1 << 56 in the hi word).
      vHi = bits::bxor(vHi, bits::sl(225, 56))
    END IF
    i = i + 1
  END WHILE
  MUT out AS List OF Integer = []
  out = collections::append(out, zHi)
  out = collections::append(out, zLo)
  RETURN out
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_ghashMul", BODY));
}
