//! `__color_hexValue` — one hex digit's value, or `-1`.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

/// Returns `0`..`15` for a hex digit in either case, and `-1` for anything else —
/// which is what makes `color::fromHex` case-insensitive without a `strings::upper`
/// pass over the input first, and what carries every rejection (`'#'`, `'g'`, a
/// space, a non-ASCII byte) into a single test at the call site.
///
/// The same three-range shape as `encoding`'s `__encoding_hexValue`. `color`
/// deliberately does not call `encoding`'s: importing `encoding` costs an importer
/// 429,312 bytes of companion (measured, plan-122-A §2) for one nine-line FUNC, and
/// `color` is imported by canvas, term and astrings consumers after plan-122-D–F.
#[rustfmt::skip]
const BODY: &str =
r#"FUNC __color_hexValue(c AS Integer) AS Integer
  IF c >= 48 AND c <= 57 THEN
    RETURN c - 48
  END IF
  IF c >= 97 AND c <= 102 THEN
    RETURN c - 87
  END IF
  IF c >= 65 AND c <= 70 THEN
    RETURN c - 55
  END IF
  RETURN -1
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("color_hexValue", BODY));
}
