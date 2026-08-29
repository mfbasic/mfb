//! `__crypto_ed448Sign` — shared private helper for the `crypto` package.
//!
//! RFC 8032 §5.2.6 PureEd448 signing with the empty context: expand the seed
//! with SHAKE256 into the pruned scalar `s` and the 57-byte `prefix`;
//! `r = SHAKE256(dom4 ‖ prefix ‖ M) mod L`; `R = [r]B`;
//! `k = SHAKE256(dom4 ‖ R ‖ A ‖ M) mod L`; `S = (r + k·s) mod L`; signature
//! `R ‖ S` (57 + 57 bytes). Deterministic — no randomness beyond the seed. The
//! scalar arithmetic is fixed-length byte-limb work with masked reductions and
//! the point work is the select-swap ladder, so nothing branches on the secret.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __crypto_ed448Sign(privateKey AS List OF Byte, message AS List OF Byte) AS List OF Byte
  IF len(privateKey) <> 57 THEN
    FAIL error(77050002, "ed448 private key must be 57 bytes")
  END IF
  LET h AS List OF Byte = __crypto_shake256(privateKey, 114)
  LET s AS List OF Byte = __crypto_ed448Prune(__crypto_truncate(h, 57))
  LET prefix AS List OF Byte = __crypto_slice(h, 57, 114)
  LET pub AS List OF Byte = __crypto_ed448Encode(__crypto_ed448Scalarmult(__CRYPTO_ED448_B, s))
  LET dom AS List OF Byte = __crypto_ed448Dom()
  LET rInput AS List OF Byte = __crypto_concat(__crypto_concat(dom, prefix), message)
  LET r AS List OF Byte = __crypto_ed448ModL(__crypto_bytesToLimbs(__crypto_shake256(rInput, 114)))
  LET bigR AS List OF Byte = __crypto_ed448Encode(__crypto_ed448Scalarmult(__CRYPTO_ED448_B, r))
  MUT kInput AS List OF Byte = __crypto_concat(__crypto_concat(dom, bigR), pub)
  kInput = __crypto_concat(kInput, message)
  LET k AS List OF Byte = __crypto_ed448ModL(__crypto_bytesToLimbs(__crypto_shake256(kInput, 114)))
  LET ks AS List OF Integer = __crypto_bnMul(__crypto_bytesToLimbs(k), __crypto_bytesToLimbs(s))
  LET bigS AS List OF Byte = __crypto_ed448ModL(__crypto_bnAdd(ks, __crypto_bytesToLimbs(r)))
  RETURN __crypto_concat(bigR, bigS)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_ed448Sign", BODY));
}
