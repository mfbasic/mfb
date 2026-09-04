//! `__http_hasFieldControlBytes` — shared private helper for the `http` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-506 / OS-55: TRUE if `s` carries a byte that may not appear in a header
' field value or a reason phrase (RFC 9110 §5.5): any control byte below 0x20
' other than HTAB (0x09), or DEL (0x7F). CR/LF split the head into a second,
' attacker-shaped response; NUL truncates it in a C-string consumer. HTAB is
' legal field whitespace and so is allowed here, unlike `__http_hasControlBytes`
' (the request-side sweep, where the method and names are tokens).
FUNC __http_hasFieldControlBytes(s AS String) AS Boolean
  FOR EACH b IN strings::toBytes(s)
    LET v AS Integer = toInt(b)
    IF (v < 32 AND v <> 9) OR v = 127 THEN
      RETURN TRUE
    END IF
  NEXT
  RETURN FALSE
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_hasFieldControlBytes", BODY));
}
