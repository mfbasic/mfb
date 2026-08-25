//! `__astrings_packColor` — shared private helper for the `astrings` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"REM Pack an (r, g, b) triple into a single `0xRRGGBB` Integer — r in the high
REM byte, b in the low byte — the payload the `Foreground`/`Background` numeric
REM attributes carry. Each channel is a Byte, so no channel overflows its 8 bits
REM and the packing is lossless (unlike a decimal r*100+... scheme). The `term`
REM bridge unpacks it with the inverse shifts/masks.
FUNC __astrings_packColor(r AS Byte, g AS Byte, b AS Byte) AS Integer
  RETURN bits::bor(bits::bor(bits::sl(toInt(r), 16), bits::sl(toInt(g), 8)), toInt(b))
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("astrings_packColor", BODY));
}
