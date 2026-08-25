//! `__astrings_remapSegment` — shared private helper for the `astrings` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"REM Emit, into `acc`, each span clipped to the kept original segment [o0, o1] and
REM re-based to the new-coordinate origin `n0`. A span covering a replaced match
REM is emitted once per surviving kept segment it intersects (drop-inside,
REM clip-straddle, split-around all fall out of the intersection).
FUNC __astrings_remapSegment(acc AS List OF AttrSpan, spans AS List OF AttrSpan, o0 AS Integer, o1 AS Integer, n0 AS Integer) AS List OF AttrSpan
  MUT out AS List OF AttrSpan = acc
  IF o1 < o0 THEN
    RETURN out
  END IF
  FOR EACH s IN spans
    MUT lo AS Integer = s.start
    MUT hi AS Integer = s.last
    IF lo < o0 THEN
      lo = o0
    END IF
    IF hi > o1 THEN
      hi = o1
    END IF
    IF lo <= hi THEN
      out = collections::append(out, AttrSpan[n0 + (lo - o0), n0 + (hi - o0), s.seq, s.class, s.member, s.text, s.number])
    END IF
  NEXT
  RETURN out
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("astrings_remapSegment", BODY));
}
