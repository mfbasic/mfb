//! `__crypto_gcmGctr` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' GCTR: CTR-mode XOR of `data` with the AES keystream from counter block `ctr`.
FUNC __crypto_gcmGctr(roundKeys AS List OF Byte, ctr AS List OF Byte, data AS List OF Byte) AS List OF Byte
  MUT out AS List OF Byte = []
  LET n AS Integer = len(data)
  MUT counter AS List OF Byte = __crypto_copyBytes(ctr)
  MUT offset AS Integer = 0
  WHILE offset < n
    LET ks AS List OF Byte = __crypto_aesEncryptBlock(roundKeys, counter)
    MUT j AS Integer = 0
    WHILE j < 16 AND (offset + j) < n
      LET v AS Integer = bits::bxor(toInt(collections::get(data, offset + j)), toInt(collections::get(ks, j)))
      out = collections::append(out, toByte(v))
      j = j + 1
    END WHILE
    counter = __crypto_gcmInc32(counter)
    offset = offset + 16
  END WHILE
  RETURN out
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_gcmGctr", BODY));
}
