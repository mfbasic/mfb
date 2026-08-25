//! `__astrings_splitSpan` — shared private helper for the `astrings` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"REM Append the surviving flanks of `s` after removing the inclusive range
REM [rs, re] to `acc`, returning the updated list. A span not overlapping the
REM range survives whole; a straddler keeps its left flank [s.start, rs-1] and/or
REM right flank [re+1, s.endIndex]; the overlap is dropped. Flanks keep `s`'s seq.
FUNC __astrings_splitSpan(acc AS List OF AttrSpan, s AS AttrSpan, rs AS Integer, re AS Integer) AS List OF AttrSpan
  MUT out AS List OF AttrSpan = acc
  IF s.last < rs OR s.start > re THEN
    out = collections::append(out, s)
    RETURN out
  END IF
  IF s.start <= rs - 1 THEN
    out = collections::append(out, AttrSpan[s.start, rs - 1, s.seq, s.class, s.member, s.text, s.number])
  END IF
  IF re + 1 <= s.last THEN
    out = collections::append(out, AttrSpan[re + 1, s.last, s.seq, s.class, s.member, s.text, s.number])
  END IF
  RETURN out
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("astrings_splitSpan", BODY));
}
