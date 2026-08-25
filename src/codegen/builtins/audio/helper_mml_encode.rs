//! `__audio_mmlEncode` — shared private helper for the `audio` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Encode signed s16 samples as little-endian s16le bytes.
FUNC __audio_mmlEncode(samples AS List OF Integer) AS List OF Byte
  MUT pcm AS List OF Byte = []
  FOR EACH s IN samples
    pcm = __audio_appendS16LE(pcm, s)
  NEXT
  RETURN pcm
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("audio_mmlEncode", BODY));
}
