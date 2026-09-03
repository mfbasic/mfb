//! `__json_indentFromCount` / `__json_indentFromText` — shared private helpers.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const COUNT_BODY: &str =
r#"' plan-120-D: JavaScript's clamp for the NUMBER form of `space`. Anything above
' 10 becomes 10 and anything at or below 0 becomes 0 (which the caller then
' renders compactly). Copied deliberately rather than invented: this letter's
' whole specification is "Node's layout, exactly", and a program that hard-codes
' 11 should get the same 10-space indent from both languages.
FUNC __json_indentFromCount(count AS Integer) AS String
  IF count <= 0 THEN
    RETURN ""
  END IF
  IF count > 10 THEN
    RETURN strings::repeat(" ", 10)
  END IF
  RETURN strings::repeat(" ", count)
END FUNC"#;

#[rustfmt::skip]
const TEXT_BODY: &str =
r#"' plan-120-D: JavaScript's clamp for the STRING form of `space` -- the first 10
' characters, the rest discarded.
'
' The unit is SCALARS, matching `strings::mid`, which slices by scalar and
' requires `start + count` not to exceed the scalar length -- so the guard must
' count the same thing it slices, or a text that is 11 graphemes but 10 scalars
' would reach a `mid` that raises. Scalars are also the closer match to Node,
' which truncates by UTF-16 code unit: the two are identical for every character
' in the BMP, which is every indent anyone writes (spaces, tabs, dashes, dots).
FUNC __json_indentFromText(text AS String) AS String
  LET scalars AS List OF Integer = encoding::utf32Encode(text)
  IF len(scalars) <= 10 THEN
    RETURN text
  END IF
  RETURN strings::mid(text, 0, 10)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("json_indentFromCount", COUNT_BODY));
    pkg.add_helper(RegistryHelper::always("json_indentFromText", TEXT_BODY));
}
