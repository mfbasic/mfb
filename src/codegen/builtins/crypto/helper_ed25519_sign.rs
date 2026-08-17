//! `__crypto_ed25519Sign` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __crypto_ed25519Sign(privateKey AS List OF Byte, message AS List OF Byte) AS List OF Byte
  IF len(privateKey) <> 32 THEN
    FAIL error(77050002, "ed25519 private key must be 32 bytes")
  END IF
  LET d AS List OF Byte = __crypto_sha512_bytes(privateKey)
  LET a AS List OF Byte = __crypto_clampScalar(__crypto_truncate(d, 32))
  LET prefix AS List OF Byte = __crypto_slice(d, 32, 64)
  LET pub AS List OF Byte = __crypto_ed25519Public(privateKey)
  LET rInput AS List OF Byte = __crypto_concat(prefix, message)
  LET r AS List OF Byte = __crypto_reduce(__crypto_sha512_bytes(rInput))
  LET rPoint AS List OF Integer = __crypto_scalarbase(r)
  LET bigR AS List OF Byte = __crypto_packPoint(rPoint)
  MUT hInput AS List OF Byte = __crypto_concat(bigR, pub)
  hInput = __crypto_concat(hInput, message)
  LET h AS List OF Byte = __crypto_reduce(__crypto_sha512_bytes(hInput))
  MUT x AS List OF Integer = []
  MUT i AS Integer = 0
  WHILE i < 64
    IF i < 32 THEN
      x = collections::append(x, toInt(collections::get(r, i)))
    ELSE
      x = collections::append(x, 0)
    END IF
    i = i + 1
  END WHILE
  i = 0
  WHILE i < 32
    MUT j AS Integer = 0
    WHILE j < 32
      LET idx AS Integer = i + j
      LET add AS Integer = toInt(collections::get(h, i)) * toInt(collections::get(a, j))
      x = collections::set(x, idx, collections::get(x, idx) + add)
      j = j + 1
    END WHILE
    i = i + 1
  END WHILE
  LET bigS AS List OF Byte = __crypto_modL(x)
  RETURN __crypto_concat(bigR, bigS)
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_ed25519Sign", BODY));
}
