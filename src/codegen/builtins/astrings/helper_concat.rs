//! `__astrings_concat` — shared private helper for the `astrings` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"REM `AttributedString & AttributedString` (plan-89-D Open Decision 2): concatenate
REM the visible text and carry both overlays — the right operand's spans shift by
REM the left operand's scalar length. The two overlays occupy disjoint scalar
REM ranges, so the reused `seq` values never tie across operands.
FUNC __astrings_concat(a AS AttributedString, b AS AttributedString) AS AttributedString
  LET newText AS String = toString(a) & toString(b)
  LET aLen AS Integer = astrings::scalarLen(a)
  MUT out AS List OF AttrSpan = astrings::readSpans(a)
  FOR EACH s IN astrings::readSpans(b)
    out = collections::append(out, AttrSpan[s.start + aLen, s.last + aLen, s.seq, s.class, s.member, s.text, s.number])
  NEXT
  RETURN __astrings_assemble(newText, out)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("astrings_concat", BODY));
}
