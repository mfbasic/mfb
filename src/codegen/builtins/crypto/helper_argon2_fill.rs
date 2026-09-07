//! `__crypto_argon2Fill` — shared private helper for the `crypto` package.
//!
//! Argon2's compression function G (RFC 9106 §3.5) over 1 KiB blocks of the caller's
//! block matrix, addressed by word offset: P applied to each of the eight rows and then
//! each of the eight columns of `mem[prevOff] XOR mem[refOff]`. Taking offsets into
//! `mem` rather than three sliced blocks keeps every working value of a block fill
//! inside this call, which matters because the core runs millions of fills without
//! returning.
//!
//! Registered via `add_helper` under [`HelperGate::WhenUsed`] on `argon2id`, so a
//! program that imports `crypto` without calling `crypto::argon2id` carries none of
//! the BLAKE2b/Argon2 source. Renders in the helper section of the assembled source.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{HelperGate, RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"IMPORT bits
IMPORT collections

' Argon2's compression function G (RFC 9106 section 3.5) over three 1 KiB blocks of
' `mem`, addressed by word offset: R = mem[prevOff] XOR mem[refOff], P applied to each
' of the eight rows and then each of the eight columns, and the result XORed back over
' R (and over mem[curOff] when `withXor`, the second and later passes). Reading the
' operands out of `mem` here rather than slicing them at the call site is what keeps
' the per-block working values inside this call.
FUNC __crypto_argon2Fill(mem AS List OF Integer, prevOff AS Integer, refOff AS Integer, curOff AS Integer, withXor AS Boolean) AS List OF Integer
  MUT r AS List OF Integer = collections::mid(mem, prevOff, 128)
  MUT i AS Integer = 0
  WHILE i < 128
    r = collections::set(r, i, bits::bxor(collections::get(r, i), collections::get(mem, refOff + i)))
    i = i + 1
  END WHILE
  MUT tmp AS List OF Integer = collections::mid(r, 0, 128)
  IF withXor THEN
    i = 0
    WHILE i < 128
      tmp = collections::set(tmp, i, bits::bxor(collections::get(tmp, i), collections::get(mem, curOff + i)))
      i = i + 1
    END WHILE
  END IF
  MUT g AS Integer = 0
  MUT k AS Integer = 0
  MUT reg AS List OF Integer = collections::mid(r, 0, 16)
  WHILE g < 8
    reg = __crypto_argon2P(collections::mid(r, g * 16, 16))
    k = 0
    WHILE k < 16
      r = collections::set(r, g * 16 + k, collections::get(reg, k))
      k = k + 1
    END WHILE
    g = g + 1
  END WHILE
  g = 0
  WHILE g < 8
    k = 0
    WHILE k < 8
      reg = collections::set(reg, 2 * k, collections::get(r, 2 * g + 16 * k))
      reg = collections::set(reg, 2 * k + 1, collections::get(r, 2 * g + 16 * k + 1))
      k = k + 1
    END WHILE
    reg = __crypto_argon2P(reg)
    k = 0
    WHILE k < 8
      r = collections::set(r, 2 * g + 16 * k, collections::get(reg, 2 * k))
      r = collections::set(r, 2 * g + 16 * k + 1, collections::get(reg, 2 * k + 1))
      k = k + 1
    END WHILE
    g = g + 1
  END WHILE
  i = 0
  WHILE i < 128
    tmp = collections::set(tmp, i, bits::bxor(collections::get(tmp, i), collections::get(r, i)))
    i = i + 1
  END WHILE
  RETURN tmp
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper {
        name: "crypto_argon2Fill",
        gate: HelperGate::WhenUsed(&["argon2id"]),
        body: Some(BODY),
        import_name: None,
        natively_called: false,
    });
}
