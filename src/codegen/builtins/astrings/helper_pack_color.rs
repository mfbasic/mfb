//! `__astrings_packColor` — shared private helper for the `astrings` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"REM Pack an (r, g, b, a) quadruple into a single `0xAARRGGBB` Integer — alpha in
REM the high byte, b in the low byte — the payload the `Foreground`/`Background`
REM numeric attributes carry. Each channel is a Byte, so no channel overflows its
REM 8 bits and the packing is lossless (unlike a decimal r*100+... scheme). The
REM order is `color::toPacked`'s, so the two agree by construction and the `term`
REM bridge can unpack with `color::fromPacked`.
FUNC __astrings_packColor(r AS Byte, g AS Byte, b AS Byte, a AS Byte) AS Integer
  LET high AS Integer = bits::bor(bits::sl(toInt(a), 24), bits::sl(toInt(r), 16))
  RETURN bits::bor(high, bits::bor(bits::sl(toInt(g), 8), toInt(b)))
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("astrings_packColor", BODY));
}
