//! `__COMPRESS_LEN_BASE` / `__COMPRESS_LEN_EXTRA` / `__COMPRESS_DIST_BASE` /
//! `__COMPRESS_DIST_EXTRA` — the RFC 1951 §3.2.5 length and distance tables.
//!
//! Length codes 257..285 (29 entries) and distance codes 0..29 (30 entries): the base value
//! each code starts at and the number of extra bits read after it. Built once at program
//! start from the §3.2.5 formula rather than written as literals (a list literal lowers to
//! per-element initializer code; plan-137-A Corrections). The formula's constants are pinned
//! against the RFC table, transcribed from the fetched text, by
//! `tests::deflate_tables_builder_matches_rfc1951` in `compress/mod.rs`.
//!
//! Registered via `add_helper` under [`HelperGate::WhenUsed`] on the decoders. A gated helper
//! is injected as its own file, so the body carries its own `IMPORT`s.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{HelperGate, RegistryHelper, RegistryPackage};

#[rustfmt::skip]
pub(super) const BODY: &str =
r#"IMPORT compress
IMPORT bits
IMPORT collections

' RFC 1951 3.2.5 length codes 257..285: codes 257..264 have no extra bits, then every four
' codes add one, and code 285 is the single length 258.
FUNC __compress_lengthTable(wantBase AS Boolean) AS List OF Integer
  MUT t AS List OF Integer = []
  MUT base AS Integer = 3
  MUT i AS Integer = 0
  WHILE i < 28
    MUT extra AS Integer = 0
    IF i >= 8 THEN
      extra = (i - 4) / 4
    END IF
    IF wantBase THEN
      t = collections::append(t, base)
    ELSE
      t = collections::append(t, extra)
    END IF
    base = base + bits::sl(1, extra)
    i = i + 1
  END WHILE
  IF wantBase THEN
    t = collections::append(t, 258)
  ELSE
    t = collections::append(t, 0)
  END IF
  RETURN t
END FUNC

' RFC 1951 3.2.5 distance codes 0..29: codes 0..3 have no extra bits, then every two codes
' add one.
FUNC __compress_distanceTable(wantBase AS Boolean) AS List OF Integer
  MUT t AS List OF Integer = []
  MUT base AS Integer = 1
  MUT i AS Integer = 0
  WHILE i < 30
    MUT extra AS Integer = 0
    IF i >= 4 THEN
      extra = (i - 2) / 2
    END IF
    IF wantBase THEN
      t = collections::append(t, base)
    ELSE
      t = collections::append(t, extra)
    END IF
    base = base + bits::sl(1, extra)
    i = i + 1
  END WHILE
  RETURN t
END FUNC

LET __COMPRESS_LEN_BASE AS List OF Integer = __compress_lengthTable(TRUE)
LET __COMPRESS_LEN_EXTRA AS List OF Integer = __compress_lengthTable(FALSE)
LET __COMPRESS_DIST_BASE AS List OF Integer = __compress_distanceTable(TRUE)
LET __COMPRESS_DIST_EXTRA AS List OF Integer = __compress_distanceTable(FALSE)
' RFC 1951 3.2.7: the order code-length code lengths are sent in.
LET __COMPRESS_CL_ORDER AS List OF Integer = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15]"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper {
        name: "compress_deflate_tables",
        gate: HelperGate::WhenUsed(&["inflate", "zlibDecode", "gzipDecode"]),
        body: Some(BODY),
        import_name: None,
        natively_called: false,
    });
}
