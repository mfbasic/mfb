//! `__compress_adler32` — the Adler-32 checksum (RFC 1950 §8.2) that ends a zlib stream.
//!
//! `a` starts at 1 and sums the bytes, `b` sums the running `a`, both modulo 65521; the result is
//! `b * 65536 + a`. The sums are reduced every 5,552 bytes, as zlib's `NMAX` does, which keeps every
//! intermediate far below 2^63. Only `zlibDecode` computes it, and not at all when the caller passes
//! `ignoreChecksum := TRUE` (plan-137-B §4.3).
//!
//! Registered via `add_helper` under [`HelperGate::WhenUsed`] on `zlibDecode`. A gated helper is
//! injected as its own file, so the body carries its own `IMPORT`s.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{HelperGate, RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"IMPORT compress
IMPORT collections

' Adler-32 of `data` (RFC 1950 8.2), reduced every 5552 bytes like zlib's NMAX.
FUNC __compress_adler32(data AS List OF Byte) AS Integer
  LET n AS Integer = len(data)
  MUT a AS Integer = 1
  MUT b AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < n
    MUT stop AS Integer = i + 5552
    IF stop > n THEN
      stop = n
    END IF
    WHILE i < stop
      a = a + toInt(collections::get(data, i))
      b = b + a
      i = i + 1
    END WHILE
    a = a MOD 65521
    b = b MOD 65521
  END WHILE
  RETURN b * 65536 + a
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper {
        name: "compress_adler32",
        gate: HelperGate::WhenUsed(&["zlibDecode"]),
        body: Some(BODY),
        import_name: None,
        natively_called: false,
    });
}
