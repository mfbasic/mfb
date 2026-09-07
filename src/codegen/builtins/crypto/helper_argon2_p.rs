//! `__crypto_argon2P` — shared private helper for the `crypto` package.
//!
//! Argon2's permutation P (RFC 9106 figures 17-19): the eight GB steps of a BLAKE2b
//! round over sixteen 64-bit words, written out over locals so the mixing does not
//! round-trip through a list per step.
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

' Argon2's permutation P (RFC 9106 figures 18 and 19): the eight GB steps of a
' BLAKE2b round over 16 64-bit words, written out so the mixing runs on locals.
FUNC __crypto_argon2P(v AS List OF Integer) AS List OF Integer
  MUT v0 AS Integer = collections::get(v, 0)
  MUT v1 AS Integer = collections::get(v, 1)
  MUT v2 AS Integer = collections::get(v, 2)
  MUT v3 AS Integer = collections::get(v, 3)
  MUT v4 AS Integer = collections::get(v, 4)
  MUT v5 AS Integer = collections::get(v, 5)
  MUT v6 AS Integer = collections::get(v, 6)
  MUT v7 AS Integer = collections::get(v, 7)
  MUT v8 AS Integer = collections::get(v, 8)
  MUT v9 AS Integer = collections::get(v, 9)
  MUT v10 AS Integer = collections::get(v, 10)
  MUT v11 AS Integer = collections::get(v, 11)
  MUT v12 AS Integer = collections::get(v, 12)
  MUT v13 AS Integer = collections::get(v, 13)
  MUT v14 AS Integer = collections::get(v, 14)
  MUT v15 AS Integer = collections::get(v, 15)
  v0 = __crypto_argon2MixAdd(v0, v4)
  v12 = bits::rr64(bits::bxor(v12, v0), 32)
  v8 = __crypto_argon2MixAdd(v8, v12)
  v4 = bits::rr64(bits::bxor(v4, v8), 24)
  v0 = __crypto_argon2MixAdd(v0, v4)
  v12 = bits::rr64(bits::bxor(v12, v0), 16)
  v8 = __crypto_argon2MixAdd(v8, v12)
  v4 = bits::rr64(bits::bxor(v4, v8), 63)
  v1 = __crypto_argon2MixAdd(v1, v5)
  v13 = bits::rr64(bits::bxor(v13, v1), 32)
  v9 = __crypto_argon2MixAdd(v9, v13)
  v5 = bits::rr64(bits::bxor(v5, v9), 24)
  v1 = __crypto_argon2MixAdd(v1, v5)
  v13 = bits::rr64(bits::bxor(v13, v1), 16)
  v9 = __crypto_argon2MixAdd(v9, v13)
  v5 = bits::rr64(bits::bxor(v5, v9), 63)
  v2 = __crypto_argon2MixAdd(v2, v6)
  v14 = bits::rr64(bits::bxor(v14, v2), 32)
  v10 = __crypto_argon2MixAdd(v10, v14)
  v6 = bits::rr64(bits::bxor(v6, v10), 24)
  v2 = __crypto_argon2MixAdd(v2, v6)
  v14 = bits::rr64(bits::bxor(v14, v2), 16)
  v10 = __crypto_argon2MixAdd(v10, v14)
  v6 = bits::rr64(bits::bxor(v6, v10), 63)
  v3 = __crypto_argon2MixAdd(v3, v7)
  v15 = bits::rr64(bits::bxor(v15, v3), 32)
  v11 = __crypto_argon2MixAdd(v11, v15)
  v7 = bits::rr64(bits::bxor(v7, v11), 24)
  v3 = __crypto_argon2MixAdd(v3, v7)
  v15 = bits::rr64(bits::bxor(v15, v3), 16)
  v11 = __crypto_argon2MixAdd(v11, v15)
  v7 = bits::rr64(bits::bxor(v7, v11), 63)
  v0 = __crypto_argon2MixAdd(v0, v5)
  v15 = bits::rr64(bits::bxor(v15, v0), 32)
  v10 = __crypto_argon2MixAdd(v10, v15)
  v5 = bits::rr64(bits::bxor(v5, v10), 24)
  v0 = __crypto_argon2MixAdd(v0, v5)
  v15 = bits::rr64(bits::bxor(v15, v0), 16)
  v10 = __crypto_argon2MixAdd(v10, v15)
  v5 = bits::rr64(bits::bxor(v5, v10), 63)
  v1 = __crypto_argon2MixAdd(v1, v6)
  v12 = bits::rr64(bits::bxor(v12, v1), 32)
  v11 = __crypto_argon2MixAdd(v11, v12)
  v6 = bits::rr64(bits::bxor(v6, v11), 24)
  v1 = __crypto_argon2MixAdd(v1, v6)
  v12 = bits::rr64(bits::bxor(v12, v1), 16)
  v11 = __crypto_argon2MixAdd(v11, v12)
  v6 = bits::rr64(bits::bxor(v6, v11), 63)
  v2 = __crypto_argon2MixAdd(v2, v7)
  v13 = bits::rr64(bits::bxor(v13, v2), 32)
  v8 = __crypto_argon2MixAdd(v8, v13)
  v7 = bits::rr64(bits::bxor(v7, v8), 24)
  v2 = __crypto_argon2MixAdd(v2, v7)
  v13 = bits::rr64(bits::bxor(v13, v2), 16)
  v8 = __crypto_argon2MixAdd(v8, v13)
  v7 = bits::rr64(bits::bxor(v7, v8), 63)
  v3 = __crypto_argon2MixAdd(v3, v4)
  v14 = bits::rr64(bits::bxor(v14, v3), 32)
  v9 = __crypto_argon2MixAdd(v9, v14)
  v4 = bits::rr64(bits::bxor(v4, v9), 24)
  v3 = __crypto_argon2MixAdd(v3, v4)
  v14 = bits::rr64(bits::bxor(v14, v3), 16)
  v9 = __crypto_argon2MixAdd(v9, v14)
  v4 = bits::rr64(bits::bxor(v4, v9), 63)
  MUT out AS List OF Integer = collections::mid(v, 0, 16)
  out = collections::set(out, 0, v0)
  out = collections::set(out, 1, v1)
  out = collections::set(out, 2, v2)
  out = collections::set(out, 3, v3)
  out = collections::set(out, 4, v4)
  out = collections::set(out, 5, v5)
  out = collections::set(out, 6, v6)
  out = collections::set(out, 7, v7)
  out = collections::set(out, 8, v8)
  out = collections::set(out, 9, v9)
  out = collections::set(out, 10, v10)
  out = collections::set(out, 11, v11)
  out = collections::set(out, 12, v12)
  out = collections::set(out, 13, v13)
  out = collections::set(out, 14, v14)
  out = collections::set(out, 15, v15)
  RETURN out
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper {
        name: "crypto_argon2P",
        gate: HelperGate::WhenUsed(&["argon2id"]),
        body: Some(BODY),
        import_name: None,
        natively_called: false,
    });
}
