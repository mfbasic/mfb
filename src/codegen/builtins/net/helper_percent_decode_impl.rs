//! `__net_percentDecodeImpl` — shared private helper for the `net` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Core percent-decoder: `%XX` escapes become bytes; when `plusSpace` is TRUE a
' literal `+` becomes a space (query semantics). Every other grapheme keeps its
' UTF-8 bytes. The accumulated bytes are UTF-8 validated by `toString`.
' Implemented with only `strings`/`collections` so `net` carries no extra
' package dependency. A function-level TRAP maps every failure — a truncated or
' non-hex escape, or a non-UTF-8 decoded result (`toString`'s ErrEncoding, which
' the inline-TRAP analysis cannot catch) — to a single `ErrInvalidFormat`.
FUNC __net_percentDecodeImpl(s AS String, plusSpace AS Boolean) AS String
  MUT out AS List OF Byte = []
  LET n AS Integer = len(s)
  MUT i AS Integer = 0
  WHILE i < n
    LET c AS String = strings::mid(s, i, 1)
    IF c = "%" THEN
      IF i + 2 >= n THEN
        FAIL error(77050003, "truncated percent-escape")
      END IF
      out = collections::append(out, toByte(toInt(strings::mid(s, i + 1, 2), 16)))
      i = i + 3
    ELSEIF c = "+" AND plusSpace = TRUE THEN
      out = collections::append(out, toByte(32))
      i = i + 1
    ELSE
      out = collections::append(out, strings::toBytes(c))
      i = i + 1
    END IF
  END WHILE
  RETURN toString(out)
  TRAP(e)
    FAIL error(77050003, "invalid percent-encoding")
  END TRAP
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("net_percentDecodeImpl", BODY));
}
