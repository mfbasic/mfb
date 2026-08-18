//! `__crypto_aesExpandKey` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Expand a 32-byte AES-256 key into 240 round-key bytes (15 round keys).
FUNC __crypto_aesExpandKey(key AS List OF Byte) AS List OF Byte
  MUT w AS List OF Byte = __crypto_copyBytes(key)
  LET rcon AS List OF Byte = encoding::hexDecode("01020408102040")
  MUT i AS Integer = 32
  MUT rc AS Integer = 0
  WHILE i < 240
    LET t0i AS Integer = i - 4
    MUT a0 AS Integer = toInt(collections::get(w, t0i))
    MUT a1 AS Integer = toInt(collections::get(w, t0i + 1))
    MUT a2 AS Integer = toInt(collections::get(w, t0i + 2))
    MUT a3 AS Integer = toInt(collections::get(w, t0i + 3))
    LET pos AS Integer = i / 4
    IF (pos MOD 8) = 0 THEN
      ' RotWord then SubWord then XOR Rcon.
      LET tmp AS Integer = a0
      a0 = __crypto_aesSub(a1)
      a1 = __crypto_aesSub(a2)
      a2 = __crypto_aesSub(a3)
      a3 = __crypto_aesSub(tmp)
      a0 = bits::bxor(a0, toInt(collections::get(rcon, rc)))
      rc = rc + 1
    ELSE
      IF (pos MOD 8) = 4 THEN
        a0 = __crypto_aesSub(a0)
        a1 = __crypto_aesSub(a1)
        a2 = __crypto_aesSub(a2)
        a3 = __crypto_aesSub(a3)
      END IF
    END IF
    LET p0 AS Integer = i - 32
    w = collections::append(w, toByte(bits::bxor(a0, toInt(collections::get(w, p0)))))
    w = collections::append(w, toByte(bits::bxor(a1, toInt(collections::get(w, p0 + 1)))))
    w = collections::append(w, toByte(bits::bxor(a2, toInt(collections::get(w, p0 + 2)))))
    w = collections::append(w, toByte(bits::bxor(a3, toInt(collections::get(w, p0 + 3)))))
    i = i + 4
  END WHILE
  RETURN w
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_aesExpandKey", BODY));
}
