//! `__json_escapeString` — shared private helper for the `json` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' plan-120-C: `/` is NOT escaped. RFC 8259 permits `\/` but never requires it,
' and no other JSON writer emits it -- Node's JSON.stringify("a/b") is "a/b".
' Emitting it made every MFB document differ byte-for-byte from every other
' producer's for no gain. Parsing still ACCEPTS `\/`, since it is valid input.
'
' `/` needs no arm of its own to fall through: it is U+002F = 47, so the C0
' arms below cannot match it and `__json_isRawControlChar` (single scalar < 32)
' answers FALSE, leaving the ELSE pass-through.
FUNC __json_escapeString(value AS String) AS String
  MUT out AS String = ""
  FOR EACH ch IN strings::graphemes(value)
    IF ch = "\"" THEN
      out = out & "\\\""
    ELSEIF ch = "\\" THEN
      out = out & "\\\\"
    ELSEIF ch = "\n" THEN
      out = out & "\\n"
    ELSEIF ch = "\t" THEN
      out = out & "\\t"
    ELSEIF ch = "\r" THEN
      out = out & "\\r"
    ELSEIF ch = "\u{8}" THEN
      out = out & "\\b"
    ELSEIF ch = "\u{C}" THEN
      out = out & "\\f"
    ELSEIF __json_isRawControlChar(ch) THEN
      out = out & __json_escapeRawControlChar(ch)
    ELSE
      out = out & ch
    END IF
  NEXT
  RETURN out
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("json_escapeString", BODY));
}
