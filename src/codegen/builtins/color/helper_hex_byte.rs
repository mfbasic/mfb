//! `__color_hexByte` — one channel rendered as two lowercase hex digits.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

/// Always two digits, zero-padded — `toHex`/`toHexAlpha` promise a fixed width, so
/// a channel below 16 must not collapse to one digit and silently shift every
/// later channel left.
///
/// Lowercase, matching `encoding::hexEncode`'s convention, so
/// `fromHex(toHex(c))` round-trips and two programs' `toHex` output compares
/// equal.
/// The digit comes out of a constant table by index rather than by arithmetic on a
/// scalar value: `strings::fromScalars` takes a `List OF Scalar`, so building a
/// digit from `48 + value` would need a per-digit `Integer`→`Scalar` conversion to
/// produce one character. `strings::mid` over a 16-character table is the same
/// answer with no conversion, and the table doubles as the statement that the
/// digits are lowercase.
///
/// `value` is always `0`..`15` here — both call sites derive it from a `Byte` by
/// `/ 16` and the remainder — so the `mid` index is always in range. `strings::mid`
/// raises rather than clamping on an out-of-range index, which is the behaviour we
/// want if that ever stops being true.
#[rustfmt::skip]
const BODY: &str =
r#"FUNC __color_hexByte(value AS Byte) AS String
  LET v AS Integer = toInt(value)
  RETURN __color_hexDigit(v / 16) & __color_hexDigit(v - (v / 16) * 16)
END FUNC

FUNC __color_hexDigit(value AS Integer) AS String
  RETURN strings::mid("0123456789abcdef", value, 1)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("color_hexByte", BODY));
}
