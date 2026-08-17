//! `__crypto_ed25519Verify` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __crypto_ed25519Verify(publicKey AS List OF Byte, message AS List OF Byte, signature AS List OF Byte) AS Boolean
  IF len(publicKey) <> 32 THEN
    RETURN FALSE
  END IF
  IF len(signature) <> 64 THEN
    RETURN FALSE
  END IF
  LET negRes AS List OF Integer = __crypto_unpackneg(publicKey)
  IF collections::get(negRes, 0) = 0 THEN
    RETURN FALSE
  END IF
  MUT q AS List OF Integer = []
  MUT i AS Integer = 1
  WHILE i < 65
    q = collections::append(q, collections::get(negRes, i))
    i = i + 1
  END WHILE
  LET bigR AS List OF Byte = __crypto_truncate(signature, 32)
  LET bigS AS List OF Byte = __crypto_slice(signature, 32, 64)
  ' bug-269 / CRY-02: reject a non-canonical S (S >= L) before verifying, so a
  ' malleated signature cannot verify against the same message.
  IF __crypto_scalarBelowL(bigS) = FALSE THEN
    RETURN FALSE
  END IF
  MUT hInput AS List OF Byte = __crypto_concat(bigR, publicKey)
  hInput = __crypto_concat(hInput, message)
  LET h AS List OF Byte = __crypto_reduce(__crypto_sha512_bytes(hInput))
  LET pMul AS List OF Integer = __crypto_scalarmult(q, h)
  LET qMul AS List OF Integer = __crypto_scalarbase(bigS)
  LET pSum AS List OF Integer = __crypto_edAdd(pMul, qMul)
  LET tCheck AS List OF Byte = __crypto_packPoint(pSum)
  RETURN __crypto_constantTimeEqual(bigR, tCheck)
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_ed25519Verify", BODY));
}
