//! `__json_parseString` — shared private helper for the `json` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Iterative string scan (plan-25-D §D3), over bytes since bug-510 (DEC-03). The
' grapheme list it replaced cost ~42 bytes per character plus a one-character
' String per chunk -- over 400 MB for a 3 MB string.
'
' It accumulates by RUN, not by element. A per-byte `out = append(out, b)` into a
' `MUT List OF Byte` was measured at ~65 bytes of peak RSS per element (195 MB
' for 3 000 000 bytes, and identically for a `List OF Integer`, so it is the
' growth churn and not the element width): a list built by repeated append is the
' wrong accumulator at document scale. A run of unescaped bytes is instead copied
' out in ONE `collections::mid` and concatenated onto a `MUT String`, which takes
' the in-place concat fast path. A string with no escapes therefore costs one
' slice and one concat however long it is, and one with escapes costs a bounded
' amount per escape.
'
' Cutting the run at `"` or `\` -- both ASCII -- never splits a UTF-8 scalar, and
' the source is a String, so every run is well-formed UTF-8 by construction.
FUNC __json_parseString(bytes AS List OF Byte, index AS Integer) AS __json_StringNode
  MUT acc AS String = ""
  MUT runStart AS Integer = index
  MUT idx AS Integer = index
  MUT finished AS Boolean = FALSE
  MUT endIndex AS Integer = index
  MUT code AS Integer = 0
  LET n AS Integer = len(bytes)
  WHILE finished = FALSE
    IF idx >= n THEN
      FAIL error(77050003, "invalid JSON format")
    END IF
    code = toInt(collections::get(bytes, idx))
    IF code = 34 THEN
      IF idx > runStart THEN
        LET run AS String = encoding::utf8Decode(collections::mid(bytes, runStart, idx - runStart))
        acc = acc & run
      END IF
      finished = TRUE
      endIndex = idx + 1
    ELSEIF code = 92 THEN
      IF idx + 1 >= n THEN
        FAIL error(77050003, "invalid JSON format")
      END IF
      IF idx > runStart THEN
        LET run AS String = encoding::utf8Decode(collections::mid(bytes, runStart, idx - runStart))
        acc = acc & run
      END IF
      LET escapeState AS __json_StringNode = __json_parseEscape(bytes, idx + 1)
      LET decoded AS String = escapeState.value
      acc = acc & decoded
      idx = escapeState.index
      runStart = idx
    ELSEIF code < 32 THEN
      ' A raw control character is a single Unicode scalar below 32, which is one
      ' ASCII byte -- so the byte test is exactly the scalar test JSON requires.
      FAIL error(77050003, "invalid JSON format")
    ELSE
      idx = idx + 1
    END IF
  END WHILE
  RETURN __json_StringNode[acc, endIndex]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("json_parseString", BODY));
}
