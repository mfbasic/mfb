//! `__COMPRESS_CRC32_TABLES` — the slicing-by-8 lookup tables for `compress::crc32`.
//!
//! Eight 256-entry tables for the reflected CRC-32 polynomial `0xEDB88320`, flattened
//! into one module-level list so every lookup is a single `collections::get`: table
//! `k` occupies indices `k*256 .. k*256+255`. Table 0 is the classic byte table;
//! table `k` is table `k-1` advanced by one more zero byte.
//!
//! The list is computed once at program start by `__compress_crc32Tables`, not written
//! as a 2,048-element literal: a list literal lowers to about 26 instructions per
//! element in the global initializer, and a probe program with the literal was
//! 379,772 B larger than the same program with this builder, at the same crc32 speed
//! (plan-137-A Corrections). The builder's one constant is pinned by
//! `tests::crc32_table_builder_uses_the_reflected_polynomial` in `compress/mod.rs`;
//! every entry is judged by `tests/interop/rt_compress_interop.rs` and
//! `tools/oracles/compress` against independent CRC-32 implementations over random data
//! that reaches every index.
//!
//! Registered via `add_helper` under [`HelperGate::WhenUsed`] on `crc32`. A gated helper
//! is injected as its own file, so the body carries its own `IMPORT`s.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{HelperGate, RegistryHelper, RegistryPackage};

#[rustfmt::skip]
pub(super) const BODY: &str =
r#"IMPORT compress
IMPORT bits
IMPORT collections

' Slicing-by-8 CRC-32 tables for the reflected polynomial 0xEDB88320: table 0 (indices
' 0..255) is the byte table, and table k (from index k*256) is table k-1 advanced by
' one zero byte.
FUNC __compress_crc32Tables() AS List OF Integer
  MUT t AS List OF Integer = []
  MUT i AS Integer = 0
  WHILE i < 256
    MUT c AS Integer = i
    MUT k AS Integer = 0
    WHILE k < 8
      IF bits::band(c, 1) = 1 THEN
        c = bits::bxor(bits::sr(c, 1), 3988292384)
      ELSE
        c = bits::sr(c, 1)
      END IF
      k = k + 1
    END WHILE
    t = collections::append(t, c)
    i = i + 1
  END WHILE
  WHILE i < 2048
    LET prev AS Integer = collections::get(t, i - 256)
    t = collections::append(t, bits::bxor(bits::sr(prev, 8), collections::get(t, bits::band(prev, 255))))
    i = i + 1
  END WHILE
  RETURN t
END FUNC

LET __COMPRESS_CRC32_TABLES AS List OF Integer = __compress_crc32Tables()"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper {
        name: "compress_crc32_table",
        gate: HelperGate::WhenUsed(&["crc32", "gzipDecode", "gzipEncode"]),
        body: Some(BODY),
        import_name: None,
        natively_called: false,
    });
}
