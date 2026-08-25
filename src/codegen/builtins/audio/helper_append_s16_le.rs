//! `__audio_appendS16LE` — shared private helper for the `audio` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-339 C7: shared little-endian s16 emit, previously inlined in __audio_render
' and duplicated in __audio_mmlEncode. `sample` must already be in signed 16-bit range;
' the +65536 wrap maps a negative sample to its unsigned two's-complement bytes.
FUNC __audio_appendS16LE(pcm AS List OF Byte, sample AS Integer) AS List OF Byte
  MUT v AS Integer = sample
  IF v < 0 THEN
    v = v + 65536
  END IF
  MUT out AS List OF Byte = pcm
  out = collections::append(out, toByte(bits::band(v, 255)))
  out = collections::append(out, toByte(bits::band(bits::sr(v, 8), 255)))
  RETURN out
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("audio_appendS16LE", BODY));
}
