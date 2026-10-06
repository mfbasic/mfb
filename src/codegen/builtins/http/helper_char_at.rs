//! `__http_charAt` — shared private helper for the `http` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-981 (declared-errors review): a scalar-indexed character read that
' CANNOT raise `ErrIndexOutOfRange`, unlike `strings::mid`. `strings::right`
' then `strings::left` both CLAMP instead of raising past the end (`mfb man
' strings right`/`left`), and their only declared errors (`ErrInvalidArgument`
' for a negative count, `ErrOutOfMemory`) are ones `http::startRead` already
' declares through unrelated call sites — so a caller that reaches this
' helper never has to grow its own declared-errors set. `__http_isHexDigitChar`
' and `__http_percentEncodeTarget`/`__http_percentByteEncode` use this instead
' of `strings::mid` for exactly that reason: a lookahead read past the target
' string's end (always guarded by the caller's own bounds check first, so
' `pos` here is never actually negative or past `len(s)`) must not turn into
' a NEW undeclared raise for every function that transitively reaches it.
' Returns "" when `pos` is at or past the end of `s`.
FUNC __http_charAt(s AS String, pos AS Integer) AS String
  RETURN strings::left(strings::right(s, len(s) - pos), 1)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_charAt", BODY));
}
