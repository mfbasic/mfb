//! `__astrings_shiftSpans` — shared private helper for the `astrings` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"REM Shift every span by +delta (padLeft inserts plain scalars at the front).
FUNC __astrings_shiftSpans(spans AS List OF AttrSpan, delta AS Integer) AS List OF AttrSpan
  MUT out AS List OF AttrSpan = []
  FOR EACH s IN spans
    out = collections::append(out, AttrSpan[s.start + delta, s.last + delta, s.seq, s.class, s.member, s.text, s.number])
  NEXT
  RETURN out
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("astrings_shiftSpans", BODY));
}
