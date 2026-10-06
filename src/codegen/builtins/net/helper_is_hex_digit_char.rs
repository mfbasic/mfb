//! `__net_isHexDigitChar` — shared private helper for the `net` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-868: TRUE if `ch` is a single ASCII hex digit (0-9, a-f, A-F).
' `__net_percentDecodeImpl` uses this to validate both characters of a `%XX`
' escape BEFORE handing them to `toInt(.., 16)`, whose signed radix parse
' otherwise accepts a leading `+`/`-` — "%+5" parsed as hex 5, "%+A" as 0x0A
' (a line feed) — neither of which is a valid RFC 3986 `pct-encoded` escape.
FUNC __net_isHexDigitChar(ch AS String) AS Boolean
  IF len(ch) <> 1 THEN
    RETURN FALSE
  END IF
  RETURN strings::contains("0123456789abcdefABCDEF", ch)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("net_isHexDigitChar", BODY));
}
