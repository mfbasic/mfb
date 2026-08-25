//! `__http_indexOf` — shared private helper for the `http` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-339 B2: __http_indexOf / __http_slice / __http_defaultPort duplicate the
' identical net_package.mfb helpers. The dup is language-mandated (built-in package
' sources are file-local; http cannot reach net's private __net_* helpers even
' though it IMPORTs net) — see the full note at net_package.mfb __net_indexOf.
'
' Grapheme index of `needle` in `s` at or after `start`, or -1 when absent.
' `strings::find` is inline-expanded and cannot be wrapped in an inline TRAP, so
' presence is checked first with `contains`.
FUNC __http_indexOf(s AS String, needle AS String, start AS Integer) AS Integer
  LET tail AS String = __http_slice(s, start, len(s))
  IF strings::contains(tail, needle) = FALSE THEN
    RETURN -1
  END IF
  RETURN strings::find(s, needle, start)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_indexOf", BODY));
}
