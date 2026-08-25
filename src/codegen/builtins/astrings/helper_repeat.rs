//! `__astrings_repeat` — shared private helper for the `astrings` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __astrings_repeat(a AS AttributedString, times AS Integer) AS AttributedString
  LET text AS String = toString(a)
  LET newText AS String = strings::repeat(text, times)
  LET n AS Integer = astrings::scalarLen(a)
  LET spans AS List OF AttrSpan = astrings::readSpans(a)
  MUT out AS List OF AttrSpan = []
  MUT k AS Integer = 0
  WHILE k < times
    LET offset AS Integer = k * n
    FOR EACH s IN spans
      out = collections::append(out, AttrSpan[s.start + offset, s.last + offset, s.seq, s.class, s.member, s.text, s.number])
    NEXT
    k = k + 1
  END WHILE
  RETURN __astrings_assemble(newText, out)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("astrings_repeat", BODY));
}
