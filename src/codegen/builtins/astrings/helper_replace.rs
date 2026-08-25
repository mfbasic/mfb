//! `__astrings_replace` — shared private helper for the `astrings` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __astrings_replace(a AS AttributedString, old AS String, new AS String) AS AttributedString
  LET text AS String = toString(a)
  LET newText AS String = strings::replace(text, old, new)
  LET matches AS List OF Integer = __astrings_findMatches(text, old)
  LET nlen AS Integer = __astrings_scalarCountStr(old)
  LET rlen AS Integer = __astrings_scalarCountStr(new)
  LET tlen AS Integer = __astrings_scalarCountStr(text)
  LET spans AS List OF AttrSpan = astrings::readSpans(a)
  MUT out AS List OF AttrSpan = []
  MUT prevEnd AS Integer = 0
  MUT newStart AS Integer = 0
  FOR EACH m IN matches
    out = __astrings_remapSegment(out, spans, prevEnd, m - 1, newStart)
    LET segLen AS Integer = m - prevEnd
    newStart = newStart + segLen + rlen
    prevEnd = m + nlen
  NEXT
  out = __astrings_remapSegment(out, spans, prevEnd, tlen - 1, newStart)
  RETURN __astrings_assemble(newText, out)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("astrings_replace", BODY));
}
