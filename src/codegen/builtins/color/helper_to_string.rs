//! `__color_toString` — the `toString(color::Color)` override target.
//!
//! Registered via `add_helper` (private-only) and named by `add_override`; it is
//! reached as an override of the general `toString` builtin, never as a `color`
//! member. Body byte-significant (2-space indent → `.ncode` columns); do not
//! reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

/// Renders `#rrggbbaa` — the **lossless** form, alpha always present.
///
/// `toString` is what a debugging `io::print` reaches for, so it must not be the
/// spelling that silently drops a channel: a colour printed while chasing a
/// transparency bug would show identically at every alpha. `color::toHex` is the
/// six-digit alpha-dropping form for callers that want it, and choosing between
/// them is the caller's call to make explicitly.
///
/// Delegates to `__color_hexByte` rather than restating the digit rendering, so
/// `toString(c)` and `color::toHexAlpha(c)` cannot drift apart — they are the same
/// bytes by construction, which is what the fixture asserts.
#[rustfmt::skip]
const BODY: &str =
r##"FUNC __color_toString(base AS Color) AS String
  RETURN "#" & __color_hexByte(base.red) & __color_hexByte(base.green) & __color_hexByte(base.blue) & __color_hexByte(base.alpha)
END FUNC"##;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("color_toString", BODY));
}
