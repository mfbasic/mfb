//! `__crypto_keccakSponge` — shared private helper for the `crypto` package.
//!
//! The Keccak sponge (FIPS 202 §4) over `__crypto_keccakF`: absorb the message a
//! `rateLanes × 8`-byte block at a time by XORing its little-endian lanes into the
//! state and permuting, then squeeze `outLen` bytes a block at a time, permuting
//! between blocks. Whole blocks are read straight out of `data`; only the final
//! block — the shorter-than-a-block tail, the domain `suffix` (`0x06` SHA-3,
//! `0x1f` SHAKE) and `pad10*1` — is built, so the message is never copied
//! (plan-142-E: this used to copy all of `data` into a padded buffer first). The only data-dependent quantities are the
//! PUBLIC message and output lengths; no branch or index depends on message or
//! state contents.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Keccak sponge: absorb `data` at `rateLanes` lanes per block under domain `suffix`
' with pad10*1, then squeeze `outLen` bytes (FIPS 202 §4, §5).
FUNC __crypto_keccakSponge(data AS List OF Byte, rateLanes AS Integer, suffix AS Integer, outLen AS Integer) AS List OF Byte
  LET rate AS Integer = rateLanes * 8
  LET n AS Integer = len(data)
  MUT state AS List OF Integer = __crypto_keccakZero()
  MUT off AS Integer = 0
  WHILE n - off >= rate
    MUT lane AS Integer = 0
    WHILE lane < rateLanes
      LET mixed AS Integer = bits::bxor(collections::get(state, lane), __crypto_leLane(data, off + lane * 8))
      state = collections::set(state, lane, mixed)
      lane = lane + 1
    END WHILE
    state = __crypto_keccakF(state)
    off = off + rate
  END WHILE
  MUT tail AS List OF Byte = __crypto_slice(data, off, n)
  tail = collections::append(tail, toByte(suffix))
  WHILE len(tail) < rate
    tail = collections::append(tail, toByte(0))
  END WHILE
  LET last AS Integer = rate - 1
  tail = collections::set(tail, last, toByte(bits::bor(toInt(collections::get(tail, last)), 128)))
  MUT fl AS Integer = 0
  WHILE fl < rateLanes
    LET mixedTail AS Integer = bits::bxor(collections::get(state, fl), __crypto_leLane(tail, fl * 8))
    state = collections::set(state, fl, mixedTail)
    fl = fl + 1
  END WHILE
  state = __crypto_keccakF(state)
  MUT out AS List OF Byte = []
  WHILE len(out) < outLen
    MUT sq AS Integer = 0
    WHILE sq < rateLanes
      out = __crypto_appendLeLane(out, collections::get(state, sq))
      sq = sq + 1
    END WHILE
    IF len(out) < outLen THEN
      state = __crypto_keccakF(state)
    END IF
  END WHILE
  RETURN __crypto_truncate(out, outLen)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_keccakSponge", BODY));
}
