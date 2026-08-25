//! `__astrings_windowSpans` — shared private helper for the `astrings` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"REM Clip each span to the kept scalar window [w0, w1] (inclusive) and shift it to
REM the new origin by -w0; drop a span that falls entirely outside the window.
FUNC __astrings_windowSpans(spans AS List OF AttrSpan, w0 AS Integer, w1 AS Integer) AS List OF AttrSpan
  MUT out AS List OF AttrSpan = []
  FOR EACH s IN spans
    MUT lo AS Integer = s.start
    MUT hi AS Integer = s.last
    IF lo < w0 THEN
      lo = w0
    END IF
    IF hi > w1 THEN
      hi = w1
    END IF
    IF lo <= hi THEN
      out = collections::append(out, AttrSpan[lo - w0, hi - w0, s.seq, s.class, s.member, s.text, s.number])
    END IF
  NEXT
  RETURN out
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("astrings_windowSpans", BODY));
}
