//! `__audio_mmlEncode` — shared private helper for the `audio` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Encode signed s16 samples as little-endian s16le bytes.
'
' The two-byte emit is INLINE, not a call to a shared `appendS16LE(pcm, sample)`
' helper. Handing the growing `pcm` to a helper passes it by value, so every
' sample copied the whole accumulator and the encode ran O(n^2): one second of
' 48 kHz audio took ~10 s instead of ~12 ms. The accumulator must stay a
' same-function local for `collections::append` to mutate it in place.
FUNC __audio_mmlEncode(samples AS List OF Integer) AS List OF Byte
  MUT pcm AS List OF Byte = []
  FOR EACH s IN samples
    MUT v AS Integer = s
    IF v < 0 THEN
      v = v + 65536
    END IF
    pcm = collections::append(pcm, toByte(bits::band(v, 255)))
    pcm = collections::append(pcm, toByte(bits::band(bits::sr(v, 8), 255)))
  NEXT
  RETURN pcm
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("audio_mmlEncode", BODY));
}
